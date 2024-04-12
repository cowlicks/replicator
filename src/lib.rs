/// trait for replication
/// unfortunately this thing has to take `self` because it usually consumes the thing
use futures_lite::{AsyncRead, AsyncWrite, Future, StreamExt};
use std::{fmt::Debug, marker::Unpin};
use thiserror::Error;
use tracing::{error, info};

use random_access_storage::RandomAccess;

use hypercore::{Hypercore, HypercoreError, RequestBlock, RequestUpgrade};
use hypercore_protocol::{
    discovery_key,
    schema::{Data, Range, Request, Synchronize},
    Channel, Event, Message, ProtocolBuilder,
};

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ReplicatorError {
    #[error("There was an error in the opening the protocol in handshake")]
    // TODO add key and show it
    IoError(#[from] std::io::Error),
    #[error("TODO")]
    // TODO add key and show it
    HypercoreError(#[from] HypercoreError),
}

pub trait Replicator {
    fn replicate<S>(
        self,
        stream: S,
        is_initiator: bool,
    ) -> impl Future<Output = Result<(), ReplicatorError>> + Send
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static;
}

impl<T: RandomAccess + Debug + Send + 'static> Replicator for Hypercore<T> {
    fn replicate<S>(
        self,
        stream: S,
        is_initiator: bool,
    ) -> impl Future<Output = Result<(), ReplicatorError>> + Send
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let key = self.key_pair().public.to_bytes().clone();
        let this_dkey = discovery_key(self.key_pair().public.as_bytes());
        async_std::task::spawn(async move {
            let mut protocol = ProtocolBuilder::new(is_initiator).connect(stream);
            // TODO while let Some(event) = protocol.next().await {
            if let Some(event) = protocol.next().await {
                let event = event.unwrap();
                info!("protocol event {:?}", event);
                match event {
                    Event::Handshake(_m) => {
                        info!("got handshake! {_m:?}");
                        if is_initiator {
                            protocol.open(key).await?;
                        }
                    }
                    Event::DiscoveryKey(dkey) => {
                        info!("got discovery key msg {dkey:?}");
                        if this_dkey == dkey {
                            protocol.open(key).await?;
                        }
                    }
                    Event::Channel(channel) => {
                        if this_dkey == *channel.discovery_key() {
                            onpeer(self, channel);
                        }
                    }
                    Event::Close(_dkey) => {}
                    _ => todo!(),
                }
            }
            Ok(())
        })
    }
}

pub async fn onpeer<T: RandomAccess + Debug + Send>(mut core: Hypercore<T>, mut channel: Channel) {
    let mut peer_state = PeerState::default();
    let info = core.info();

    if info.fork != peer_state.remote_fork {
        peer_state.can_upgrade = false;
    }
    let remote_length = if info.fork == peer_state.remote_fork {
        peer_state.remote_length
    } else {
        0
    };

    let sync_msg = Synchronize {
        fork: info.fork,
        length: info.length,
        remote_length,
        can_upgrade: peer_state.can_upgrade,
        uploading: true,
        downloading: true,
    };

    if info.contiguous_length > 0 {
        let range_msg = Range {
            drop: false,
            start: 0,
            length: info.contiguous_length,
        };
        channel
            .send_batch(&[Message::Synchronize(sync_msg), Message::Range(range_msg)])
            .await
            .unwrap();
    } else {
        channel.send(Message::Synchronize(sync_msg)).await.unwrap();
    }
    while let Some(message) = channel.next().await {
        let result = onmessage(&mut core, &mut peer_state, &mut channel, message).await;
        if let Err(e) = result {
            error!("protocol error: {}", e);
            break;
        }
    }
}

/// A PeerState stores the head seq of the remote.
/// This would have a bitfield to support sparse sync in the actual impl.
#[derive(Debug)]
struct PeerState {
    can_upgrade: bool,
    remote_fork: u64,
    remote_length: u64,
    remote_can_upgrade: bool,
    remote_uploading: bool,
    remote_downloading: bool,
    remote_synced: bool,
    length_acked: u64,
}
impl Default for PeerState {
    fn default() -> Self {
        PeerState {
            can_upgrade: true,
            remote_fork: 0,
            remote_length: 0,
            remote_can_upgrade: false,
            remote_uploading: true,
            remote_downloading: true,
            remote_synced: false,
            length_acked: 0,
        }
    }
}

pub async fn onmessage<T>(
    hypercore: &mut Hypercore<T>,
    peer_state: &mut PeerState,
    channel: &mut Channel,
    message: Message,
) -> Result<(), ReplicatorError>
where
    T: RandomAccess + Debug + Send,
{
    match message {
        Message::Synchronize(message) => {
            println!("Got Synchronize message {message:?}");
            let length_changed = message.length != peer_state.remote_length;
            let first_sync = !peer_state.remote_synced;
            let info = hypercore.info();
            let same_fork = message.fork == info.fork;

            peer_state.remote_fork = message.fork;
            peer_state.remote_length = message.length;
            peer_state.remote_can_upgrade = message.can_upgrade;
            peer_state.remote_uploading = message.uploading;
            peer_state.remote_downloading = message.downloading;
            peer_state.remote_synced = true;

            peer_state.length_acked = if same_fork { message.remote_length } else { 0 };

            let mut messages = vec![];

            if first_sync {
                // Need to send another sync back that acknowledges the received sync
                let msg = Synchronize {
                    fork: info.fork,
                    length: info.length,
                    remote_length: peer_state.remote_length,
                    can_upgrade: peer_state.can_upgrade,
                    uploading: true,
                    downloading: true,
                };
                messages.push(Message::Synchronize(msg));
            }

            if peer_state.remote_length > info.length
                && peer_state.length_acked == info.length
                && length_changed
            {
                let msg = Request {
                    id: 1, // There should be proper handling for in-flight request ids
                    fork: info.fork,
                    hash: None,
                    block: None,
                    seek: None,
                    upgrade: Some(RequestUpgrade {
                        start: info.length,
                        length: peer_state.remote_length - info.length,
                    }),
                };
                messages.push(Message::Request(msg));
            }
            channel.send_batch(&messages).await?;
        }
        Message::Request(message) => {
            println!("Got Request message {message:?}");
            let (info, proof) = {
                let proof = hypercore
                    .create_proof(message.block, message.hash, message.seek, message.upgrade)
                    .await
                    .unwrap();
                //TODO .await?;
                (hypercore.info(), proof)
            };
            if let Some(proof) = proof {
                let msg = Data {
                    request: message.id,
                    fork: info.fork,
                    hash: proof.hash,
                    block: proof.block,
                    seek: proof.seek,
                    upgrade: proof.upgrade,
                };
                channel.send(Message::Data(msg)).await?;
            }
        }
        Message::Data(message) => {
            println!("Got Data message {message:?}");
            let (_old_info, _applied, new_info, request_block) = {
                let old_info = hypercore.info();
                let proof = message.clone().into_proof();
                let applied = hypercore.verify_and_apply_proof(&proof).await?;
                let new_info = hypercore.info();
                let request_block: Option<RequestBlock> = if let Some(upgrade) = &message.upgrade {
                    // When getting the initial upgrade, send a request for the first missing block
                    if old_info.length < upgrade.length {
                        let request_index = old_info.length;
                        let nodes = hypercore.missing_nodes(request_index).await?;
                        Some(RequestBlock {
                            index: request_index,
                            nodes,
                        })
                    } else {
                        None
                    }
                } else if let Some(block) = &message.block {
                    // When receiving a block, ask for the next, if there are still some missing
                    if block.index < peer_state.remote_length - 1 {
                        let request_index = block.index + 1;
                        let nodes = hypercore.missing_nodes(request_index).await?;
                        Some(RequestBlock {
                            index: request_index,
                            nodes,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                // If all have been replicated, print the result
                if new_info.contiguous_length == new_info.length {
                    println!();
                    println!("### Results");
                    println!();
                    println!("Replication succeeded if this prints '0: hi', '1: ola', '2: hello' and '3: mundo':");
                    println!();
                    for i in 0..new_info.contiguous_length {
                        println!(
                            "{}: {}",
                            i,
                            String::from_utf8(hypercore.get(i).await?.unwrap()).unwrap()
                        );
                    }
                    println!("Press Ctrl-C to exit");
                }
                (old_info, applied, new_info, request_block)
            };

            let mut messages: Vec<Message> = vec![];
            if let Some(upgrade) = &message.upgrade {
                let new_length = upgrade.length;
                let remote_length = if new_info.fork == peer_state.remote_fork {
                    peer_state.remote_length
                } else {
                    0
                };
                messages.push(Message::Synchronize(Synchronize {
                    fork: new_info.fork,
                    length: new_length,
                    remote_length,
                    can_upgrade: false,
                    uploading: true,
                    downloading: true,
                }));
            }
            if let Some(request_block) = request_block {
                messages.push(Message::Request(Request {
                    id: request_block.index + 1,
                    fork: new_info.fork,
                    hash: None,
                    block: Some(request_block),
                    seek: None,
                    upgrade: None,
                }));
            }
            channel.send_batch(&messages).await.unwrap();
        }
        _ => {}
    };
    Ok(())
}

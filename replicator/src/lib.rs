//! trait for replication
#[cfg(test)]
mod test;

use std::{fmt::Debug, marker::Unpin};

use async_channel::Receiver;
use async_std::{
    sync::{Arc, Mutex, RwLock},
    task::{spawn, JoinHandle},
};
use futures_lite::{AsyncRead, AsyncWrite, Future, StreamExt};

use thiserror::Error;
use tracing::{error, info, trace, warn};

use hypercore::{Hypercore, HypercoreError, RequestBlock, RequestUpgrade};
use hypercore_protocol::{
    discovery_key,
    schema::{Data, Range, Request, Synchronize},
    Channel, Event, Key, Message, Protocol, ProtocolBuilder,
};

trait StreamTraits: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static {}
impl<S: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static> StreamTraits for S {}

type ShareRw<T> = Arc<RwLock<T>>;
type SharedCore = Arc<Mutex<Hypercore>>;

#[macro_export]
macro_rules! lk {
    ($core:tt) => {
        $core.lock().await
    };
}

#[macro_export]
macro_rules! r {
    ($core:tt) => {
        $core.read().await
    };
}

macro_rules! reader_or_writer {
    ($core:ident) => {
        if is_writer($core.clone()).await {
            "Writer"
        } else {
            "Reader"
        }
    };
}

async fn is_writer(c: SharedCore) -> bool {
    lk!(c).key_pair().secret.is_some()
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ReplicatorError {
    #[error("There was an error in the opening the protocol in handshake")]
    // TODO add key and show it
    IoError(#[from] std::io::Error),
    #[error("TODO")]
    // TODO add error and show it
    HypercoreError(#[from] HypercoreError),
}

pub trait Replicate {
    fn replicate(&self) -> impl Future<Output = Result<HcReplicator, ReplicatorError>> + Send;
}

impl Replicate for SharedCore {
    fn replicate(&self) -> impl Future<Output = Result<HcReplicator, ReplicatorError>> + Send {
        let core = self.clone();
        async move { Ok(HcReplicator::new(core)) }
    }
}

#[async_trait::async_trait]
trait ProtoMethods: Debug + Send + Sync {
    async fn open(&mut self, key: Key) -> std::io::Result<()>;
    fn receiver_for_all_channel_messages(&self) -> Receiver<Message>;
    async fn _next(&mut self) -> Option<std::io::Result<Event>>;
}

#[async_trait::async_trait]
impl<S: StreamTraits> ProtoMethods for Protocol<S> {
    async fn open(&mut self, key: Key) -> std::io::Result<()> {
        Protocol::open(self, key).await
    }
    fn receiver_for_all_channel_messages(&self) -> Receiver<Message> {
        Protocol::receiver_for_all_channel_messages(self)
    }
    async fn _next(&mut self) -> Option<std::io::Result<Event>> {
        futures_lite::StreamExt::next(&mut self).await
    }
}

pub struct Peer {
    core: SharedCore,
    protocol: ShareRw<Box<dyn ProtoMethods>>,
    message_buff: ShareRw<Vec<Message>>,
}

impl Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peer")
            //.field("core", &self.core)
            //.field("protocol", &self.protocol)
            .field("message_buff", &self.message_buff)
            .finish()
    }
}

impl Peer {
    fn new(core: SharedCore, protocol: ShareRw<Box<dyn ProtoMethods>>) -> Self {
        Self {
            core,
            protocol,
            message_buff: Arc::new(RwLock::new(vec![])),
        }
    }

    async fn listen_to_channel_messages(&self) {
        let receiver = self
            .protocol
            .read()
            .await
            .receiver_for_all_channel_messages();
        let message_buff = self.message_buff.clone();
        spawn(async move {
            while let Ok(msg) = receiver.recv().await {
                message_buff.write().await.push(msg);
            }
        });
    }

    async fn start_message_loop(&self, is_initiator: bool) -> Result<(), ReplicatorError> {
        let key = self.core.lock().await.key_pair().public.to_bytes();
        let this_dkey = discovery_key(&key);
        let core = self.core.clone();
        let name = reader_or_writer!(core);
        let protocol = self.protocol.clone();
        while let Some(Ok(event)) = {
            // this block is just here to release the `.write()` lock
            #[allow(clippy::let_and_return)]
            let p = protocol.write().await._next().await;
            p
        } {
            info!("\n\t{name} Proto RX:\n\t{:#?}", event);
            match event {
                Event::Handshake(_m) => {
                    if is_initiator {
                        protocol.write().await.open(key).await?;
                    }
                }
                Event::DiscoveryKey(dkey) => {
                    if this_dkey == dkey {
                        protocol.write().await.open(key).await?;
                    } else {
                        warn!("Got discovery key for different core: {dkey:?}");
                    }
                }
                Event::Channel(channel) => {
                    if this_dkey == *channel.discovery_key() {
                        onpeer(core.clone(), channel).await?;
                    } else {
                        error!("Wrong discovery key?");
                    }
                }
                Event::Close(_dkey) => {}
                _ => todo!(),
            }
        }
        Ok(())
    }
}

pub struct HcReplicator {
    core: SharedCore,
    peers: Vec<ShareRw<Peer>>,
}

impl HcReplicator {
    pub fn new(core: SharedCore) -> Self {
        Self {
            core,
            peers: vec![],
        }
    }

    #[allow(private_bounds)]
    pub async fn add_peer<S: StreamTraits>(
        &mut self,
        stream: S,
        is_initiator: bool,
    ) -> Result<ShareRw<Peer>, ReplicatorError> {
        let core = self.core.clone();
        let protocol = ProtocolBuilder::new(is_initiator).connect(stream);

        let peer = Arc::new(RwLock::new(Peer::new(
            core,
            Arc::new(RwLock::new(Box::new(protocol))),
        )));
        self.peers.push(peer.clone());
        Ok(peer)
    }

    #[allow(private_bounds)]
    pub async fn add_stream<S: StreamTraits>(
        &mut self,
        stream: S,
        is_initiator: bool,
    ) -> Result<(), ReplicatorError> {
        let peer = self.add_peer(stream, is_initiator).await?;
        peer.read().await.listen_to_channel_messages().await;
        spawn(async move {
            peer.read().await.start_message_loop(is_initiator).await?;
            Ok::<(), ReplicatorError>(())
        });
        Ok(())
    }
}
async fn initiate_sync(
    core: SharedCore,
    peer_state: ShareRw<PeerState>,
    channel: &mut Channel,
) -> Result<(), ReplicatorError> {
    let name = reader_or_writer!(core);
    let info = lk!(core).info();
    if info.fork != peer_state.read().await.remote_fork {
        peer_state.write().await.can_upgrade = false;
    }
    let remote_length = if info.fork == peer_state.read().await.remote_fork {
        peer_state.read().await.remote_length
    } else {
        0
    };

    let sync_msg = Synchronize {
        fork: info.fork,
        length: info.length,
        remote_length,
        can_upgrade: peer_state.read().await.can_upgrade,
        uploading: true,
        downloading: true,
    };

    if info.contiguous_length > 0 {
        let range_msg = Range {
            drop: false,
            start: 0,
            length: info.contiguous_length,
        };
        info!("\n\t{name} Channel TX:[\n\t{sync_msg:#?},\n\t{range_msg:#?}\n])");
        channel
            .send_batch(&[Message::Synchronize(sync_msg), Message::Range(range_msg)])
            .await?;
    } else {
        info!("\n\t{name} Channel TX:\n\t{sync_msg:#?})");
        channel.send(Message::Synchronize(sync_msg)).await?;
    }
    Ok(())
}

async fn core_event_loop(
    core: SharedCore,
    peer_state: ShareRw<PeerState>,
    mut channel: Channel,
) -> Result<(), ReplicatorError> {
    let mut on_upgrade = core.lock().await.on_upgrade();
    while let Ok(_event) = on_upgrade.recv().await {
        trace!("got core upgrade event. Notifying peers");
        initiate_sync(core.clone(), peer_state.clone(), &mut channel).await?
    }
    Ok(())
}

async fn on_get_event_loop(core: SharedCore, mut channel: Channel) -> Result<(), ReplicatorError> {
    let mut on_get = core.lock().await.on_get_subscribe();
    while let Ok((index, _tx)) = on_get.recv().await {
        trace!("got core upgrade event. Notifying peers");
        let block = RequestBlock {
            index,
            nodes: core.lock().await.missing_nodes(index).await?,
        };
        let msg = Message::Request(Request {
            fork: core.lock().await.info().fork,
            id: block.index + 1,
            block: Some(block),
            hash: None,
            seek: None,
            upgrade: None,
        });

        channel.send_batch(&[msg]).await?;
    }
    Ok(())
}

pub async fn onpeer(core: SharedCore, mut channel: Channel) -> Result<(), ReplicatorError> {
    let peer_state = Arc::new(RwLock::new(PeerState::default()));

    initiate_sync(core.clone(), peer_state.clone(), &mut channel).await?;

    let _event_loop = spawn(core_event_loop(
        core.clone(),
        peer_state.clone(),
        channel.clone(),
    ));

    let _on_get_loop = spawn(on_get_event_loop(core.clone(), channel.clone()));

    let _channel_rx_loop = spawn(async move {
        while let Some(message) = channel.next().await {
            onmessage(core.clone(), peer_state.clone(), channel.clone(), message);
        }
    });
    Ok(())
}

fn onmessage(
    core: SharedCore,
    peer_state: ShareRw<PeerState>,
    channel: Channel,
    message: Message,
) -> JoinHandle<Result<(), ReplicatorError>> {
    spawn(async move {
        if let Err(e) = onmessage_inner(core, peer_state, channel, message).await {
            error!("Error handling message: {e}");
            return Err(e);
        }
        Ok(())
    })
}

async fn onmessage_inner(
    core: SharedCore,
    peer_state: ShareRw<PeerState>,
    mut channel: Channel,
    message: Message,
) -> Result<(), ReplicatorError> {
    let name = reader_or_writer!(core);
    trace!("{name} onmessage {}", message.kind());
    match message {
        Message::Synchronize(message) => {
            let peer_length_changed = message.length != r!(peer_state).remote_length;
            let first_sync = !r!(peer_state).remote_synced;
            let info = lk!(core).info();
            let same_fork = message.fork == info.fork;

            {
                let mut ps = peer_state.write().await;
                ps.remote_fork = message.fork;
                ps.remote_length = message.length;
                ps.remote_can_upgrade = message.can_upgrade;
                ps.remote_uploading = message.uploading;
                ps.remote_downloading = message.downloading;
                ps.remote_synced = true;

                ps.length_acked = if same_fork { message.remote_length } else { 0 };
            }
            let mut messages = vec![];

            if first_sync {
                // Need to send another sync back that acknowledges the received sync
                let msg = Synchronize {
                    fork: info.fork,
                    length: info.length,
                    remote_length: r!(peer_state).remote_length,
                    can_upgrade: r!(peer_state).can_upgrade,
                    uploading: true,
                    downloading: true,
                };
                messages.push(Message::Synchronize(msg));
            }

            // if peer is longer than us
            if r!(peer_state).remote_length > info.length
                // and peer knows our correct length
                && r!(peer_state).length_acked == info.length
                // and peer's length has changed
                && peer_length_changed
            {
                let msg = Request {
                    id: 1, // There should be proper handling for in-flight request ids
                    fork: info.fork,
                    hash: None,
                    block: None,
                    seek: None,
                    upgrade: Some(RequestUpgrade {
                        start: info.length,
                        length: r!(peer_state).remote_length - info.length,
                    }),
                };

                messages.push(Message::Request(msg));
            }
            info!("\n\t{name} Channel TX:\n\t{messages:#?}");
            if !messages.is_empty() {
                channel.send_batch(&messages).await?;
            }
        }

        Message::Request(message) => {
            trace!("Got Request message {message:?}");
            let (info, proof) = {
                let proof = lk!(core)
                    .create_proof(message.block, message.hash, message.seek, message.upgrade)
                    .await
                    .unwrap();
                //TODO .await?;
                (lk!(core).info(), proof)
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
            trace!("Got Data message Data {{...}}");
            let (_old_info, _applied, new_info, request_block) = {
                let old_info = lk!(core).info();

                let proof = message.clone().into_proof();
                let applied = lk!(core).verify_and_apply_proof(&proof).await?;
                let new_info = lk!(core).info();

                let request_block: Option<RequestBlock> = if let Some(upgrade) = &message.upgrade {
                    // When getting the initial upgrade, send a request for the first missing block
                    if old_info.length < upgrade.length {
                        let request_index = old_info.length;
                        let nodes = lk!(core).missing_nodes(request_index).await?;
                        Some(RequestBlock {
                            index: request_index,
                            nodes,
                        })
                    } else {
                        None
                    }
                } else if let Some(block) = &message.block {
                    // When receiving a block, ask for the next, if there are still some missing
                    if block.index < r!(peer_state).remote_length - 1 {
                        let request_index = block.index + 1;
                        let nodes = lk!(core).missing_nodes(request_index).await?;
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
                    for i in 0..new_info.contiguous_length {
                        trace!(
                            "{}: {}",
                            i,
                            String::from_utf8(lk!(core).get(i).await?.unwrap()).unwrap()
                        );
                    }
                }
                (old_info, applied, new_info, request_block)
            };

            let mut messages: Vec<Message> = vec![];

            // If we got an upgrade send a Sync
            if message.upgrade.is_some() {
                let remote_length = if new_info.fork == r!(peer_state).remote_fork {
                    r!(peer_state).remote_length
                } else {
                    0
                };
                messages.push(Message::Synchronize(Synchronize {
                    fork: new_info.fork,
                    length: new_info.length,
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
            info!("\n\t{name} Channel TX:\n\t{messages:#?}");
            channel.send_batch(&messages).await.unwrap();
        }

        _ => {}
    }
    Ok(())
}

/// A PeerState stores the head seq of the remote.
/// This would have a bitfield to support sparse sync in the actual impl.
#[derive(Debug)]
struct PeerState {
    can_upgrade: bool,
    remote_fork: u64,
    /// how long the peer said it's core was
    remote_length: u64,
    remote_can_upgrade: bool,
    remote_uploading: bool,
    remote_downloading: bool,
    remote_synced: bool,
    /// how long the peer thinks our core is
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

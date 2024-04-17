//! trait for replication
// currently this is just a  one-to-one hc replication
// `Message`'s are sent over the "channel" in  `Event::Channel(channel)`.
// These `Event` things come from the `hypercore_protocol::Protocol` stream, which comes from outside.
// `Command` seems to be a way to pass something from a `Channel` stream to the `Protocol` stream.

// TODO add an integration test with a remote JS hypercore. Run a js hypercore and bind it to a
// local address somehow... How? Check out the code use for testing here:
// https://github.com/holepunchto/hypercore/blob/3fda699f306fa3f4781ad66ea13ea0df108a48cd/test/replicate.js
// and the code in the hypercore_protocol rs examples:
// https://github.com/datrs/hypercore-protocol-rs/blob/54d4d91c3fb770688a349e6974eeeff0d50c7c8a/examples-nodejs/replicate.js
//
// set up rust client js server
// make js server
// copy in hb/tests/common/ stuff and adapt it
// TODO next add a test for a Protocol with Event close
use std::{fmt::Debug, marker::Unpin};

use async_std::{
    sync::{Arc, Mutex},
    task::spawn,
};
use futures_lite::{AsyncRead, AsyncWrite, Future, StreamExt};

use thiserror::Error;
use tracing::{error, info, trace, warn};

use random_access_storage::RandomAccess;

use hypercore::{Hypercore, HypercoreError, RequestBlock, RequestUpgrade};
use hypercore_protocol::{
    discovery_key,
    schema::{Data, Range, Request, Synchronize},
    Channel, Event, Message, ProtocolBuilder,
};

#[cfg(feature = "utils")]
pub trait HcTraits: RandomAccess + Debug + Send {}
#[cfg(not(feature = "utils"))]
trait HcTraits: RandomAccess + Debug + Send {}

impl<T: RandomAccess + Debug + Send> HcTraits for T {}

trait StreamTraits: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<S: AsyncRead + AsyncWrite + Send + Unpin + 'static> StreamTraits for S {}

type SharedCore<T> = Arc<Mutex<Hypercore<T>>>;

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

#[macro_export]
macro_rules! r {
    ($core:tt) => {
        $core.lock().await
    };
}

/*
/// the thing that holds replicator state
struct Replicator<T: HcTraits> {
    core: SharedCore<T>,
}
*/

/// unfortunately this thing has to take `self` because it usually consumes the thing
pub trait Replicate {
    #[allow(private_bounds)]
    fn replicate<S: StreamTraits>(
        self,
        stream: S,
        is_initiator: bool,
    ) -> impl Future<Output = Result<(), ReplicatorError>> + Send;
}

impl<T: HcTraits + 'static> Replicate for SharedCore<T> {
    // TODO currently this blocks until the channel closes
    // it should run in the background or something
    // I could prob do this by passing core ino onpeer as a referenced lifetime
    #[allow(private_bounds)]
    fn replicate<S: StreamTraits>(
        self,
        stream: S,
        is_initiator: bool,
    ) -> impl Future<Output = Result<(), ReplicatorError>> + Send {
        let core = self.clone();
        spawn(protocol_msg_loop(core, stream, is_initiator))
    }
}

async fn protocol_msg_loop<T: HcTraits + 'static, S: StreamTraits>(
    core: SharedCore<T>,
    stream: S,
    is_initiator: bool,
) -> Result<(), ReplicatorError> {
    let key = r!(core).key_pair().public.to_bytes().clone();
    let this_dkey = discovery_key(&key);
    let mut protocol = ProtocolBuilder::new(is_initiator).connect(stream);
    while let Some(Ok(event)) = protocol.next().await {
        let core = core.clone();
        info!("protocol event {:?}", event);
        match event {
            Event::Handshake(_m) => {
                if is_initiator {
                    protocol.open(key).await?;
                }
            }
            Event::DiscoveryKey(dkey) => {
                if this_dkey == dkey {
                    protocol.open(key).await?;
                } else {
                    warn!("Got discovery key for different core: {dkey:?}");
                }
            }
            Event::Channel(channel) => {
                if this_dkey == *channel.discovery_key() {
                    onpeer(core, channel).await?;
                }
            }
            Event::Close(_dkey) => {}
            _ => todo!(),
        }
    }
    Ok(())
}

#[allow(private_bounds)]
pub async fn onpeer<T: HcTraits + 'static>(
    core: SharedCore<T>,
    mut channel: Channel,
) -> Result<(), ReplicatorError> {
    let mut peer_state = PeerState::default();
    let info = r!(core).info();

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
        info!("Channel.send_batch([{sync_msg:?}, {range_msg:?}])");
        channel
            .send_batch(&[Message::Synchronize(sync_msg), Message::Range(range_msg)])
            .await?;
    } else {
        info!("channel.send({sync_msg:?})");
        channel.send(Message::Synchronize(sync_msg)).await.unwrap();
    }

    spawn(async move {
        info!("Start listening to channel messages");
        while let Some(message) = channel.next().await {
            let result = onmessage(core.clone(), &mut peer_state, &mut channel, message).await;
            if let Err(e) = result {
                info!("protocol error: {}", e);
                break;
            }
        }
    });
    Ok(())
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

async fn onmessage<T: HcTraits>(
    core: SharedCore<T>,
    peer_state: &mut PeerState,
    channel: &mut Channel,
    message: Message,
) -> Result<(), ReplicatorError> {
    match message {
        Message::Synchronize(message) => {
            info!("Got Synchronize message {message:?}");
            let length_changed = message.length != peer_state.remote_length;
            let first_sync = !peer_state.remote_synced;
            let info = r!(core).info();
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
            info!("Got Request message {message:?}");
            let (info, proof) = {
                let proof = r!(core)
                    .create_proof(message.block, message.hash, message.seek, message.upgrade)
                    .await
                    .unwrap();
                //TODO .await?;
                (r!(core).info(), proof)
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
            info!("Got Data message {message:?}");
            let (_old_info, _applied, new_info, request_block) = {
                let old_info = r!(core).info();
                let proof = message.clone().into_proof();
                let applied = r!(core).verify_and_apply_proof(&proof).await?;
                let new_info = r!(core).info();
                let request_block: Option<RequestBlock> = if let Some(upgrade) = &message.upgrade {
                    // When getting the initial upgrade, send a request for the first missing block
                    if old_info.length < upgrade.length {
                        let request_index = old_info.length;
                        let nodes = r!(core).missing_nodes(request_index).await?;
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
                        let nodes = r!(core).missing_nodes(request_index).await?;
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
                            String::from_utf8(r!(core).get(i).await?.unwrap()).unwrap()
                        );
                    }
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

#[cfg(test)]
mod test {

    use async_std::task::sleep;
    use hypercore_protocol::Duplex;
    use piper::pipe;
    use std::time::Duration;

    use hypercore::{generate_signing_key, HypercoreBuilder, PartialKeypair, Storage};

    use super::*;

    static PIPE_CAPACITY: usize = 1024 * 1024 * 4;

    fn make_reader_and_writer_keys() -> (PartialKeypair, PartialKeypair) {
        let signing_key = generate_signing_key();
        let writer_key = PartialKeypair {
            public: signing_key.verifying_key(),
            secret: Some(signing_key),
        };
        let mut reader_key = writer_key.clone();
        reader_key.secret = None;
        (reader_key, writer_key)
    }

    #[tokio::test]
    async fn one_to_one() -> Result<(), ReplicatorError> {
        //utils::setup_logs().await;

        let (reader_key, writer_key) = make_reader_and_writer_keys();
        let writer_core = Arc::new(Mutex::new(
            HypercoreBuilder::new(Storage::new_memory().await.unwrap())
                .key_pair(writer_key)
                .build()
                .await
                .unwrap(),
        ));

        // add data
        let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];
        r!(writer_core).append_batch(batch).await.unwrap();

        let reader_core = Arc::new(Mutex::new(
            HypercoreBuilder::new(Storage::new_memory().await.unwrap())
                .key_pair(reader_key)
                .build()
                .await
                .unwrap(),
        ));

        let (read_from_writer, write_to_writer) = pipe(PIPE_CAPACITY);
        let (read_from_reader, write_to_reader) = pipe(PIPE_CAPACITY);

        let stream_to_reader = Duplex::new(read_from_writer, write_to_reader);
        let stream_to_writer = Duplex::new(read_from_reader, write_to_writer);
        let _server = spawn(writer_core.replicate(stream_to_writer, false));
        let _client = spawn(reader_core.clone().replicate(stream_to_reader, true));
        loop {
            let length = r!(reader_core).info().length;
            dbg!(&length);
            if r!(reader_core).info().length == 4 {
                if let Some(block) = r!(reader_core).get(3).await? {
                    dbg!(String::from_utf8_lossy(&block));
                    break;
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }
}

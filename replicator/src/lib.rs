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
//
// problem reader gets sync from writer but Sync.remote_length = 1 which isn't right. remote
// (reader here) is 2. writer got remote length from readers last sync message.
// but that message in reader took the length from a data message from writer. it is
// data.upgrade.length.
use std::{fmt::Debug, marker::Unpin};

use async_std::{
    sync::{Arc, Mutex, RwLock},
    task::spawn,
};
use futures_lite::{AsyncRead, AsyncWrite, Future, StreamExt};

use thiserror::Error;
use tracing::{error, info, trace, warn};

use hypercore::{DataUpgrade, Hypercore, HypercoreError, RequestBlock, RequestUpgrade};
use hypercore_protocol::{
    discovery_key,
    schema::{Data, Range, Request, Synchronize},
    Channel, Event, Message, Protocol, ProtocolBuilder,
};

trait StreamTraits: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<S: AsyncRead + AsyncWrite + Send + Unpin + 'static> StreamTraits for S {}

type ShareRw<T> = Arc<RwLock<T>>;
type SharedCore = Arc<Mutex<Hypercore>>;

async fn is_writer(c: SharedCore) -> bool {
    lk!(c).key_pair().secret.is_some()
}

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

macro_rules! name {
    ($core:tt) => {
        if is_writer($core.clone()).await {
            "Writer"
        } else {
            "Reader"
        }
    };
}

struct DebugUpgrade(DataUpgrade);
impl Debug for DebugUpgrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = &self.0;
        f.debug_struct("DUpgrade")
            .field("start", &d.start)
            .field("length", &d.length)
            //.field("nodes", &d.nodes)
            .finish()
    }
}

struct DebugData(Data);
impl Debug for DebugData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = &self.0;
        let u = match &d.upgrade {
            Some(u) => Some(DebugUpgrade(u.clone())),
            None => None,
        };
        f.debug_struct("DData")
            .field("request", &d.request)
            .field("fork", &d.fork)
            .field("upgrade", &u)
            .finish()
    }
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

pub struct HcReplicator {
    core: SharedCore,
}

#[allow(private_bounds)] // TODO rmme
pub struct Peer<IO: StreamTraits> {
    core: SharedCore,
    protocol: ShareRw<Protocol<IO>>,
    message_buff: ShareRw<Vec<Message>>,
}

#[allow(private_bounds)] // TODO rmme
impl<IO: StreamTraits> Peer<IO> {
    fn new(core: SharedCore, protocol: ShareRw<Protocol<IO>>) -> Self {
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
                println!("{msg}");
                message_buff.write().await.push(msg);
            }
        });
    }

    async fn start_message_loop(&self, is_initiator: bool) -> Result<(), ReplicatorError> {
        let key = self.core.lock().await.key_pair().public.to_bytes().clone();
        let this_dkey = discovery_key(&key);
        let name = "snthsth";
        //let name = name!(self.core);
        let protocol = self.protocol.clone();
        while let Some(Ok(event)) = {
            // this block is just here to release the `.write()` lock
            let p = protocol.write().await.next().await;
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
                        onpeer(self.core.clone(), channel).await?;
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

#[allow(private_bounds)]
impl HcReplicator {
    pub fn new(core: SharedCore) -> Self {
        Self { core }
    }

    pub async fn add_peer<S: StreamTraits>(
        &mut self,
        stream: S,
        is_initiator: bool,
    ) -> Result<Peer<S>, ReplicatorError> {
        let core = self.core.clone();
        let protocol = ProtocolBuilder::new(is_initiator).connect(stream);
        Ok(Peer::new(core, Arc::new(RwLock::new(protocol))))
    }

    pub async fn add_stream<S: StreamTraits + Sync>(
        &mut self,
        stream: S,
        is_initiator: bool,
    ) -> Result<(), ReplicatorError> {
        let peer = self.add_peer(stream, is_initiator).await?;
        spawn(async move {
            peer.listen_to_channel_messages().await;
            peer.start_message_loop(is_initiator).await?;
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
    let name = name!(core);
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

pub async fn onpeer(core: SharedCore, mut channel: Channel) -> Result<(), ReplicatorError> {
    let peer_state = Arc::new(RwLock::new(PeerState::default()));

    initiate_sync(core.clone(), peer_state.clone(), &mut channel).await?;

    let _event_loop = spawn(core_event_loop(
        core.clone(),
        peer_state.clone(),
        channel.clone(),
    ));
    let _channel_rx_loop = spawn(async move {
        while let Some(message) = channel.next().await {
            let result =
                onmessage(core.clone(), peer_state.clone(), channel.clone(), message).await;
            if let Err(e) = result {
                trace!("protocol error: {}", e);
                break;
            }
        }
    });
    Ok(())
}

async fn onmessage(
    core: SharedCore,
    peer_state: ShareRw<PeerState>,
    mut channel: Channel,
    message: Message,
) -> Result<(), ReplicatorError> {
    let name = name!(core);
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
                    // HERE
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
                info!("\n\t{name} Channel TX:\n\t{:#?}", DebugData(msg.clone()));
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
            if let Some(upgrade) = &message.upgrade {
                let new_length = upgrade.length;
                let remote_length = if new_info.fork == r!(peer_state).remote_fork {
                    r!(peer_state).remote_length
                } else {
                    0
                };
                messages.push(Message::Synchronize(Synchronize {
                    fork: new_info.fork,
                    length: new_length,
                    // HERE
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
    };
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

#[cfg(test)]
mod test {

    use async_std::task::{sleep, JoinHandle};
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
    async fn create_connected_cores<A: AsRef<[u8]>, B: AsRef<[A]>>(
        initial_data: B,
    ) -> Result<
        (
            (SharedCore, JoinHandle<Result<(), ReplicatorError>>),
            (SharedCore, JoinHandle<Result<(), ReplicatorError>>),
        ),
        ReplicatorError,
    > {
        let (reader_key, writer_key) = make_reader_and_writer_keys();
        let writer_core = Arc::new(Mutex::new(
            HypercoreBuilder::new(Storage::new_memory().await.unwrap())
                .key_pair(writer_key)
                .build()
                .await
                .unwrap(),
        ));

        // add data
        lk!(writer_core).append_batch(initial_data).await.unwrap();

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

        let mut server_replicator = writer_core.clone().replicate().await?;
        let mut client_replicator = reader_core.clone().replicate().await?;

        assert_eq!(
            writer_core.lock().await.key_pair().public,
            reader_core.lock().await.key_pair().public
        );

        let _server =
            spawn(async move { server_replicator.add_stream(stream_to_writer, false).await });
        let _client =
            spawn(async move { client_replicator.add_stream(stream_to_reader, true).await });

        Ok(((writer_core, _server), (reader_core, _client)))
    }

    #[tokio::test]
    async fn initial_data_replicated() -> Result<(), ReplicatorError> {
        let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];
        let (_, (reader_core, _)) = create_connected_cores(batch).await?;
        for i in 0..batch.len() {
            loop {
                if lk!(reader_core).info().length as usize >= i + 1 {
                    if let Some(block) = lk!(reader_core).get(i as u64).await? {
                        if block == batch[i].to_vec() {
                            break;
                        }
                    }
                }
                sleep(Duration::from_millis(100)).await;
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn initial_sync() -> Result<(), ReplicatorError> {
        let _ = create_connected_cores(vec![] as Vec<&[u8]>).await?;
        Ok(())
    }
    #[tokio::test]
    async fn new_data_replicated() -> Result<(), ReplicatorError> {
        let data: Vec<&[u8]> = vec![b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];
        let ((writer_core, _), (reader_core, _)) =
            create_connected_cores(vec![] as Vec<&[u8]>).await?;
        for i in 0..data.len() {
            dbg!(i);
            if i == 2 {
                println!("START THE LOGS");
            }
            writer_core.lock().await.append(data[i as usize]).await?;
            println!("Block i = {i} appended [{:?}]", data[i as usize]);
            while (lk!(reader_core).info().length as usize) < i + 1 {
                sleep(Duration::from_millis(10)).await;
            }
            let len = lk!(reader_core).info().length;
            assert_eq!(len as usize, i + 1);
            loop {
                let block = lk!(reader_core).get(i as u64).await?;
                if block == Some(data[i as usize].to_vec()) {
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }
        }
        Ok(())
    }
}

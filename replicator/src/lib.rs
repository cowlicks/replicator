//! Implement replication for Hypercores.
//!
//! The [`hypercore`] crate exposes a the [`CoreMethods`] trait, which contains methods for reading
//! and writing to a [`Hypercore`]. We use this trait for things we want to treat like hypercores,
//! but aren't, like a `Arc<Mutex<Hypercore>>`. We implement [`CoreMethods`] on [`Replicator`]
//! to provide a thing that replicates and can still be treated like a [`Hypercore`].
//!
//! The [`Replicate`] trait defines a method that returns the [`Replicator`] struct that implements
//! [`CoreMethods`].
//!

#![warn(missing_debug_implementations, rust_2024_compatibility)]
#[cfg(test)]
mod test;

#[cfg(any(test, feature = "utils"))]
pub mod utils;

use std::{fmt::Debug, marker::Unpin, sync::Arc};

use futures_lite::{AsyncRead, AsyncWrite, Future, StreamExt};

use thiserror::Error;
use tracing::{error, trace, warn};

use tokio::{spawn, sync::RwLock, task::JoinHandle};

use hypercore::{
    replication::{
        CoreInfo, CoreMethods, CoreMethodsError, ReplicationMethods, ReplicationMethodsError,
        SharedCore,
    },
    Hypercore, HypercoreError, RequestBlock, RequestUpgrade,
};
use hypercore_protocol::{
    discovery_key,
    schema::{Data, Range, Request, Synchronize},
    Channel, Event, Key, Message, Protocol, ProtocolBuilder,
};

type ShareRw<T> = Arc<RwLock<T>>;

macro_rules! reader_or_writer {
    ($core:ident) => {
        if $core.key_pair().await.secret.is_some() {
            "Writer"
        } else {
            "Reader"
        }
    };
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ReplicatorError {
    #[error("There was an error in the opening the protocol in handshake: [{0}]")]
    IoError(#[from] std::io::Error),
    #[error("HypercoreError: [{0}]")]
    HypercoreError(#[from] HypercoreError),
    #[error("ReplMethodsError: [{0}]")]
    ReplMethodsError(#[from] ReplicationMethodsError),
    #[error("CoreMethodsError: [{0}]")]
    CoreMethodsError(#[from] CoreMethodsError),
}

/// Enables hypercore replication
/// TODO have this return trait instead of a struct
/// maybe rename `Replicate` -> `Replicatable`, and have `Replicatable.replicate() -> impl Replicator`
/// like how JS has [`Iterables` and
/// `Iterators`](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Guide/Iterators_and_generators#iterables).
///
/// trait Replicator: CoreMethods { .. }
///
/// trait Replicatable {
///     fn replicate() -> Replicator
/// }
pub trait Replicate {
    fn replicate(&self) -> ReplicatingCore;
}

impl Replicate for SharedCore {
    fn replicate(&self) -> ReplicatingCore {
        ReplicatingCore::new(self.clone())
    }
}

#[async_trait::async_trait]
trait ProtoMethods: Debug + Send + Sync {
    async fn open(&mut self, key: Key) -> std::io::Result<()>;
    async fn _next(&mut self) -> Option<std::io::Result<Event>>;
}

#[async_trait::async_trait]
impl<S: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static> ProtoMethods for Protocol<S> {
    async fn open(&mut self, key: Key) -> std::io::Result<()> {
        Protocol::open(self, key).await
    }
    async fn _next(&mut self) -> Option<std::io::Result<Event>> {
        futures_lite::StreamExt::next(&mut self).await
    }
}

pub struct Peer {
    /// reference to the parent core
    core: SharedCore,
    /// stream of events to the peer
    protocol: ShareRw<Box<dyn ProtoMethods>>,
}

impl Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peer")
            //.field("core", &self.core)
            //.field("protocol", &self.protocol)
            //.field("message_buff", &self.message_buff)
            .finish()
    }
}

impl Peer {
    fn new(core: SharedCore, protocol: ShareRw<Box<dyn ProtoMethods>>) -> Self {
        Self { core, protocol }
    }

    async fn start_message_loop(&self, is_initiator: bool) -> Result<(), ReplicatorError> {
        let key = self.core.key_pair().await.public.to_bytes();
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
            trace!("\n\t{name} Proto RX:\n\t{:#?}", event);
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
                        on_peer(core.clone(), channel).await?;
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

#[derive(Debug, Clone)]
pub struct Peers(ShareRw<Vec<ShareRw<Peer>>>);

impl Peers {
    fn new() -> Self {
        Self(Arc::new(RwLock::new(vec![])))
    }
    pub async fn push(&self, peer: ShareRw<Peer>) {
        self.0.write().await.push(peer);
    }
    pub async fn get(&self, i: usize) -> ShareRw<Peer> {
        self.0.read().await[i].clone()
    }
}

#[derive(Debug, Clone)]
pub struct ReplicatingCore {
    core: SharedCore,
    peers: Peers,
}

impl From<Hypercore> for ReplicatingCore {
    fn from(core: Hypercore) -> Self {
        let sc: SharedCore = core.into();
        ReplicatingCore::new(sc)
    }
}

impl From<SharedCore> for ReplicatingCore {
    fn from(core: SharedCore) -> Self {
        ReplicatingCore::new(core)
    }
}

impl ReplicatingCore {
    pub fn new(core: SharedCore) -> Self {
        Self {
            core,
            peers: Peers::new(),
        }
    }

    async fn add_peer<S: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static>(
        &self,
        stream: S,
        is_initiator: bool,
    ) -> ShareRw<Peer> {
        let core = self.core.clone();
        let protocol = ProtocolBuilder::new(is_initiator).connect(stream);

        let peer = Arc::new(RwLock::new(Peer::new(
            core,
            Arc::new(RwLock::new(Box::new(protocol))),
        )));
        self.peers.push(peer.clone()).await;
        peer
    }

    pub async fn add_stream<S: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static>(
        &self,
        stream: S,
        is_initiator: bool,
    ) {
        let peer = self.add_peer(stream, is_initiator).await;
        spawn(async move {
            peer.read().await.start_message_loop(is_initiator).await?;
            Ok::<(), ReplicatorError>(())
        });
    }
}

impl CoreInfo for ReplicatingCore {
    fn info(&self) -> impl Future<Output = hypercore::Info> + Send {
        self.core.info()
    }
    fn key_pair(&self) -> impl Future<Output = hypercore::PartialKeypair> + Send {
        self.core.key_pair()
    }
}
impl CoreMethods for ReplicatingCore {
    fn get(
        &self,
        index: u64,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, CoreMethodsError>> + Send {
        self.core.get(index)
    }

    fn append(
        &self,
        data: &[u8],
    ) -> impl Future<Output = Result<hypercore::AppendOutcome, CoreMethodsError>> + Send {
        self.core.append(data)
    }
}

async fn initiate_sync(
    core: impl ReplicationMethods + Clone + 'static,
    peer_state: ShareRw<PeerState>,
    channel: &mut Channel,
) -> Result<(), ReplicatorError> {
    let info = core.info().await;
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
        channel
            .send_batch(&[Message::Synchronize(sync_msg), Message::Range(range_msg)])
            .await?;
    } else {
        channel.send(Message::Synchronize(sync_msg)).await?;
    }
    Ok(())
}

async fn core_event_loop(
    core: impl ReplicationMethods + Clone + 'static,
    peer_state: ShareRw<PeerState>,
    mut channel: Channel,
) -> Result<(), ReplicatorError> {
    let mut on_append = core.on_append_subscribe().await;
    while let Ok(_event) = on_append.recv().await {
        trace!("got core upgrade event. Notifying peers");
        initiate_sync(core.clone(), peer_state.clone(), &mut channel).await?
    }
    Ok(())
}

async fn on_get_loop(
    core: impl ReplicationMethods + Clone + 'static,
    channel: Channel,
) -> Result<(), ReplicatorError> {
    let mut on_get_events = core.on_get_subscribe().await;
    while let Ok((index, _tx)) = on_get_events.recv().await {
        trace!("got on_get({index}) event. Notifying peers");
        on_get(core.clone(), channel.clone(), index);
    }
    Ok(())
}
fn on_get(
    core: impl ReplicationMethods + 'static,
    channel: Channel,
    index: u64,
) -> JoinHandle<Result<(), ReplicatorError>> {
    spawn(on_get_inner(core, channel, index))
}

async fn on_get_inner(
    core: impl ReplicationMethods,
    mut channel: Channel,
    index: u64,
) -> Result<(), ReplicatorError> {
    let block = RequestBlock {
        index,
        nodes: core.missing_nodes(index).await?,
    };
    let msg = Message::Request(Request {
        fork: core.info().await.fork,
        id: block.index + 1,
        block: Some(block),
        hash: None,
        seek: None,
        upgrade: None,
    });

    channel.send_batch(&[msg]).await?;
    Ok(())
}

async fn on_peer(core: SharedCore, mut channel: Channel) -> Result<(), ReplicatorError> {
    let peer_state = Arc::new(RwLock::new(PeerState::default()));

    initiate_sync(core.clone(), peer_state.clone(), &mut channel).await?;

    let _event_loop = spawn(core_event_loop(
        core.clone(),
        peer_state.clone(),
        channel.clone(),
    ));

    let _on_get_loop = spawn(on_get_loop(core.clone(), channel.clone()));

    let _channel_rx_loop = spawn(async move {
        while let Some(message) = channel.next().await {
            on_message(core.clone(), peer_state.clone(), channel.clone(), message);
        }
    });
    Ok(())
}

fn on_message(
    core: SharedCore,
    peer_state: ShareRw<PeerState>,
    channel: Channel,
    message: Message,
) -> JoinHandle<Result<(), ReplicatorError>> {
    spawn(async move {
        if let Err(e) = on_message_inner(core, peer_state, channel, message).await {
            error!("Error handling message: {e}");
            return Err(e);
        }
        Ok(())
    })
}

async fn on_message_inner(
    core: impl ReplicationMethods,
    peer_state: ShareRw<PeerState>,
    mut channel: Channel,
    message: Message,
) -> Result<(), ReplicatorError> {
    let name = reader_or_writer!(core);
    trace!("{name}:{}:RX {}", channel.name, message.kind());
    match message {
        Message::Synchronize(message) => {
            let peer_length_changed = message.length != peer_state.read().await.remote_length;
            let first_sync = !peer_state.read().await.remote_synced;
            let info = core.info().await;
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
                    remote_length: peer_state.read().await.remote_length,
                    can_upgrade: peer_state.read().await.can_upgrade,
                    uploading: true,
                    downloading: true,
                };
                messages.push(Message::Synchronize(msg));
            }

            // if peer is longer than us
            if peer_state.read().await.remote_length > info.length
                // and peer knows our correct length
                && peer_state.read().await.length_acked == info.length
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
                        length: peer_state.read().await.remote_length - info.length,
                    }),
                };

                messages.push(Message::Request(msg));
            }
            if !messages.is_empty() {
                channel.send_batch(&messages).await?;
            }
        }

        Message::Request(message) => {
            let (info, proof) = {
                let proof = core
                    .create_proof(message.block, message.hash, message.seek, message.upgrade)
                    .await?;
                (core.info().await, proof)
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
            let (_old_info, _applied, new_info, request_block) = {
                let old_info = core.info().await;

                let proof = message.clone().into_proof();
                let applied = core.verify_and_apply_proof(&proof).await?;
                let new_info = core.info().await;

                let request_block: Option<RequestBlock> = if let Some(upgrade) = &message.upgrade {
                    // When getting the initial upgrade, send a request for the first missing block
                    if old_info.length < upgrade.length {
                        let request_index = old_info.length;
                        let nodes = core.missing_nodes(request_index).await?;
                        Some(RequestBlock {
                            index: request_index,
                            nodes,
                        })
                    } else {
                        None
                    }
                } else if let Some(block) = &message.block {
                    // When receiving a block, ask for the next, if there are still some missing
                    if block.index < peer_state.read().await.remote_length - 1 {
                        let request_index = block.index + 1;
                        let nodes = core.missing_nodes(request_index).await?;
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

                if new_info.contiguous_length == new_info.length {
                    trace!("All data replicated. length = {}", new_info.length);
                }
                (old_info, applied, new_info, request_block)
            };

            let mut messages: Vec<Message> = vec![];

            // If we got an upgrade send a Sync
            if message.upgrade.is_some() {
                let remote_length = if new_info.fork == peer_state.read().await.remote_fork {
                    peer_state.read().await.remote_length
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
            if !messages.is_empty() {
                channel.send_batch(&messages).await.unwrap();
            }
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

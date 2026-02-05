use std::{
    pin::{Pin, pin},
    sync::{Arc, RwLock},
    task::{Context, Poll},
};

use bitfield::Bitfield;
use futures::{Sink, Stream, stream::FuturesUnordered};
use futures_lite::StreamExt;
use hypercore::{Hypercore, PartialKeypair};
use hypercore_protocol::{
    Channel, DiscoveryKey, HandshakeResult, Message, Protocol, discovery_key,
    schema::{Range, Request, Synchronize},
};
use hypercore_schema::RequestUpgrade;
use tracing::{error, trace, warn};

#[derive(Debug)]
pub struct Peer {
    /// stream of events to the peer
    protocol: Protocol,
    discovery_key: DiscoveryKey,
    public_key: [u8; 32], // TODO use real const var for 32
    pending_opens: FuturesUnordered<Pin<Box<dyn Future<Output = Result<(), std::io::Error>>>>>,
    channel_state: Option<ChannelState>,
}

impl Peer {
    fn new(keypair: PartialKeypair, protocol: Protocol) -> Self {
        let public_key = keypair.public.to_bytes();
        let discovery_key = discovery_key(&public_key);
        Self {
            protocol,
            discovery_key,
            public_key,
            pending_opens: Default::default(),
            channel_state: Default::default(),
        }
    }
    fn poll_message_loop(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<()>> {
        let event = match Pin::new(&mut self.protocol).poll_next(cx) {
            Poll::Ready(Some(Ok(e))) => e,
            Poll::Pending => return Poll::Pending,
            _ => todo!(),
        };
        match event {
            hypercore_protocol::Event::Handshake(_remote_public_key) => {
                if self.protocol.is_initiator() {
                    let opening = self.protocol.open(self.public_key);
                    self.pending_opens.push(Box::pin(opening));
                }
                todo!()
            }
            hypercore_protocol::Event::DiscoveryKey(dkey) => {
                if self.discovery_key == dkey {
                    let opening = self.protocol.open(self.public_key);
                    self.pending_opens.push(Box::pin(opening));
                } else {
                    warn!("Got discovery key for different core: {dkey:?}");
                }
                todo!()
            }
            hypercore_protocol::Event::Channel(channel) => {
                if self.discovery_key == *channel.discovery_key() {
                    //on_peer(core.clone(), channel).await?;
                } else {
                    error!("Wrong discovery key?");
                }
                todo!()
            }
            hypercore_protocol::Event::Close(_) => todo!(),
            hypercore_protocol::Event::LocalSignal(_) => todo!(),
            _ => todo!(),
        }
    }
}

#[derive(Debug)]
struct ChannelState {
    core: Arc<RwLock<Hypercore>>,
    channel: Channel,
    state: crate::PeerState,
    synced: bool,
    pending_sends: FuturesUnordered<Pin<Box<dyn Future<Output = Result<(), std::io::Error>>>>>,
}

impl ChannelState {
    fn new(core: Arc<RwLock<Hypercore>>, channel: Channel) -> Self {
        Self {
            core,
            channel,
            state: crate::PeerState::default(),
            synced: false,
            pending_sends: Default::default(),
        }
    }

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();

        // Drive pending sends
        loop {
            match Pin::new(&mut this.pending_sends).poll_next(cx) {
                Poll::Ready(Some(Ok(()))) => continue,
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                Poll::Ready(None) => break,
                Poll::Pending => break,
            }
        }

        // Initial sync
        if !this.synced {
            let info = this.core.read().unwrap().info();

            let remote_length = if info.fork == this.state.remote_fork {
                this.state.remote_length
            } else {
                0
            };

            let mut messages = vec![Message::Synchronize(Synchronize {
                fork: info.fork,
                length: info.length,
                remote_length,
                can_upgrade: this.state.can_upgrade,
                uploading: true,
                downloading: true,
            })];

            if info.contiguous_length > 0 {
                messages.push(Message::Range(Range {
                    drop: false,
                    start: 0,
                    length: info.contiguous_length,
                }));
            }
            this.pending_sends
                .push(Box::pin(this.channel.send_batch(&messages)));

            this.synced = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }

        // Poll channel for incoming messages
        match Pin::new(&mut this.channel).poll_next(cx) {
            Poll::Ready(Some(message)) => {
                trace!("RX:{:?}", message);
                this.on_message(message);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }

    fn on_message(&mut self, message: Message) {
        match message {
            Message::Synchronize(msg) => self.on_synchronize(msg),
            Message::Range(Range { start, length, .. }) => {
                self.state.remote_bitfield.set_range(start, length, true);
            }
            Message::Request(_msg) => {
                // TODO: needs async core.create_proof()
                todo!()
            }
            Message::Data(_msg) => {
                // TODO: needs async core.verify_and_apply_proof()
                todo!()
            }
            _ => {}
        }
    }

    fn on_synchronize(&mut self, msg: Synchronize) {
        let info = self.core.read().unwrap().info();
        let peer_length_changed = msg.length != self.state.remote_length;
        let first_sync = !self.state.remote_synced;
        let same_fork = msg.fork == info.fork;

        self.state.remote_fork = msg.fork;
        self.state.remote_length = msg.length;
        self.state.remote_can_upgrade = msg.can_upgrade;
        self.state.remote_uploading = msg.uploading;
        self.state.remote_downloading = msg.downloading;
        self.state.remote_synced = true;
        self.state.length_acked = if same_fork { msg.remote_length } else { 0 };

        let mut messages = vec![];
        if first_sync {
            messages.push(Message::Synchronize(Synchronize {
                fork: info.fork,
                length: info.length,
                remote_length: self.state.remote_length,
                can_upgrade: self.state.can_upgrade,
                uploading: true,
                downloading: true,
            }));
        }

        if self.state.remote_length > info.length
            && self.state.length_acked == info.length
            && peer_length_changed
        {
            messages.push(Message::Request(Request {
                id: 1, // There should be proper handling for in-flight request ids
                fork: info.fork,
                hash: None,
                block: None,
                seek: None,
                upgrade: Some(RequestUpgrade {
                    start: info.length,
                    length: self.state.remote_length - info.length,
                }),
                manifest: false, // Dummy values
                priority: 42,
            }));
        }
        if !messages.is_empty() {
            self.pending_sends
                .push(Box::pin(self.channel.send_batch(&messages)));
        }
    }
}

pub struct Replicator {
    core: Hypercore,
    peers: Vec<Peer>,
}
impl Replicator {
    pub fn add_stream<S>(&mut self, stream: S, is_initiator: bool, handshake: HandshakeResult)
    where
        S: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin + Send + Sync + 'static,
        <S as Sink<Vec<u8>>>::Error: Into<std::io::Error>,
    {
        let protocol = Protocol::new(stream, is_initiator, handshake);
        let keypair = self.core.key_pair();
        self.peers.push(Peer::new(keypair.clone(), protocol));
    }
}

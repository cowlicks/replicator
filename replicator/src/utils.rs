use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    Sink, Stream,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
};
use hypercore::{
    generate_signing_key,
    replication::{CoreMethods, SharedCore},
    HypercoreBuilder, PartialKeypair, Storage,
};
use hypercore_protocol::HandshakeResult;

use crate::{Replicate, ReplicatingCore, ReplicatorError};

/// Create a test HandshakeResult with dummy keys.
/// The keys are deterministic based on is_initiator to ensure both sides
/// have compatible handshake results.
pub fn test_handshake(is_initiator: bool) -> HandshakeResult {
    // Use deterministic keys - initiator gets [0; 32], responder gets [1; 32]
    let (local, remote) = if is_initiator {
        ([0u8; 32], [1u8; 32])
    } else {
        ([1u8; 32], [0u8; 32])
    };
    HandshakeResult::from_pre_encrypted(is_initiator, local, remote, vec![0u8; 64])
}

/// Bidirectional pipe for testing that implements Stream + Sink.
#[derive(Debug)]
pub struct BiDiPipe {
    receiver: UnboundedReceiver<Vec<u8>>,
    sender: UnboundedSender<Vec<u8>>,
}

impl Stream for BiDiPipe {
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

impl Sink<Vec<u8>> for BiDiPipe {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        Pin::new(&mut self.sender)
            .start_send(item)
            .map_err(|_e| io::Error::other("SendError"))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Create a pair of connected bidirectional pipes.
/// Data sent on one pipe is received on the other.
pub fn create_connected_streams() -> (BiDiPipe, BiDiPipe) {
    let (tx_a, rx_a) = unbounded();
    let (tx_b, rx_b) = unbounded();

    let stream_a = BiDiPipe {
        receiver: rx_b,
        sender: tx_a,
    };
    let stream_b = BiDiPipe {
        receiver: rx_a,
        sender: tx_b,
    };
    (stream_a, stream_b)
}

pub async fn public_key(core: &impl CoreMethods) -> PartialKeypair {
    let PartialKeypair { public, .. } = core.key_pair().await;
    PartialKeypair {
        public,
        secret: None,
    }
}

pub fn make_reader_and_writer_keys() -> (PartialKeypair, PartialKeypair) {
    let signing_key = generate_signing_key();
    let writer_key = PartialKeypair {
        public: signing_key.verifying_key(),
        secret: Some(signing_key),
    };
    let mut reader_key = writer_key.clone();
    reader_key.secret = None;
    (reader_key, writer_key)
}

pub async fn make_slave(master: &SharedCore) -> Result<SharedCore, ReplicatorError> {
    let key = public_key(master).await;

    let core: SharedCore = HypercoreBuilder::new(Storage::new_memory().await.unwrap())
        .key_pair(key)
        .build()
        .await?
        .into();
    Ok(core)
}

pub async fn make_connected_slave(
    master: &ReplicatingCore,
    master_is_initiator: bool,
) -> Result<ReplicatingCore, ReplicatorError> {
    let key = public_key(master).await;

    let slave: ReplicatingCore = HypercoreBuilder::new(Storage::new_memory().await.unwrap())
        .key_pair(key)
        .build()
        .await?
        .into();

    let (a_b, b_a) = create_connected_streams();

    master
        .add_stream(a_b, master_is_initiator, test_handshake(master_is_initiator))
        .await;
    slave
        .add_stream(b_a, !master_is_initiator, test_handshake(!master_is_initiator))
        .await;

    Ok(slave)
}

pub async fn writer_and_reader_cores() -> Result<(SharedCore, SharedCore), ReplicatorError> {
    let writer_core: SharedCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .build()
        .await?
        .into();

    let reader_core = make_slave(&writer_core).await?;
    Ok((writer_core, reader_core))
}

pub async fn create_connected_cores<A: AsRef<[u8]>, B: AsRef<[A]>>(
    initial_data: B,
) -> ((SharedCore, ReplicatingCore), (SharedCore, ReplicatingCore)) {
    let (writer_core, reader_core) = writer_and_reader_cores().await.unwrap();

    writer_core
        .0
        .lock()
        .await
        .append_batch(initial_data)
        .await
        .unwrap();

    let (a_b, b_a) = create_connected_streams();
    let server_replicator = writer_core.clone().replicate();
    let client_replicator = reader_core.clone().replicate();

    server_replicator
        .add_stream(a_b, true, test_handshake(true))
        .await;
    client_replicator
        .add_stream(b_a, false, test_handshake(false))
        .await;

    (
        (writer_core, server_replicator),
        (reader_core, client_replicator),
    )
}

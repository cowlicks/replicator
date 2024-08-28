use crate::{Replicate, Replicator, ReplicatorError};
use hypercore::{generate_signing_key, HypercoreBuilder, PartialKeypair, SharedCore, Storage};
use hypercore_protocol::Duplex;
use piper::{pipe, Reader, Writer};

static PIPE_CAPACITY: usize = 1024 * 1024 * 4;

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

type S = Duplex<Reader, Writer>;
pub fn create_connected_streams() -> (S, S) {
    let (read_from_a, write_to_a) = pipe(PIPE_CAPACITY);
    let (read_from_b, write_to_b) = pipe(PIPE_CAPACITY);

    let stream_to_b = Duplex::new(read_from_a, write_to_b);
    let stream_to_a = Duplex::new(read_from_b, write_to_a);
    (stream_to_a, stream_to_b)
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

    server_replicator.add_stream(a_b, true).await.unwrap();
    client_replicator.add_stream(b_a, false).await.unwrap();

    (
        (writer_core, server_replicator),
        (reader_core, client_replicator),
    )
}

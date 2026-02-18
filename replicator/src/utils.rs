use crate::{Replicate, ReplicatingCore, ReplicatorError};
use hypercore::{
    HypercoreBuilder, PartialKeypair, Storage, generate_signing_key,
    replication::{CoreMethods, SharedCore},
};
use hypercore_handshake::{
    Cipher,
    state_machine::{SecStream, hc_specific::generate_keypair},
};
use tokio_util::compat::TokioAsyncReadCompatExt;
use uint24le_framing::Uint24LELengthPrefixedFraming;

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

    let (a_b, b_a) = tokio::io::duplex(10_000);
    let a_b = Uint24LELengthPrefixedFraming::new(a_b.compat());
    let b_a = Uint24LELengthPrefixedFraming::new(b_a.compat());

    let mkey = generate_keypair().unwrap();
    let master_stream = if master_is_initiator {
        Cipher::new(
            Some(Box::new(a_b)),
            SecStream::new_initiator_xx(&[]).unwrap().into(),
        )
    } else {
        Cipher::new(
            Some(Box::new(a_b)),
            SecStream::new_responder_xx(&mkey, &[]).unwrap().into(),
        )
    };

    let skey = generate_keypair().unwrap();
    let slave_stream = if !master_is_initiator {
        Cipher::new(
            Some(Box::new(b_a)),
            SecStream::new_initiator_xx(&[]).unwrap().into(),
        )
    } else {
        Cipher::new(
            Some(Box::new(b_a)),
            SecStream::new_responder_xx(&skey, &[]).unwrap().into(),
        )
    };
    master.add_stream(master_stream).await;
    slave.add_stream(slave_stream).await;

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

    let server_is_initiator = true;
    let (a_b, b_a) = tokio::io::duplex(10_000);
    let a_b = Uint24LELengthPrefixedFraming::new(a_b.compat());
    let b_a = Uint24LELengthPrefixedFraming::new(b_a.compat());

    let server_stream = if server_is_initiator {
        Cipher::new(
            Some(Box::new(a_b)),
            SecStream::new_initiator_xx(&[]).unwrap().into(),
        )
    } else {
        let server_key = generate_keypair().unwrap();
        Cipher::new(
            Some(Box::new(a_b)),
            SecStream::new_responder_xx(&server_key, &[])
                .unwrap()
                .into(),
        )
    };

    let client_stream = if !server_is_initiator {
        Cipher::new(
            Some(Box::new(b_a)),
            SecStream::new_initiator_xx(&[]).unwrap().into(),
        )
    } else {
        let client_key = generate_keypair().unwrap();
        Cipher::new(
            Some(Box::new(b_a)),
            SecStream::new_responder_xx(&client_key, &[])
                .unwrap()
                .into(),
        )
    };

    let server_replicator = writer_core.clone().replicate();
    let client_replicator = reader_core.clone().replicate();

    server_replicator.add_stream(server_stream).await;
    client_replicator.add_stream(client_stream).await;

    (
        (writer_core, server_replicator),
        (reader_core, client_replicator),
    )
}

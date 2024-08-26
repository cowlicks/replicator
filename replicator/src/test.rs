use hypercore_protocol::{schema::Synchronize, Duplex, Message};
use piper::pipe;
use std::time::Duration;

use hypercore::{generate_signing_key, HypercoreBuilder, PartialKeypair, Storage};

use super::*;

static PIPE_CAPACITY: usize = 1024 * 1024 * 4;

macro_rules! wait {
    () => {
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
}

async fn get_messages(repl: &Replicator) -> Vec<Message> {
    let peer = repl.peers.get(0).await;
    let out = peer
        .read()
        .await
        .message_buff
        .read()
        .await
        .iter()
        .cloned()
        .collect();
    out
}

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
) -> Result<((SharedCore, Replicator), (SharedCore, Replicator)), ReplicatorError> {
    let (reader_key, writer_key) = make_reader_and_writer_keys();
    let writer_core: SharedCore = HypercoreBuilder::new(Storage::new_memory().await.unwrap())
        .key_pair(writer_key)
        .build()
        .await
        .unwrap()
        .into();

    // add data
    writer_core
        .0
        .lock()
        .await
        .append_batch(initial_data)
        .await
        .unwrap();

    let reader_core: SharedCore = HypercoreBuilder::new(Storage::new_memory().await.unwrap())
        .key_pair(reader_key)
        .build()
        .await
        .unwrap()
        .into();

    let (read_from_writer, write_to_writer) = pipe(PIPE_CAPACITY);
    let (read_from_reader, write_to_reader) = pipe(PIPE_CAPACITY);

    let stream_to_reader = Duplex::new(read_from_writer, write_to_reader);
    let stream_to_writer = Duplex::new(read_from_reader, write_to_writer);

    let mut server_replicator = writer_core.clone().replicate();
    let mut client_replicator = reader_core.clone().replicate();

    assert_eq!(
        writer_core.key_pair().await.public,
        reader_core.key_pair().await.public,
    );

    server_replicator.add_stream(stream_to_writer, true).await?;
    client_replicator
        .add_stream(stream_to_reader, false)
        .await?;

    Ok((
        (writer_core, server_replicator),
        (reader_core, client_replicator),
    ))
}

#[tokio::test]
/// This is **not** the same as js. Both the reader and writer send an extra Sync message.
/// Seemingly as a reply to the first received one. But it works.
async fn initial_sync() -> Result<(), ReplicatorError> {
    let ((_wcore, wrep), (_rcore, rrep)) = create_connected_cores(vec![] as Vec<&[u8]>).await?;

    loop {
        if get_messages(&wrep).await.len() >= 2 && get_messages(&rrep).await.len() >= 2 {
            break;
        }
        wait!();
    }
    let sync_msg = Message::Synchronize(Synchronize {
        fork: 0,
        length: 0,
        remote_length: 0,
        downloading: true,
        uploading: true,
        can_upgrade: true,
    });
    let expected = vec![sync_msg.clone(), sync_msg.clone()];
    let msgs = get_messages(&wrep).await;
    assert_eq!(msgs, expected);
    let msgs = get_messages(&rrep).await;
    assert_eq!(msgs, expected);
    Ok(())
}

#[tokio::test]
/// works but not the same as js
async fn one_block_before_get() -> Result<(), ReplicatorError> {
    let batch: &[&[u8]] = &[b"0"];
    let ((_, writer_replicator), (reader_core, _reader_replicator)) =
        create_connected_cores(batch).await?;
    for (i, expected_block) in batch.iter().enumerate() {
        loop {
            if reader_core.info().await.length as usize > i {
                if let Some(block) = reader_core.get(i as u64).await? {
                    if block == *expected_block {
                        break;
                    }
                }
            }
            wait!();
        }
    }
    assert_eq!(reader_core.get(0).await?, Some(b"0".to_vec()));
    Ok(())
}

#[tokio::test]
async fn one_block_after_might_lock() -> Result<(), ReplicatorError> {
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(vec![] as Vec<&[u8]>).await?;
    writer_core.append(b"0").await?;
    loop {
        if let Some(block) = reader_core.get(0_u64).await? {
            assert_eq!(block, b"0");
            break;
        }
        wait!();
    }
    assert_eq!(reader_core.get(0).await?, Some(b"0".to_vec()));
    Ok(())
}

#[tokio::test]
async fn one_before_one_after_get() -> Result<(), ReplicatorError> {
    let batch: &[&[u8]] = &[&[0]];
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(batch).await?;
    writer_core.append(&[1]).await?;
    for i in 0..=1 {
        loop {
            if let Some(block) = reader_core.get(i as u64).await? {
                assert_eq!(block, vec![i as u8]);
                break;
            }
            wait!();
        }
    }
    Ok(())
}

#[tokio::test]
async fn append_many_foreach_reader_update_reader_get() -> Result<(), ReplicatorError> {
    let data: Vec<Vec<u8>> = (0..10).map(|x| vec![x as u8]).collect();
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(vec![] as Vec<&[u8]>).await?;
    for (i, val) in data.iter().enumerate() {
        // add new data to writer
        writer_core.append(val).await?;

        // wait for reader's length to update
        while (reader_core.info().await.length as usize) != i + 1 {
            wait!();
        }
        assert_eq!(reader_core.info().await.length as usize, i + 1);

        // wait for reader to `.get(i)`
        loop {
            if let Some(block) = reader_core.get(i as u64).await? {
                if block == vec![i as u8] {
                    break;
                }
            }
            wait!();
        }
    }
    Ok(())
}

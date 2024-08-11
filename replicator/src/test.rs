use async_std::task::sleep;
use hypercore_protocol::{schema::Synchronize, Duplex, Message};
use piper::pipe;
use std::time::Duration;

use hypercore::{generate_signing_key, HypercoreBuilder, PartialKeypair, Storage};

use super::*;

static PIPE_CAPACITY: usize = 1024 * 1024 * 4;

async fn pause() {
    sleep(Duration::from_millis(500)).await;
}

async fn get_messages(rep: &HcReplicator) -> Vec<Message> {
    let peer = rep.peers[0].read().await;
    let out = peer
        .message_buff
        .read()
        .await
        .iter()
        .map(|m| m.clone())
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
) -> Result<((SharedCore, HcReplicator), (SharedCore, HcReplicator)), ReplicatorError> {
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

    let _server = server_replicator.add_stream(stream_to_writer, true).await?;
    let _client = client_replicator
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
        if get_messages(&wrep).await.len() >= 2 {
            break;
        }
    }
    pause().await;
    pause().await;
    pause().await;
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

// writer:    |   reader: | no get after
//            |
// rust:      |
// * Sync     |  * Sync   |
// * Range    |  * Sync   |
// * Sync     |  * Req    |
// * Data     |  * Sync   |
// * Data     |  * Req    |
//            |
// js:        |
// * Sync     |  * Sync   |
// * Range    |  * Req    |
// * Sync     |  * Sync   |
// * Data     |  * Sync   |
// * Sync     |  * Req    |
// * Data     |  * Range  | (this data missing) | miss the ending range's
//            |  * Range  |
#[tokio::test]
/// works but not the same as js
async fn one_block_before_get() -> Result<(), ReplicatorError> {
    let batch: &[&[u8]] = &[b"0"];
    let ((_, writer_replicator), (reader_core, _reader_replicator)) =
        create_connected_cores(batch).await?;
    for i in 0..batch.len() {
        let mut j = 0;
        loop {
            if lk!(reader_core).info().length as usize >= i + 1 {
                j += 1;
                if j > 5 {
                    break;
                }
                //if let Some(block) = lk!(reader_core).get(i as u64).await? {
                //    if block == batch[i].to_vec() {
                //        break;
                //    }
                //}
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
    println!("\nWriter\n");
    let peer = &writer_replicator.peers[0].read().await;
    let msgs = peer.message_buff.read().await;
    for _m in msgs.iter() {
        //println!("\n{m:?}");
    }
    //dbg!(msgs.len());

    println!("\nReader\n");
    let peer = &_reader_replicator.peers[0].read().await;
    let msgs = peer.message_buff.read().await;
    for _m in msgs.iter() {
        //println!("\n{m:?}");
    }
    assert_eq!(reader_core.lock().await.get(0).await?, Some(b"0".to_vec()));
    Ok(())
}

#[tokio::test]
async fn one_block_after() -> Result<(), ReplicatorError> {
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(vec![] as Vec<&[u8]>).await?;
    writer_core.lock().await.append(b"0").await?;
    loop {
        let block = reader_core.lock().await.get(0_u64).await?;
        if block.is_some() {
            assert_eq!(block.unwrap(), b"0");
            break;
        }
        pause().await;
    }
    assert_eq!(reader_core.lock().await.get(0).await?, Some(b"0".to_vec()));
    Ok(())
}

#[tokio::test]
async fn one_before_one_after_get() -> Result<(), ReplicatorError> {
    let batch: &[&[u8]] = &[b"0"];
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(batch).await?;
    writer_core.lock().await.append(b"1").await?;
    loop {
        let block = reader_core.lock().await.get(0_u64).await?;
        if block.is_some() {
            assert_eq!(block.unwrap(), b"0");
            break;
        }
        pause().await;
    }
    assert_eq!(reader_core.lock().await.get(0).await?, Some(b"0".to_vec()));
    assert_eq!(reader_core.lock().await.get(1).await?, Some(b"1".to_vec()));
    Ok(())
}

#[tokio::test]
async fn append_many_foreach_reader_update_reader_get() -> Result<(), ReplicatorError> {
    let data: Vec<&[u8]> = vec![&[0], &[1], &[2], &[3]];
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(vec![] as Vec<&[u8]>).await?;
    pause().await;
    pause().await;
    pause().await;
    for i in 0..data.len() {
        // add new data to writer
        println!("QQ ------------------------- Do APPEND: {i} ---------------------");
        writer_core.lock().await.append(data[i as usize]).await?;

        // wait for reader's length to update
        println!("QQ check length i = {}", i + 1);
        while (lk!(reader_core).info().length as usize) != i + 1 {
            println!("attempt to check length");
            pause().await;
        }
        println!("QQ len updated to {}", i + 1);
        assert_eq!(reader_core.lock().await.info().length as usize, i + 1);

        // wait for reader to `.get(i)`
        loop {
            println!("QQ try get {i}");
            let block = reader_core.lock().await.get(i as u64).await?;
            if block == Some(data[i as usize].to_vec()) {
                println!("QQ get got {i}");
                break;
            }
            s += 1;
            pause().await;
        }
    }
    Ok(())
}

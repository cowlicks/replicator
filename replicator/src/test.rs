#![allow(unused_variables)]
use std::time::Duration;

use hypercore::{HypercoreBuilder, Storage};
use tracing::info;

use crate::{
    utils::{create_connected_cores, make_connected_slave},
    *,
};

const DEFAULT_MILLIS: u64 = 100;

macro_rules! wait {
    ($millis:expr) => {
        tokio::time::sleep(Duration::from_millis($millis)).await;
    };
    () => {
        wait!(DEFAULT_MILLIS)
    };
}

#[tokio::test]
/// works but not the same as js
async fn one_block_before_get() -> Result<(), ReplicatorError> {
    let batch: &[&[u8]] = &[b"0"];
    let ((_, _writer_replicator), (reader_core, _reader_replicator)) =
        create_connected_cores(batch).await;
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
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(vec![] as Vec<&[u8]>).await;
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
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(batch).await;
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
    let writer_core: ReplicatingCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .build()
        .await?
        .into();
    let reader_core = make_connected_slave(&writer_core, false).await?;

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

const MAX_LOOPS: usize = 10;
macro_rules! assert_core_get {
    ($core:tt, $block_index:expr, $expected:expr) => {
        let mut i = 0;
        loop {
            if i >= MAX_LOOPS {
                panic!("too many attempts getting data, expected: {:?}", $expected);
            }
            if let Some(x) = $core.get($block_index).await? {
                assert_eq!(x, $expected);
                break;
            }
            wait!(50);
            println!("retry core.get({})", $block_index);
            i += 1;
        }
    };
}

#[tokio::test]
/// This tests buils a star-like topology where every peer connects to the master.
/// Data is added in 3 parts: before peers, while peers are added, and after they are added.
/// Each peer is the "initiator" (so master_is_initiator == false)
/// We check that every peer can get every piece of data
/// All check appends, peer-adding, and core.get calls are done serially
async fn one_to_many_topology() -> Result<(), ReplicatorError> {
    let n_peers = 5;

    let master: ReplicatingCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .build()
        .await?
        .into();

    // add initial data to master
    let mut data = vec![];
    for i in 0..n_peers {
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    // add peers, add data each time we add peer
    let mut cores = vec![master.clone()];
    for i in n_peers..(n_peers * 2) {
        cores.push(make_connected_slave(&master, false).await?);
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    // add more data
    for i in (n_peers * 2)..(n_peers * 3) {
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    // check
    for core in cores.iter() {
        for data_i in data.iter().cloned() {
            let expected = &[data_i as u8];
            assert_core_get!(core, data_i, expected);
        }
    }

    Ok(())
}

#[tokio::test]
/// This tests buils a PATH-like topology where every peer connects to the last, in a line.
/// Data is added in 3 parts: before peers, while peers are added, and after they are added.
/// Each peer added peer is the "initiator"
/// We check that every peer can get every piece of data
/// All check appends, peer-adding, and core.get calls are done serially
async fn path_topology() -> Result<(), ReplicatorError> {
    let n_peers = 5;

    let master: ReplicatingCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .build()
        .await?
        .into();

    let mut data = vec![];
    for i in 0..n_peers {
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    let mut cores = vec![master.clone()];
    for i in 0..n_peers {
        println!("adding peer # [{i}]");
        let last = cores.last().unwrap();
        let new_peer = make_connected_slave(last, false).await?;
        cores.push(new_peer);
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    for i in 0..n_peers {
        println!("appending data # [{i}]");
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    for (peer_i, core) in cores.iter().enumerate() {
        if peer_i == 0 {
            // skip master
            continue;
        }

        println!("checking data for peer: # [{peer_i}");
        for (index, data_i) in data.iter().enumerate() {
            println!("peer # [{peer_i}] GET block #[{index}]. should have data [{data_i}]");
            let expected = &[*data_i as u8];
            assert_core_get!(core, index as u64, expected);
            println!("GOT peer # [{peer_i}] block #[{index}]. should have data [{data_i}]");
        }
    }

    Ok(())
}

#[tokio::test]
async fn path_topo_only_initial_data() -> Result<(), ReplicatorError> {
    let n_peers = 5;

    let master: ReplicatingCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .build()
        .await?
        .into();

    let mut data = vec![];
    for i in 0..n_peers {
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    let mut cores = vec![master.clone()];
    for core_i in 0..n_peers {
        let last = cores.last().unwrap();
        let new_peer = make_connected_slave(last, false).await?;
        // NB without this, later assert does not work
        for data_i in 0..n_peers {
            assert_core_get!(new_peer, data_i, &[data_i as u8]);
        }
        cores.push(new_peer);
    }

    for (core_i, core) in cores.iter().enumerate() {
        for data_i in 0..n_peers {
            assert_core_get!(core, data_i, &[data_i as u8]);
        }
    }
    Ok(())
}

#[tokio::test]
async fn after_connect_path_topo() -> Result<(), ReplicatorError> {
    let n_peers = 3;

    let master: ReplicatingCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .build()
        .await?
        .into();

    let mut data = vec![];
    let mut cores = vec![master.clone()];
    for data_i in 0..3 {
        let last = cores.last().unwrap();
        let new_peer = make_connected_slave(last, false).await?;
        cores.push(new_peer);
    }

    for i in 0..2 {
        master.append(&[i as u8]).await?;
        data.push(i);
    }

    for (peer_i, core) in cores.iter().enumerate() {
        if peer_i == 0 {
            continue;
        }
        for (index, data_val) in data.iter().enumerate() {
            info!("peer_i ={} AND index = {}", peer_i, index);
            let expected = &[*data_val as u8];
            assert_core_get!(core, index as u64, expected);
        }
    }

    Ok(())
}

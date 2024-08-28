use std::time::Duration;

use hypercore::{HypercoreBuilder, Storage};

use crate::{
    utils::{create_connected_cores, create_connected_streams, writer_and_reader_cores},
    *,
};

macro_rules! wait {
    () => {
        tokio::time::sleep(Duration::from_millis(25)).await;
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
    let ((writer_core, _), (reader_core, _)) = create_connected_cores(vec![] as Vec<&[u8]>).await;
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

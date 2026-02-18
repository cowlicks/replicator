use std::time::Duration;

use futures::StreamExt;
use hypercore::replication::{CoreInfo, CoreMethods, SharedCore};
use hypercore::{HypercoreBuilder, PartialKeypair, Storage};
use hyperswarm::{DhtConfig, JoinOpts, Swarm, Topic};
use replicator::Replicate;
use test_utils::{Result, Testnet};

macro_rules! wait {
    ($millis:expr) => {
        tokio::time::sleep(Duration::from_millis($millis)).await
    };
}

macro_rules! timeout {
    ($fut:expr, $ms:expr) => {
        tokio::time::timeout(Duration::from_millis($ms), $fut).await
    };
}

/// Two Rust peers discover each other over HyperDHT and replicate a hypercore.
///
/// The writer (server) appends a block before connecting; the reader (client)
/// polls until it can read back the same block through the replicator.
#[tokio::test]
async fn rust_to_rust_replication_through_hyperswarm() -> Result<()> {
    let mut tn = Testnet::new().await?;
    let bs_addr = tn.bootstrap_addr().await?;

    let topic = Topic::random();

    // --- Set up hypercores ---
    let writer_core: SharedCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .build()
        .await?
        .into();

    // Append a block before any connection is made
    writer_core.append(b"hello from hyperswarm").await?;

    // Reader uses the same public key but no secret (read-only)
    let writer_keypair = writer_core.key_pair().await;
    let reader_keypair = PartialKeypair {
        public: writer_keypair.public,
        secret: None,
    };
    let reader_core: SharedCore = HypercoreBuilder::new(Storage::new_memory().await?)
        .key_pair(reader_keypair)
        .build()
        .await?
        .into();

    let writer_replicator = writer_core.clone().replicate();
    let reader_replicator = reader_core.clone().replicate();

    // --- Set up swarms ---
    let swarm_server = Swarm::new(DhtConfig::default().add_bootstrap_node(bs_addr)).await?;
    let swarm_client = Swarm::new(DhtConfig::default().add_bootstrap_node(bs_addr)).await?;

    swarm_server.bootstrap().await?;
    swarm_client.bootstrap().await?;

    // Server announces, client discovers
    let mut server_conns = swarm_server.connections();
    swarm_server.join(topic, JoinOpts::Server);
    swarm_server.flush().await?;

    let mut client_conns = swarm_client.connections();
    swarm_client.join(topic, JoinOpts::Client);
    swarm_client.flush().await?;

    // --- Wire connections to replicators ---
    //
    // The hypercore protocol initiator is the one who opens the channel first.
    // The hyperswarm server-side connection has conn_event.client == false, so
    let server_task = tokio::spawn(async move {
        let conn_event = timeout!(server_conns.next(), 5000)
            .expect("timed out waiting for server connection")
            .expect("server connection stream ended")
            .expect("server connection error");

        writer_replicator.add_stream(conn_event.connection).await;

        // Keep the task alive long enough for replication to complete
        wait!(1000);
    });

    let client_task = tokio::spawn(async move {
        let conn_event = timeout!(client_conns.next(), 5000)
            .expect("timed out waiting for client connection")
            .expect("client connection stream ended")
            .expect("client connection error");

        reader_replicator.add_stream(conn_event.connection).await;

        // Poll until the block arrives
        loop {
            if let Some(block) = reader_core.get(0).await.unwrap() {
                assert_eq!(block, b"hello from hyperswarm");
                break;
            }
            wait!(100);
        }
    });

    let (server_res, client_res) = tokio::join!(server_task, client_task);
    server_res?;
    client_res?;

    Ok(())
}

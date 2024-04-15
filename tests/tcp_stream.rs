use async_std::{
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    task::spawn,
};
use futures_lite::{FutureExt, StreamExt};
use hypercore::{generate_signing_key, Hypercore, HypercoreBuilder, PartialKeypair, Storage};
use replicator::{r, HcTraits, Replicator};
use std::sync::OnceLock;

static PORT: &str = "6776";
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

fn key_pair() -> &'static PartialKeypair {
    static KEY_PAIR: OnceLock<PartialKeypair> = OnceLock::new();
    KEY_PAIR.get_or_init(|| {
        let signing_key = generate_signing_key();
        PartialKeypair {
            public: signing_key.verifying_key(),
            secret: Some(signing_key),
        }
    })
}

async fn run_server<T: HcTraits + 'static>(core: Arc<Mutex<Hypercore<T>>>, listener: TcpListener) {
    let mut incoming = listener.incoming();
    if let Some(Ok(stream)) = incoming.next().await {
        let peer_addr = stream.peer_addr().unwrap();
        println!("SERVER got stream from CLIENT: {peer_addr}");
        core.replicate(stream, false).await.unwrap();
        println!("SERVER repl DONE");
    }
}

async fn run_client() {
    println!("start CLIENT");
    let mut key_pair = key_pair().clone();
    key_pair.secret = None;
    let address = format!("127.0.0.1:{PORT}");
    let core = Arc::new(Mutex::new(
        HypercoreBuilder::new(Storage::new_memory().await.unwrap())
            .key_pair(key_pair)
            .build()
            .await
            .unwrap(),
    ));

    println!("CLIENT initiating connection with SERVER");
    let stream = TcpStream::connect(&address).await.unwrap();
    println!("run repl CLIENT");
    core.replicate(stream, true).await.unwrap();
    println!("CLIENT repl DONE");
}

#[tokio::test]
async fn run_client_server() -> Result<()> {
    let key_pair = key_pair().clone();
    let address = format!("127.0.0.1:{PORT}");
    let hypercore = Arc::new(Mutex::new(
        HypercoreBuilder::new(Storage::new_memory().await.unwrap())
            .key_pair(key_pair)
            .build()
            .await
            .unwrap(),
    ));

    // add data
    let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];
    r!(hypercore).append_batch(batch).await.unwrap();

    println!("start SERVER");
    println!("start listening on SERVER");
    let listener = TcpListener::bind(&address).await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());

    let server = spawn(run_server(hypercore, listener));

    let client = spawn(run_client());
    server.race(client).await;
    Ok(())
}

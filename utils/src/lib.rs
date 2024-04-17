use async_std::sync::{Arc, Mutex};
use hypercore::{generate_signing_key, Hypercore, HypercoreBuilder, PartialKeypair, Storage};
use random_access_memory::RandomAccessMemory;

pub use replicator::HcTraits;

pub type SharedCore<T> = Arc<Mutex<Hypercore<T>>>;

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

pub async fn ram_core(key: Option<PartialKeypair>) -> SharedCore<RandomAccessMemory> {
    Arc::new(Mutex::new(
        HypercoreBuilder::new(Storage::new_memory().await.unwrap())
            //.key_pair(writer_key)
            .build()
            .await
            .unwrap(),
    ))
}

static INIT_LOG: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
pub async fn setup_logs() {
    INIT_LOG
        .get_or_init(|| async {
            tracing_subscriber::fmt::fmt()
                .event_format(
                    tracing_subscriber::fmt::format()
                        .without_time()
                        .with_file(true)
                        .with_line_number(true),
                )
                .init();
        })
        .await;
}

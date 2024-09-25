use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

use hypercore::{
    generate_signing_key, replication::SharedCore, HypercoreBuilder, PartialKeypair, Storage,
};

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

pub async fn ram_core(key: Option<&PartialKeypair>) -> SharedCore {
    let builder = HypercoreBuilder::new(Storage::new_memory().await.unwrap());
    let builder = match key {
        Some(key) => builder.key_pair(key.clone()),
        None => builder,
    };
    builder.build().await.unwrap().into()
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

use tracing_subscriber::EnvFilter;
pub fn init_env_logs() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()) // Reads `RUST_LOG` environment variable
        .init();
}

/// Seedable deterministic pseudorandom number generator used for reproducible randomized testing
pub struct Rand {
    seed: u64,
    counter: AtomicU64,
    sin_scale: f64,
    ordering: Ordering,
}

impl Rand {
    pub fn rand(&self) -> f64 {
        let count = self.counter.fetch_add(1, self.ordering);
        let x = ((self.seed + count) as f64).sin() * self.sin_scale;
        x - x.floor()
    }
    pub fn rand_int_lt(&self, max: u64) -> u64 {
        (self.rand() * (max as f64)).floor() as u64
    }
    pub fn shuffle<T>(&self, mut arr: Vec<T>) -> Vec<T> {
        let mut out = vec![];
        while !arr.is_empty() {
            let i = self.rand_int_lt(arr.len() as u64) as usize;
            out.push(arr.remove(i));
        }
        out
    }
}

impl Default for Rand {
    fn default() -> Self {
        Self {
            seed: 42,
            counter: Default::default(),
            sin_scale: 10_000_f64,
            ordering: Ordering::SeqCst,
        }
    }
}

fn log() {
    static START_LOGS: OnceLock<()> = OnceLock::new();
    START_LOGS.get_or_init(|| {
        tracing_subscriber::fmt()
            .with_line_number(true)
            .without_time()
            // Reads `RUST_LOG` environment variable
            .with_env_filter(EnvFilter::from_default_env())
            .init();
    });
}

fn exit_on_thread_panic() {
    static EXIT_ON_THREAD_PANIC: OnceLock<()> = OnceLock::new();
    EXIT_ON_THREAD_PANIC.get_or_init(|| {
        // take_hook() returns the default hook in case when a custom one is not set
        let orig_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            // invoke the default handler and exit the process
            orig_hook(panic_info);
            std::process::exit(1);
        }));
    });
}

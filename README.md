# Hypercore Replicator
# ⚠️  WARNING 🚧 API unstable ⚒️  and still in development 👷


This repo defines a `Replicator` trait for use with [`Hypercore`](#todo).

Usage (aspirational):
```rust

// import the trait
use replicator::Replicate;

// Get a hypercore
let hypercore = HypercoreBuilder::new(Storage::new_memory().await.unwrap())
    .build()
    .await
    .unwrap();

// Get a AsyncRead + AsyncWrite stream to another hypercore
let listener = TcpListener::bind(&address).await?;

// the `true` indicates this hypercore is the initiator
hypercore.replicate(stream, true))
```

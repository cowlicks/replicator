# DEPRECATED

This crate is no longer developed. Replication now lives in the
[`hypercore`](https://crates.io/crates/hypercore) crate itself, behind its
`replication` feature (on by default). Use that instead.

This repo was an experiment in implementing
[JavaScript Hypercore's replication](https://github.com/holepunchto/hypercore/blob/3fda699f306fa3f4781ad66ea13ea0df108a48cd/lib/replicator.js)
as a separate `Replicator` trait for Rust's `Hypercore`. That work has been moved into
`hypercore`, where it can reach the core's internals directly. Nothing here is maintained,
and nothing depends on it.

# Hypercore Replicator

This repo defines a `Replicator` trait for use with [`Hypercore`](https://github.com/holepunchto/hypercore). It implements the same functionality as this [JavaScript Hypercore replication code](https://github.com/holepunchto/hypercore/blob/3fda699f306fa3f4781ad66ea13ea0df108a48cd/lib/replicator.js).

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

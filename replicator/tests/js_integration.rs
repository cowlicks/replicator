mod common;

use std::time::Duration;

use hypercore::{
    PartialKeypair, VerifyingKey,
    replication::{CoreInfo, CoreMethods},
};
use hypercore_protocol::HandshakeResult;
use macros::start_func_with;
use tokio::{net::TcpListener, spawn};
use tokio_util::compat::TokioAsyncReadCompatExt;
use uint24le_framing::Uint24LELengthPrefixedFraming;
use utils::ram_core;

use rusty_nodejs_repl::{Config, Repl};

use common::{
    LOOPBACK, Result,
    js::{path_to_node_modules, require_js_data},
    serialize_public_key,
};
use replicator::{ReplicatingCore, utils::make_reader_and_writer_keys};

async fn rust_writer_js_reader<A: AsRef<[u8]>, B: AsRef<[A]>>(
    batch: B,
) -> Result<(ReplicatingCore, Repl)> {
    let (rkey, wkey) = make_reader_and_writer_keys();
    let core: ReplicatingCore = ram_core(Some(&wkey)).await.into();

    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = format!("{}", listener.local_addr()?.port());
    let hostname = LOOPBACK;

    let replicating_core = core.clone();
    spawn(async move {
        let bytes = listener.accept().await?.0;
        let stream = Uint24LELengthPrefixedFraming::new(bytes.compat());
        let (stream, _errors) = stream.split_off_errors();
        let hs2 = HandshakeResult::from_pre_encrypted(false, [2u8; 32], [3u8; 32], vec![]);
        let fut = replicating_core.add_stream(stream, false, hs2);
        fut.await;
        Ok::<(), std::io::Error>(())
    });

    for b in batch.as_ref().iter() {
        core.append(b.as_ref()).await?;
    }

    let mut conf = Config::build()?;
    conf.imports.push(
        "
RAM = require('random-access-memory');
Hypercore = require('hypercore');
net = require('net');
"
        .into(),
    );

    conf.before.push(format!(
        "
key = '{}';
core = new Hypercore(RAM, key);
await core.ready();
",
        serialize_public_key(&rkey)
    ));
    conf.after.push("await core.close()".into());

    conf.before.push(format!(
        "
socket = net.connect('{port}', '{hostname}');
socket.pipe(core.replicate(true)).pipe(socket);
await core.update({{wait: true}});
"
    ));
    conf.after.push("socket.destroy();".into());
    conf.path_to_node_modules = Some(path_to_node_modules()?.display().to_string());

    Ok((core, conf.start().await?))
}

async fn setup_js_writer_rust_reader() -> Result<(ReplicatingCore, Repl)> {
    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;

    let mut conf = Config::build()?;
    conf.imports.push(
        "
RAM = require('random-access-memory');
Hypercore = require('hypercore');
net = require('net');
"
        .into(),
    );

    conf.before.push(
        "
core = new Hypercore(RAM);
"
        .into(),
    );
    conf.after.push("await core.close()".into());

    conf.before.push(format!(
        "
socket = net.connect('{}', '{LOOPBACK}');
socket.pipe(core.replicate(false)).pipe(socket);
",
        listener.local_addr()?.port()
    ));
    conf.after.push("socket.destroy();".into());
    conf.path_to_node_modules = Some(path_to_node_modules()?.display().to_string());

    let mut repl = conf.start().await?;

    // get the public key from the JS core for the RS core
    let key = repl.run("output(core.key.toString('hex'))").await?;

    let key: [u8; 32] = data_encoding::HEXLOWER
        .decode(&key)?
        .as_slice()
        .try_into()?;

    let core: ReplicatingCore = ram_core(Some(&PartialKeypair {
        public: VerifyingKey::from_bytes(&key)?,
        secret: None,
    }))
    .await
    .into();

    let replicating_core = core.clone();
    spawn(async move {
        let bytes = listener.accept().await?.0;
        let stream = Uint24LELengthPrefixedFraming::new(bytes.compat());
        let (stream, _errors) = stream.split_off_errors();
        let hs2 = HandshakeResult::from_pre_encrypted(false, [2u8; 32], [3u8; 32], vec![]);
        let fut = replicating_core.add_stream(stream, true, hs2);
        fut.await;
        Ok::<(), std::io::Error>(())
    });

    repl.run("await core.ready();").await?;
    Ok((core, repl))
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn initial_rust_data_replicates_to_js() -> Result<()> {
    let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];

    let (_core, mut repl) = rust_writer_js_reader(batch).await?;
    // print the length of the core so we can check it in rust
    let result = repl
        .run("output(String((await core.info()).length));")
        .await?;

    dbg!();
    let result = repl
        .run(
            "
            await core.update({wait: true});
            output(String((await core.info()).length));
            ",
        )
        .await?;

    let expected: Vec<u8> = format!("{}", batch.len()).bytes().collect();
    assert_eq!(result, expected);

    for (i, val) in batch.iter().enumerate() {
        let code = format!("output(String((await core.get({i}))));");
        let stdout = repl.run(&code).await?;
        assert_eq!(stdout, *val);
    }

    // stop the repl. When repl is stopped hypercore & socket are closed
    let _ = repl.stop().await?;
    Ok(())
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn added_rust_data_replicates_to_js() -> Result<()> {
    let initial_datas = [b"a", b"b", b"c"];
    let (core, mut repl) = rust_writer_js_reader(&initial_datas).await?;

    let _res = repl
        .run(
            "
await core.update({wait: true});
await new Promise(r => setTimeout(r, 1e2))
",
        )
        .await?;
    let stdout = repl
        .run("output(String((await core.info()).length));")
        .await?;
    assert_eq!(stdout, b"3");

    for (i, l) in "defghijklmnopqrstuvwxyz".bytes().enumerate() {
        core.append(&[l]).await?;
        let new_size = initial_datas.len() + i;
        let stdout = repl
            .run(
                "
await core.update({wait: true});
await new Promise(r => setTimeout(r, 1e2))
output(String((await core.info()).length));",
            )
            .await?;
        let len: Vec<u8> = format!("{}", new_size + 1).bytes().collect();
        assert_eq!(stdout, len);

        let code = format!("output(String((await core.get({new_size}))));");
        let stdout = repl.run(&code).await?;
        assert_eq!(stdout, &[l]);
    }

    // stop the repl. When repl is stopped hypercore & socket are closed
    let _ = repl.stop().await?;
    Ok(())
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn js_writer_replicates_to_rust_reader() -> Result<()> {
    let (core, mut repl) = setup_js_writer_rust_reader().await?;

    for i in 0..10 {
        repl.run(&format!("await core.append(Buffer.from([{i}]));"))
            .await?;
        loop {
            // this does not work without thin check to info..
            if let Some(x) = core.get(i).await? {
                assert_eq!(x, vec![i as u8]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    Ok(())
}

// TODO finish this to replace other replication setup code
#[allow(unused)]
async fn make_js_slave_from_rs(
    core: ReplicatingCore,
    js_core_name: &str,
    repl: &mut Repl,
) -> Result<()> {
    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = format!("{}", listener.local_addr()?.port());
    let hostname = LOOPBACK;

    let replicating_core = core.clone();
    spawn(async move {
        let bytes = listener.accept().await?.0;
        let stream = Uint24LELengthPrefixedFraming::new(bytes.compat());
        let (stream, _errors) = stream.split_off_errors();
        let hs2 = HandshakeResult::from_pre_encrypted(false, [2u8; 32], [3u8; 32], vec![]);
        let fut = replicating_core.add_stream(stream, false, hs2);
        fut.await;
        Ok::<(), std::io::Error>(())
    });
    let k = core.key_pair().await;

    repl.run(&format!(
        "
{js_core_name} = (async () => {{
    const key = '{}';
    const core = new Hypercore(RAM, key);
    await core.ready();
    const socket = net.connect('{port}', '{hostname}');
    socket.pipe(core.replicate(true)).pipe(socket);
    await core.update({{wait: true}});
    return core;
}})();
",
        serialize_public_key(&k),
    ))
    .await?;
    Ok(())
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn new_js_slave() -> Result<()> {
    let (core, mut repl) = setup_js_writer_rust_reader().await?;

    for i in 0..10 {
        repl.run(&format!("await core.append(Buffer.from([{i}]));"))
            .await?;
        loop {
            // this does not work without thin check to info..
            if let Some(x) = core.get(i).await? {
                assert_eq!(x, vec![i as u8]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    Ok(())
}

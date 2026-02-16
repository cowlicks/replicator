mod common;

use std::time::Duration;

use hypercore::{
    PartialKeypair, VerifyingKey,
    replication::{CoreInfo, CoreMethods},
};
use hypercore_handshake::{Cipher, state_machine::hc_specific};
use macros::start_func_with;
use tokio::{net::TcpListener, spawn};
use tokio_util::compat::TokioAsyncReadCompatExt;
use uint24le_framing::Uint24LELengthPrefixedFraming;
use utils::{
    // log,
    ram_core,
};

use rusty_nodejs_repl::{Config, Repl, integration_utils::log};

use common::{
    LOOPBACK, Result,
    js::{path_to_node_modules, require_js_data},
    serialize_public_key,
};
use replicator::{ReplicatingCore, utils::make_reader_and_writer_keys};

fn js_imports() -> String {
    "
RAM = require('random-access-memory');
Hypercore = require('hypercore');
NoiseSecretStream = require('@hyperswarm/secret-stream');
Protomux = require('protomux');
net = require('net');
"
    .into()
}

async fn setup_js_writer_rust_reader() -> Result<(ReplicatingCore, Repl)> {
    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = listener.local_addr()?.port();

    // Generate Noise keypair for the Rust side (responder)
    let noise_kp = hc_specific::generate_keypair()?;
    let noise_pub_hex = data_encoding::HEXLOWER.encode(&noise_kp.public);

    let mut conf = Config::build()?;
    conf.imports.push(js_imports());

    conf.before.push(
        "
core = new Hypercore(RAM);
await core.ready();
"
        .into(),
    );
    conf.after.push("await core.close()".into());

    // JS is initiator with IK pattern, connecting to Rust responder
    conf.before.push(format!(
        "
socket = net.connect('{port}', '{LOOPBACK}');
const noiseStream = new NoiseSecretStream(true, socket, {{
    pattern: 'IK',
    remotePublicKey: Buffer.from('{noise_pub_hex}', 'hex')
}});
const mux = new Protomux(noiseStream);
core.replicate(mux);
"
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
        let tcp = listener.accept().await?.0;
        let framed = Uint24LELengthPrefixedFraming::new(tcp.compat());

        let cipher = Cipher::resp_from_private(Some(Box::new(framed)), &noise_kp)?;

        replicating_core.add_stream(cipher, true).await;
        Ok::<(), std::io::Error>(())
    });

    Ok((core, repl))
}

async fn rust_writer_js_reader<A: AsRef<[u8]>, B: AsRef<[A]>>(
    batch: B,
) -> Result<(ReplicatingCore, Repl)> {
    let (rkey, wkey) = make_reader_and_writer_keys();
    let core: ReplicatingCore = ram_core(Some(&wkey)).await.into();

    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = listener.local_addr()?.port();
    let hostname = LOOPBACK;

    // Generate Noise keypair for the Rust side (responder)
    let noise_kp = hc_specific::generate_keypair()?;
    let noise_pub_hex = data_encoding::HEXLOWER.encode(&noise_kp.public);

    let replicating_core = core.clone();
    //let noise_public: [u8; 32] = noise_kp.public.clone().try_into().unwrap();
    spawn(async move {
        let tcp = listener.accept().await?.0;
        let framed = Uint24LELengthPrefixedFraming::new(tcp.compat());

        let cipher = Cipher::resp_from_private(Some(Box::new(framed)), &noise_kp)?;

        replicating_core.add_stream(cipher, false).await;
        Ok::<(), std::io::Error>(())
    });

    for b in batch.as_ref().iter() {
        core.append(b.as_ref()).await?;
    }

    let mut conf = Config::build()?;
    conf.imports.push(js_imports());

    conf.before.push(format!(
        "
key = '{}';
core = new Hypercore(RAM, key);
await core.ready();
",
        serialize_public_key(&rkey)
    ));
    conf.after.push("await core.close()".into());

    // JS is initiator with IK pattern
    conf.before.push(format!(
        "
socket = net.connect('{port}', '{hostname}');
const noiseStream = new NoiseSecretStream(true, socket, {{
    pattern: 'IK',
    remotePublicKey: Buffer.from('{noise_pub_hex}', 'hex')
}});
const mux = new Protomux(noiseStream);
core.replicate(mux);
"
    ));
    conf.after.push("socket.destroy();".into());
    conf.path_to_node_modules = Some(path_to_node_modules()?.display().to_string());

    Ok((core, conf.start().await?))
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn initial_rust_data_replicates_to_js() -> Result<()> {
    log();
    let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];

    let (_core, mut repl) = rust_writer_js_reader(batch).await?;
    dbg!();

    let result = repl
        .run(
            "
await core.update({wait: true});
output(String((await core.info()).length));
",
        )
        .await?;

    println!("js core length {}", String::from_utf8_lossy(&result));
    println!("rs core length {}", _core.info().await.length);
    let expected: Vec<u8> = format!("{}", batch.len()).bytes().collect();
    assert_eq!(result, expected);

    for (i, val) in batch.iter().enumerate() {
        let code = format!("output(String((await core.get({i}))));");
        let result = repl.run(&code).await?;
        assert_eq!(result, *val);
    }

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
            if let Some(x) = core.get(i).await? {
                assert_eq!(x, vec![i as u8]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
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
            if let Some(x) = core.get(i).await? {
                assert_eq!(x, vec![i as u8]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    Ok(())
}

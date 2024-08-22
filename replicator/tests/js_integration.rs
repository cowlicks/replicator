mod common;

use async_std::net::TcpListener;
use common::{js::path_to_node_modules, run_replicate, Result};
use macros::start_func_with;
use utils::{make_reader_and_writer_keys, ram_core, SharedCore};

use rusty_nodejs_repl::{JsContext, ReplConfBuilder};

use crate::common::{js::require_js_data, serialize_public_key, LOOPBACK};

async fn rust_writer_js_reader<A: AsRef<[u8]>, B: AsRef<[A]>>(
    batch: B,
) -> Result<(SharedCore, JsContext)> {
    let (rkey, wkey) = make_reader_and_writer_keys();
    let core = ram_core(Some(&wkey)).await;
    let server_core = core.clone();

    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = format!("{}", listener.local_addr()?.port());
    let hostname = LOOPBACK;

    let _server =
        async_std::task::spawn(async move { run_replicate(listener, server_core).await.unwrap() });
    core.lock().await.append_batch(batch).await?;

    let mut repl = ReplConfBuilder::default().build()?;
    repl.imports.push(
        "
RAM = require('random-access-memory');
Hypercore = require('hypercore');
net = require('net');
"
        .into(),
    );

    repl.before.push(format!(
        "
key = '{}';
core = new Hypercore(RAM, key);
await core.ready();
",
        serialize_public_key(&rkey)
    ));
    repl.after.push("await core.close()".into());

    repl.before.push(format!(
        "
socket = net.connect('{port}', '{hostname}');
socket.pipe(core.replicate(true)).pipe(socket);
await core.update({{wait: true}});
"
    ));
    repl.after.push("socket.destroy();".into());
    repl.path_to_node_modules = Some(path_to_node_modules()?.display().to_string());

    Ok((core, repl.start()?))
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn initial_data_rs_data_replicates_to_js() -> Result<()> {
    let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];

    let (_core, mut context) = rust_writer_js_reader(batch).await?;
    // print the length of the core so we can check it in rust
    //flush_stdout!(context);
    let result = context
        .repl("process.stdout.write(String((await core.info()).length));")
        .await?;

    let expected: Vec<u8> = format!("{}", batch.len()).bytes().collect();
    assert_eq!(result, expected);

    for (i, val) in batch.iter().enumerate() {
        let code = format!("process.stdout.write(String((await core.get({i}))));");
        let stdout = context.repl(&code).await?;
        assert_eq!(stdout, *val);
    }

    // stop the repl. When repl is stopped hypercore & socket are closed
    let _ = context.repl("queue.done();").await?;

    // ensure js process closes properly
    assert_eq!(context.child.output().await?.status.code(), Some(0));
    Ok(())
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn added_data_replicates_to_js() -> Result<()> {
    let initial_datas = [b"a", b"b", b"c"];
    let (core, mut context) = rust_writer_js_reader(&initial_datas).await?;

    let _res = context
        .repl(
            "
await core.update({wait: true});
await new Promise(r => setTimeout(r, 1e2))
",
        )
        .await?;
    let stdout = context
        .repl("process.stdout.write(String((await core.info()).length));")
        .await?;
    assert_eq!(stdout, b"3");

    for (i, l) in "defghijklmnopqrstuvwxyz".bytes().enumerate() {
        core.lock().await.append(&[l]).await?;
        let new_size = initial_datas.len() + i;
        let stdout = context
            .repl(
                "
await core.update({wait: true});
await new Promise(r => setTimeout(r, 1e2))
process.stdout.write(String((await core.info()).length));",
            )
            .await?;
        let len: Vec<u8> = format!("{}", new_size + 1).bytes().collect();
        assert_eq!(stdout, len);

        let code = format!("process.stdout.write(String((await core.get({new_size}))));");
        let stdout = context.repl(&code).await?;
        assert_eq!(stdout, &[l]);
    }

    // stop the repl. When repl is stopped hypercore & socket are closed
    let _ = context.repl("queue.done();").await?;
    let _ = context.child.output().await?;
    Ok(())
}

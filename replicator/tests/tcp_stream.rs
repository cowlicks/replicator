mod common;

use async_std::net::TcpListener;
use common::{
    js::{connect_and_teardown_js_core, run_hypercore_js, JsContext},
    run_replicate, Result,
};
use futures_lite::AsyncWriteExt;
use macros::start_func_with;
use random_access_memory::RandomAccessMemory;
use utils::{make_reader_and_writer_keys, ram_core, SharedCore};

use crate::common::{
    js::{
        async_iiaf_template, flush_stdout, path_to_js_dir, repl, require_js_data,
        run_async_js_block, run_js, RUN_REPL_CODE,
    },
    serialize_public_key, LOOPBACK,
};

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn initial_data_rs_data_replicates_to_js() -> Result<()> {
    let (rkey, wkey) = make_reader_and_writer_keys();

    // create the writer core in rust
    let core = ram_core(Some(&wkey)).await;
    let server_core = core.clone();

    // add some data to the core
    let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];
    core.lock().await.append_batch(batch).await?;

    // run replication in the background
    // NB: the :0 makes it choose a random port
    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = format!("{}", listener.local_addr()?.port());
    let _server =
        async_std::task::spawn(async move { run_replicate(listener, server_core).await.unwrap() });

    // create a reader core in javascript
    let rkey_hex = serialize_public_key(&rkey);
    let mut context = run_hypercore_js(
        Some(&rkey_hex),
        &connect_and_teardown_js_core(&port, LOOPBACK, RUN_REPL_CODE),
        vec![
            format!("{}/utils.js", path_to_js_dir()?.to_string_lossy()),
            format!("{}/repl.js", path_to_js_dir()?.to_string_lossy()),
        ],
    )?;
    // print the length of the core so we can check it in rust
    flush_stdout!(context);
    let result = repl!(
        context,
        "process.stdout.write(String((await core.info()).length));"
    );
    // assert the js core has 4 blocks
    assert_eq!(result, b"4");

    // stop the repl. When repl is stopped hypercore & socket are closed
    let _ = repl!(context, "queue.done();");

    // ensure js process closes properly
    assert_eq!(context.child.output().await?.status.code(), Some(0));
    Ok(())
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn read_eval_print_macro_works() -> Result<()> {
    let mut context = run_js(
        &async_iiaf_template(RUN_REPL_CODE),
        vec![
            format!("{}/utils.js", path_to_js_dir()?.to_string_lossy()),
            format!("{}/repl.js", path_to_js_dir()?.to_string_lossy()),
        ],
    )?;

    let result = repl!(context, "process.stdout.write('fooo6!');");
    assert_eq!(result, b"fooo6!");
    let result = repl!(
        context,
        "
a = 66;
b = 7 + a;
process.stdout.write(`${{b}}`);
"
    );
    assert_eq!(result, b"73");

    let _result = repl!(context, "queue.done();");
    let _ = context.child.output().await?;
    Ok(())
}

async fn setup_rs_writer_js_reader<A: AsRef<[u8]>, B: AsRef<[A]>>(
    batch: B,
) -> Result<(SharedCore<RandomAccessMemory>, JsContext)> {
    let (rkey, wkey) = make_reader_and_writer_keys();

    // create the writer core in rust
    let core = ram_core(Some(&wkey)).await;
    let server_core = core.clone();

    // run replication in the background
    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = format!("{}", listener.local_addr()?.port());

    let _server =
        async_std::task::spawn(async move { run_replicate(listener, server_core).await.unwrap() });
    core.lock().await.append_batch(batch).await?;

    // create a reader core in javascript
    let rkey_hex = serialize_public_key(&rkey);
    let context = run_hypercore_js(
        Some(&rkey_hex),
        &connect_and_teardown_js_core(&port, LOOPBACK, RUN_REPL_CODE),
        vec![
            format!("{}/utils.js", path_to_js_dir()?.to_string_lossy()),
            format!("{}/repl.js", path_to_js_dir()?.to_string_lossy()),
        ],
    )?;
    Ok((core, context))
}

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn added_data_replicates_to_js() -> Result<()> {
    let (core, mut context) = setup_rs_writer_js_reader([b"a", b"b", b"c"]).await?;

    let _ = repl!(
        context,
        "
        await core.update({{wait: true}});
await new Promise(r => setTimeout(r, 1e2))
"
    );
    let result = repl!(
        context,
        "process.stdout.write(String((await core.info()).length));"
    );
    assert_eq!(result, b"3");

    core.lock().await.append(b"1").await?;
    let _ = repl!(
        context,
        "
await core.update({{wait: true}});
await new Promise(r => setTimeout(r, 1e2))
"
    );
    let result = repl!(
        context,
        "process.stdout.write(String((await core.info()).length));"
    );
    assert_eq!(result, b"4");

    // stop the repl. When repl is stopped hypercore & socket are closed
    let _ = repl!(context, "queue.done();");
    let _ = context.child.output().await?;
    Ok(())
}

#[tokio::test]
async fn events() -> Result<()> {
    let (_rkey, wkey) = make_reader_and_writer_keys();

    // create the writer core in rust
    let core = ram_core(Some(&wkey)).await;
    let mut rec = core.lock().await.onupgrade();
    core.lock().await.append(b"foo").await?;
    let Ok(_) = rec.recv().await else {
        panic!("Colud not get event");
    };
    Ok(())
}
// 1800558 8321

mod common;

use async_process::ChildStdout;
use async_std::net::TcpListener;
use common::{
    js::{connect_and_teardown_js_core, run_hypercore_js, JsContext},
    run_server, Result,
};
use futures_lite::{io::Bytes, AsyncWriteExt, StreamExt};
use macros::start_func_with;
use random_access_memory::RandomAccessMemory;
use utils::{make_reader_and_writer_keys, ram_core, HcTraits, SharedCore};

use crate::common::{
    js::{
        async_iiaf_template, path_to_js_dir, repl, require_js_data, run_async_js_block, run_js,
        RUN_REPL_CODE,
    },
    serialize_public_key, LOOPBACK,
};

#[start_func_with(require_js_data()?;)]
#[tokio::test]
async fn rs_server_js_client_initial_data_moves() -> Result<()> {
    let (rkey, wkey) = make_reader_and_writer_keys();

    // create the writer core in rust
    let core = ram_core(Some(&wkey)).await;
    let server_core = core.clone();

    // add some data to the core
    let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];
    core.lock().await.append_batch(batch).await?;

    // run replication in the background
    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = format!("{}", listener.local_addr()?.port());
    let _server =
        async_std::task::spawn(async move { run_server(listener, server_core).await.unwrap() });

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
    println!(
        "{}",
        String::from_utf8_lossy(&context.child.output().await?.stderr)
    );
    Ok(())
}

async fn setup_rs_writer_js_reader() -> Result<(SharedCore<RandomAccessMemory>, JsContext)> {
    let (rkey, wkey) = make_reader_and_writer_keys();

    // create the writer core in rust
    let core = ram_core(Some(&wkey)).await;
    let server_core = core.clone();

    // run replication in the background
    let listener = TcpListener::bind(format!("{}:0", LOOPBACK)).await?;
    let port = format!("{}", listener.local_addr()?.port());

    let _server =
        async_std::task::spawn(async move { run_server(listener, server_core).await.unwrap() });

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
async fn new_data_moved_to_js() -> Result<()> {
    let (core, mut context) = setup_rs_writer_js_reader().await?;

    let result = repl!(context, "write(String((await core.info()).length))");
    assert_eq!(result, b"0");

    // add code to core here
    core.lock().await.append(b"hello").await?;
    let result = repl!(
        context,
        "
await core.update({{wait: true}});
process.stdout.write(String((await core.info()).length));
                       "
    );
    assert_eq!(result, b"1");
    let _ = repl!(context, "queue.done();");
    println!(
        "{}",
        String::from_utf8_lossy(&context.child.output().await?.stderr)
    );
    // stop the repl. When repl is stopped hypercore & socket are closed

    // ensure js process closes properly
    Ok(())
}

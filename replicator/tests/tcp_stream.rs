mod common;

use async_process::ChildStdout;
use common::{js::run_hypercore_js, run_server, Result};
use futures_lite::{io::Bytes, AsyncReadExt, AsyncWriteExt, StreamExt};
use utils::{make_reader_and_writer_keys, ram_core};

use crate::common::{
    js::{path_to_js_dir, run_js_async_block},
    serialize_public_key, HOSTNAME, PORT,
};

static START_ASYNC_BLOCK: &[u8] = b";(async () => {\n";
static END_ASYNC_BLOCK: &[u8] = b"})();";

static START_DELIM_MSG: &[u8] = b"; process.stdout.write('";
static END_DELIM_MSG: &[u8] = b"');";

macro_rules! js_code {
    ($stdin:expr, $eof:expr, $($arg:tt)*) => {{
        let block = format!($($arg)*);
        let code = [
            START_ASYNC_BLOCK,
            block.as_bytes(),
            START_DELIM_MSG,
            $eof,
            END_DELIM_MSG,
            END_ASYNC_BLOCK,
        ].concat();
        $stdin.write_all(&code).await?;
    }}
}
async fn pull_result(stdout: &mut Bytes<ChildStdout>, eof: &[u8]) -> Vec<u8> {
    let mut buff = vec![];
    while let Some(Ok(b)) = stdout.next().await {
        buff.push(b);
        if buff.ends_with(eof) {
            buff.truncate(buff.len() - eof.len());
            break;
        }
    }
    buff
}

macro_rules! repl {
    ($context:expr, $($arg:tt)*) => {{
        js_code!($context.0, $context.2, $($arg)*);
        pull_result(&mut $context.1, $context.2).await
    }}
}
static EOF: &[u8] = &[0, 1, 0];

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
    let _server =
        async_std::task::spawn(
            async move { run_server(server_core, HOSTNAME, PORT).await.unwrap() },
        );

    // create a reader core in javascript
    let rkey_hex = serialize_public_key(&rkey);
    let (_dir, mut child) = run_hypercore_js(
        Some(&rkey_hex),
        &format!(
            "
const {{ repl }} = require('./repl.js');
// start a read-eval-print-loop we use from rust
await repl();"
        ),
        vec![
            format!("{}/utils.js", path_to_js_dir()?.to_string_lossy()),
            format!("{}/repl.js", path_to_js_dir()?.to_string_lossy()),
        ],
    )?;
    // data used for the rs -> js repl
    let mut context = (
        child.stdin.take().unwrap(),
        child.stdout.take().unwrap().bytes(),
        EOF,
    );

    // connect the js reader core and update it
    let result = repl!(
        context,
        "
socket = net.connect('{PORT}', '{HOSTNAME}');
await core.update();
socket.pipe(core.replicate(true)).pipe(socket)
await core.update({{wait: true}});
// print the length of the core so we can check it in rust
process.stdout.write(String((await core.info()).length));
"
    );
    // assert the js core has 4 blocks
    assert_eq!(result, b"4");

    // close the socket in js, stop the repl. When repl is stopped hypercore is closed
    let _ = repl!(
        context,
        "socket.destroy();
queue.done();"
    );

    // ensure js process closes properly
    assert_eq!(child.output().await?.status.code(), Some(0));
    Ok(())
}

#[tokio::test]
async fn read_eval_print_macro_works() -> Result<()> {
    let (_dir, mut child) = run_js_async_block(
        &format!(
            "
const {{ repl }} = require('./repl.js');
await repl();
"
        ),
        vec![
            format!("{}/utils.js", path_to_js_dir()?.to_string_lossy()),
            format!("{}/repl.js", path_to_js_dir()?.to_string_lossy()),
        ],
    )?;

    let mut context = (
        child.stdin.take().unwrap(),
        child.stdout.take().unwrap().bytes(),
        EOF,
    );

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
    println!("{}", String::from_utf8_lossy(&child.output().await?.stderr));
    Ok(())
}

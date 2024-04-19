mod common;

use std::time::Duration;

use async_process::ChildStdout;
use async_std::{net::TcpListener, task::sleep};
use common::{js::run_hypercore_js, run_server, Result};
use futures_lite::{io::Bytes, AsyncReadExt, AsyncWriteExt, StreamExt};
use replicator::Replicate;
//use futures_lite::FutureExt;
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
    ($stdin:expr, $stdout:expr, $eof:expr, $($arg:tt)*) => {{
        js_code!($stdin, $eof, $($arg)*);
        pull_result($stdout, $eof).await
    }}
}

#[tokio::test]
async fn rs_server_js_client_initial_data_moves() -> Result<()> {
    let (rkey, wkey) = make_reader_and_writer_keys();

    let core = ram_core(Some(&wkey)).await;
    let server_core = core.clone();
    let _server =
        async_std::task::spawn(
            async move { run_server(server_core, HOSTNAME, PORT).await.unwrap() },
        );

    let rkey_hex = serialize_public_key(&rkey);
    let (_dir, mut child) = run_hypercore_js(
        Some(&rkey_hex),
        &format!(
            "
const {{ repl }} = require('./repl.js');

await repl();"
        ),
        vec![
            format!("{}/utils.js", path_to_js_dir()?.to_string_lossy()),
            format!("{}/repl.js", path_to_js_dir()?.to_string_lossy()),
        ],
    )?;

    let eof: &[u8] = &[0, 1, 0];

    let mut stdout = child.stdout.take().unwrap().bytes();
    let mut stdin = child.stdin.take().unwrap();
    macro_rules! run {
    ($($arg:tt)*) => {{
        repl!(stdin, &mut stdout, eof, $($arg)*)
    }}
}

    let result = run!(
        "
socket = net.connect('{PORT}', '{HOSTNAME}');
await core.update();
socket.pipe(core.replicate(true)).pipe(socket)
await core.update({{wait: true}});
process.stdout.write(String((await core.info()).length));
"
    );
    assert_eq!(result, b"4");

    core.lock().await.append(b"more").await?;

    /* FAILS
        let result = run!(
            "
    await new Promise(r => setTimeout(r, 1e3*2));
    await core.update({{wait: true}});
    process.stdout.write(String((await core.info()).length));
    "
        );
        assert_eq!(result, b"5");
        */

    let _result = run!(
        "
                        socket.destroy();
                        queue.done();
                        "
    );
    dbg!(&child.output().await?);
    Ok(())
}

#[tokio::test]
async fn repl() -> Result<()> {
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

    let eof: &[u8] = &[0];

    let mut stdout = child.stdout.take().unwrap().bytes();
    let mut stdin = child.stdin.take().unwrap();

    let result = repl!(stdin, &mut stdout, eof, "process.stdout.write('fooo6!');");
    assert_eq!(result, b"fooo6!");
    let result = repl!(
        stdin,
        &mut stdout,
        eof,
        "
a = 66;
b = 7 + a;
process.stdout.write(`${{b}}`);
"
    );
    assert_eq!(result, b"73");

    let _result = repl!(stdin, &mut stdout, eof, "queue.done();");
    println!("{}", String::from_utf8_lossy(&child.output().await?.stderr));
    Ok(())
}

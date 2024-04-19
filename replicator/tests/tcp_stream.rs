mod common;

use std::time::Duration;

use async_process::ChildStdout;
use async_std::task::sleep;
use common::{js::run_hypercore_js, run_server, Result};
use futures_lite::{io::Bytes, AsyncReadExt, AsyncWriteExt, StreamExt};
//use futures_lite::FutureExt;
use utils::make_reader_and_writer_keys;

use crate::common::{
    js::{path_to_js_dir, run_js_async_block},
    serialize_public_key, HOSTNAME, PORT,
};

#[tokio::test]
async fn rs_server_js_client() -> Result<()> {
    let (rkey, wkey) = make_reader_and_writer_keys();

    let _server =
        async_std::task::spawn(async move { run_server(wkey, HOSTNAME, PORT).await.unwrap() });

    let rkey_hex = serialize_public_key(&rkey);
    let (_dir, mut child) = run_hypercore_js(
        Some(&rkey_hex),
        &format!(
            "
const {{ repl }} = require('./repl.js');

await repl();
/*
core.on('append', x => {{
    console.log('core appeded too: ', x);
}});
let socket = net.connect('{PORT}', '{HOSTNAME}');
console.log('info = ', await core.info());
await core.update();
console.log('info after =', await core.info());

socket.pipe(core.replicate(true)).pipe(socket)
*/
"
        ),
        vec![
            format!("{}/utils.js", path_to_js_dir()?.to_string_lossy()),
            format!("{}/repl.js", path_to_js_dir()?.to_string_lossy()),
        ],
    )?;

    sleep(Duration::from_millis(500)).await;
    // TODO the NULL byte eof thing aint working
    /* TODO add these?
    let line_limit = 88;
    let timeout_seconds = 5;
    */
    let eof_flag = 0u8;

    let stdout = child.stdout.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write(b"console.log(queue);\n").await?;
    stdin
        .write(b"console.log('asnteohushaosenthusahu');\n")
        .await?;
    stdin.flush().await?;
    stdin.write(b"queue.done();\nconsole.log('hhh');").await?;
    stdin.flush().await?;
    // THIS WAS THE PROBLEM
    //stdout.read_to_end(&mut buff).await?;
    let mut stream = stdout.bytes();
    let mut buff = vec![];
    while let Some(Ok(b)) = stream.next().await {
        buff.push(b);
        if b == eof_flag {
            break;
        }
    }
    println!(
        "GOT FRM STDOUT #1 -----
{}
---- END STDOUT",
        String::from_utf8_lossy(&buff)
    );
    //    let mut buff = vec![];
    //    stdout.read_to_end(&mut buff).await?;
    //
    //    println!("GOT FRM STDOUT #2 {}", String::from_utf8_lossy(&buff));

    dbg!(child.output().await?);
    //println!("{}", String::from_utf8_lossy(&child.output().await?.stderr));
    Ok(())
}

static START_DELIM_MSG: &[u8] = b"; process.stdout.write('";
static END_DELIM_MSG: &[u8] = b"');";

macro_rules! js_code {
    ($stdin:expr, $eof:expr, $($arg:tt)*) => {{
        let code = [
        format!($($arg)*).as_bytes(),
            START_DELIM_MSG, $eof, END_DELIM_MSG].concat();
        $stdin.write_all(&code).await?;
    }}
}
async fn pull_result(stream: &mut Bytes<ChildStdout>, eof: &[u8]) -> Vec<u8> {
    let mut buff = vec![];
    while let Some(Ok(b)) = stream.next().await {
        buff.push(b);
        if buff.ends_with(eof) {
            buff.truncate(buff.len() - eof.len());
            break;
        }
    }
    buff
}
macro_rules! repl {
    ($stdin:expr, $stream:expr, $eof:expr, $($arg:tt)*) => {{
        js_code!($stdin, $eof, $($arg)*);
        pull_result($stream, $eof).await
    }}
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
    dbg!(child.output().await?);
    Ok(())
}

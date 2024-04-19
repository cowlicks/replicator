mod common;

use std::time::Duration;

use async_std::task::sleep;
use common::{js::run_hypercore_js, run_server, Result};
use futures_lite::{AsyncReadExt, AsyncWriteExt, StreamExt};
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

    let eof_flag = 0u8;

    let stdout = child.stdout.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();

    // We need to end the line with a "\n" for the nodejs readline to push it through
    stdin
        .write_all(b"console.log(queue); console.log('\0');\n")
        .await?;
    stdin.flush().await?;
    let mut stream = stdout.bytes();
    let mut buff = vec![];
    while let Some(Ok(b)) = stream.next().await {
        buff.push(b);
        println!("buff {}", String::from_utf8_lossy(&buff));
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
    dbg!(child.output().await?);
    Ok(())
}

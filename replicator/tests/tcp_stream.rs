mod common;
use common::{js::run_js, run_server, Result};
use utils::make_reader_and_writer_keys;

use crate::common::{serialize_public_key, HOSTNAME, PORT};

#[tokio::test]
async fn rs_server_js_client() -> Result<()> {
    let (rkey, wkey) = make_reader_and_writer_keys();

    let _server = async_std::task::spawn(async move { run_server(wkey, HOSTNAME, PORT).await });

    let rkey_hex = serialize_public_key(&rkey);
    let _ = run_js(
        Some(&rkey_hex),
        "
console.log(key);
console.log('-------------------------------------------------------------------------');
",
    )?;
    Ok(())
}

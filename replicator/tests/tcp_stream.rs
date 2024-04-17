mod common;
use common::{js::run_js, run_server, Result};

use crate::common::{HOSTNAME, PORT};

#[tokio::test]
async fn rs_server_js_client() -> Result<()> {
    let _ = run_js(
        "
console.log(key)
",
    )?;
    run_server(HOSTNAME, PORT).await?;
    Ok(())
}

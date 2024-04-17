use super::{_run_make_from_with, git_root, join_paths, run_code};
use std::{path::PathBuf, process::Output};

pub static REL_PATH_TO_NODE_MODULES: &str = "./replicator/tests/common/js/node_modules";
pub static _REL_PATH_TO_JS_DIR: &str = "./replicator/tests/common/js";

pub fn _require_js_data() -> Result<(), Box<dyn std::error::Error>> {
    let _ = _run_make_from_with(_REL_PATH_TO_JS_DIR, "")?;
    Ok(())
}

pub fn path_to_node_modules() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = join_paths!(git_root()?, &REL_PATH_TO_NODE_MODULES);
    Ok(p.into())
}

static POST_SCRIPT: &str = "
await core.close();
})()
";

static SCRIPT_FILE_NAME: &str = "script.js";

fn build_command(_working_dir: &str, script_path: &str) -> String {
    format!(
        "NODE_PATH={} node {}",
        path_to_node_modules().unwrap().display(),
        script_path
    )
}

fn ram_client_pre_script(_hostname: &str, _port: &str, key: Option<&str>) -> String {
    let key = match key {
        None => "undefined".to_string(),
        Some(k) => format!("'{k}'"),
    };
    format!(
        "
const RAM = require('random-access-memory');
const Hypercore = require('hypercore');

const net = require('net');
const write = (x) => process.stdout.write(x);

(async() => {{
    const key = {key};
    const core = new Hypercore(RAM, key);
    await core.ready();
    write('KEY=' + core.key.toString('hex'));
    "
    )
}

pub fn run_js(key: Option<&str>, script: &str) -> Result<Output, Box<dyn std::error::Error>> {
    run_code(
        &ram_client_pre_script(super::HOSTNAME, super::PORT, key),
        script,
        POST_SCRIPT,
        SCRIPT_FILE_NAME,
        build_command,
        vec![],
    )
}

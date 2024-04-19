use tempfile::TempDir;

use super::{_run_make_from_with, git_root, join_paths, run_code};
use std::path::PathBuf;

pub static REL_PATH_TO_NODE_MODULES: &str = "./replicator/tests/common/js/node_modules";
pub static REL_PATH_TO_JS_DIR: &str = "./replicator/tests/common/js";

pub fn _require_js_data() -> Result<(), Box<dyn std::error::Error>> {
    let _ = _run_make_from_with(REL_PATH_TO_JS_DIR, "")?;
    Ok(())
}

pub fn path_to_js_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(join_paths!(git_root()?, &REL_PATH_TO_JS_DIR).into())
}

pub fn path_to_node_modules() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = join_paths!(git_root()?, &REL_PATH_TO_NODE_MODULES);
    Ok(p.into())
}

fn async_iiaf_template(async_body_str: &str) -> String {
    format!(
        "(async () => {{
{}
}})()",
        async_body_str
    )
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
RAM = require('random-access-memory');
Hypercore = require('hypercore');

net = require('net');
write = (x) => process.stdout.write(x);
start = Date.now();
key = {key};
core = new Hypercore(RAM, key);

(async() => {{
    await core.ready();
    "
    )
}

pub fn run_js(
    code_string: &str,
    copy_dirs: Vec<String>,
) -> Result<(TempDir, async_process::Child), Box<dyn std::error::Error>> {
    run_code(&code_string, SCRIPT_FILE_NAME, build_command, copy_dirs)
}

pub fn run_js_async_block(
    code_block_string: &str,
    copy_dirs: Vec<String>,
) -> Result<(TempDir, async_process::Child), Box<dyn std::error::Error>> {
    let code_string = async_iiaf_template(code_block_string);
    println!("{}", code_string);
    run_code(&code_string, SCRIPT_FILE_NAME, build_command, copy_dirs)
}

pub fn run_hypercore_js(
    key: Option<&str>,
    script: &str,
    copy_dirs: Vec<String>,
) -> Result<(TempDir, async_process::Child), Box<dyn std::error::Error>> {
    let code_string = format!(
        "{}\n{}\n{}\n",
        ram_client_pre_script(super::HOSTNAME, super::PORT, key),
        script,
        POST_SCRIPT
    );
    println!("{}", code_string);
    run_js(&code_string, copy_dirs)
}

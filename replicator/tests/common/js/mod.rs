use tempfile::TempDir;

use super::{_run_make_from_with, git_root, join_paths, run_code};
use std::path::PathBuf;

pub static REL_PATH_TO_NODE_MODULES: &str = "./replicator/tests/common/js/node_modules";
pub static REL_PATH_TO_JS_DIR: &str = "./replicator/tests/common/js";
static POST_SCRIPT: &str = "
await core.close();
})()
";

static SCRIPT_FILE_NAME: &str = "script.js";
pub static RUN_REPL_CODE: &str = "const { repl } = require('./repl.js');
// start a read-eval-print-loop we use from rust
await repl();";

pub fn require_js_data() -> Result<(), Box<dyn std::error::Error>> {
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

pub fn async_iiaf_template(async_body_str: &str) -> String {
    format!(
        "(async () => {{
{}
}})()",
        async_body_str
    )
}

fn build_command(_working_dir: &str, script_path: &str) -> String {
    format!(
        "NODE_PATH={} node {}",
        path_to_node_modules().unwrap().display(),
        script_path
    )
}

fn ram_client_pre_script(key: Option<&str>) -> String {
    let key = key.map(|k| format!("'{k}'")).unwrap_or("undefined".into());
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

pub fn run_hypercore_js(
    key: Option<&str>,
    script: &str,
    copy_dirs: Vec<String>,
) -> Result<(TempDir, async_process::Child), Box<dyn std::error::Error>> {
    let code_string = format!(
        "{}\n{}\n{}\n",
        ram_client_pre_script(key),
        script,
        POST_SCRIPT
    );
    println!("{}", code_string);
    run_js(&code_string, copy_dirs)
}

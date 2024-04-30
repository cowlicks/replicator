use async_process::ChildStdout;
use futures_lite::{io::Bytes, AsyncReadExt, StreamExt};
use tempfile::TempDir;

use super::{_run_make_from_with, git_root, join_paths, run_code};
use std::path::PathBuf;

pub static REL_PATH_TO_NODE_MODULES: &str = "./replicator/tests/common/js/node_modules";
pub static REL_PATH_TO_JS_DIR: &str = "./replicator/tests/common/js";

static DEFAULT_EOF: &[u8] = &[0, 1, 0];
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

pub fn connect_and_teardown_js_core(port: &str, hostname: &str, code: &str) -> String {
    format!(
        "
socket = net.connect('{port}', '{hostname}');
socket.pipe(core.replicate(true)).pipe(socket)
await core.update({{wait: true}});
{code}
socket.destroy();
"
    )
}

fn build_command(_working_dir: &str, script_path: &str) -> String {
    format!(
        "NODE_PATH={} node {}",
        path_to_node_modules().unwrap().display(),
        script_path
    )
}

static CORE_NAME: &str = "core";

fn ram_hypercore_prerequisite_code(key: Option<&str>, core_var_name: &str) -> String {
    let key = key.map(|k| format!("'{k}'")).unwrap_or("undefined".into());
    format!(
        "
RAM = require('random-access-memory');
Hypercore = require('hypercore');

net = require('net');
write = (x) => process.stdout.write(x);
start = Date.now();
key = {key};
{core_var_name} = new Hypercore(RAM, key);
    "
    )
}

pub struct JsContext {
    /// needs to be held so working directory is not dropped
    pub dir: TempDir,
    pub stdin: async_process::ChildStdin,
    pub stdout: Bytes<async_process::ChildStdout>,
    pub child: async_process::Child,
    pub eof: Vec<u8>,
}

pub fn run_js(
    code_string: &str,
    copy_dirs: Vec<String>,
) -> Result<JsContext, Box<dyn std::error::Error>> {
    let (dir, mut child) = run_code(&code_string, SCRIPT_FILE_NAME, build_command, copy_dirs)?;
    Ok(JsContext {
        dir,
        stdin: child.stdin.take().unwrap(),
        stdout: child.stdout.take().unwrap().bytes(),
        child,
        eof: DEFAULT_EOF.to_vec(),
    })
}

pub fn run_hypercore_js(
    key: Option<&str>,
    script: &str,
    copy_dirs: Vec<String>,
) -> Result<JsContext, Box<dyn std::error::Error>> {
    let code_string = format!(
        "{}\n{}\n",
        ram_hypercore_prerequisite_code(key, CORE_NAME),
        &async_iiaf_template(&format!(
            "
await core.ready();
{script}
await core.close();"
        ))
    );
    run_js(&code_string, copy_dirs)
}

macro_rules! run_async_js_block {
    ($stdin:expr, $eof:expr, $($arg:tt)*) => {{
        let block = format!($($arg)*);
        let code = [
            b";(async () =>{\n",
            block.as_bytes(),
            b"; process.stdout.write('",
            $eof,
            b"');",
            b"})();",
        ].concat();
        $stdin.write_all(&code).await?;
    }}
}
pub(crate) use run_async_js_block;

pub async fn pull_result_from_stdout(stdout: &mut Bytes<ChildStdout>, eof: &[u8]) -> Vec<u8> {
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
        run_async_js_block!($context.stdin, &$context.eof, $($arg)*);
        crate::common::js::pull_result_from_stdout(&mut $context.stdout, &$context.eof).await
    }}
}
pub(crate) use repl;

macro_rules! flush_stdout {
    ($context:expr) => {{
        repl!($context, "")
    }};
}
pub(crate) use flush_stdout;

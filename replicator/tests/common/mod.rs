// cargo thinks everything in here is unused even though it is used in the integration tests
use std::{
    fs::File,
    io::Write,
    process::{Command, Output},
};

use async_std::net::TcpListener;
use futures_lite::StreamExt;
use hypercore::PartialKeypair;
use replicator::{Replicate, ReplicatorError};

pub mod js;

pub type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

pub static _PATH_TO_DATA_DIR: &str = "tests/common/js/data";

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Problem in tests: {0}")]
    TestError(String),
}

macro_rules! join_paths {
    ( $path:expr$(,)?) => {
        $path
    };
    ( $p1:expr,  $p2:expr) => {{
        let p = std::path::Path::new(&*$p1).join($p2);
        p.display().to_string()
    }};
    ( $p1:expr,  $p2:expr, $($tail:tt)+) => {{
        let p = std::path::Path::new($p1).join($p2);
        join_paths!(p.display().to_string(), $($tail)*)
    }};
}

pub(crate) use join_paths;

pub fn git_root() -> Result<String> {
    let x = Command::new("sh")
        .arg("-c")
        .arg("git rev-parse --show-toplevel")
        .output()?;
    Ok(String::from_utf8(x.stdout)?.trim().to_string())
}

pub fn _get_data_dir() -> Result<String> {
    Ok(join_paths!(git_root()?, &_PATH_TO_DATA_DIR))
}

pub fn _run_script_relative_to_git_root(script: &str) -> Result<Output> {
    Ok(Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && {}", git_root()?, script))
        .output()?)
}

pub fn run_code(
    pre_script: &str,
    script: &str,
    post_script: &str,
    script_file_name: &str,
    build_command: impl FnOnce(&str, &str) -> String,
    copy_dirs: Vec<String>,
) -> Result<Output> {
    let working_dir = tempfile::tempdir()?;

    let code = format!(
        "{pre_script}
{script}
{post_script}
"
    );
    let script_path = working_dir.path().join(script_file_name);
    let script_file = File::create(&script_path)?;
    write!(&script_file, "{}", &code)?;

    let working_dir_path = working_dir.path().display().to_string();
    // copy dirs into working dir
    for dir in copy_dirs {
        let dir_cp_cmd = Command::new("cp")
            .arg("-r")
            .arg(&dir)
            .arg(&working_dir_path)
            .output()?;
        if dir_cp_cmd.status.code() != Some(0) {
            return Err(Box::new(Error::TestError(format!(
                "failed to copy dir [{dir}] to [{working_dir_path}] got stderr: {}",
                String::from_utf8_lossy(&dir_cp_cmd.stderr),
            ))));
        }
    }
    let script_path_str = script_path.display().to_string();
    let cmd = build_command(&working_dir_path, &script_path_str);
    check_cmd_output(Command::new("sh").arg("-c").arg(cmd).output()?)
}

pub fn _run_make_from_with(dir: &str, arg: &str) -> Result<Output> {
    let path = join_paths!(git_root()?, dir);
    let cmd = format!("cd {path} && flock make.lock make {arg} && rm -f make.lock ");
    let out = check_cmd_output(Command::new("sh").arg("-c").arg(cmd).output()?)?;
    Ok(out)
}

pub fn _parse_json_result(output: &Output) -> Result<Vec<Vec<u8>>> {
    let stdout = String::from_utf8(output.stdout.clone())?;
    let res: Vec<String> = serde_json::from_str(&stdout)?;
    Ok(res.into_iter().map(|x| x.into()).collect())
}

#[allow(unused_macros)]
macro_rules! write_range_to_hb {
    ($hb:expr) => {{
        write_range_to_hb!($hb, 0..100)
    }};
    ($hb:expr, $range:expr) => {{
        let hb = $hb;
        let keys: Vec<Vec<u8>> = ($range)
            .map(|x| x.clone().to_string().as_bytes().to_vec())
            .collect();

        for k in keys.iter() {
            let val: Option<&[u8]> = Some(k);
            hb.put(k, val).await?;
        }
        keys
    }};
}

use utils::ram_core;
#[allow(unused_imports)]
pub(crate) use write_range_to_hb;

pub fn check_cmd_output(out: Output) -> Result<Output> {
    eprint!("{}", String::from_utf8_lossy(&out.stdout));
    if out.status.code() != Some(0) {
        return Err(Box::new(Error::TestError(format!(
            "comand output status was not zero. Got:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ))));
    }
    Ok(out)
}

pub static HOSTNAME: &str = "127.0.0.1";
pub static PORT: &str = "15001";

pub fn serialize_public_key(key: &PartialKeypair) -> String {
    hex::encode(key.public.as_bytes())
}

pub async fn run_server(
    key: PartialKeypair,
    hostname: &str,
    port: &str,
) -> std::result::Result<(), ReplicatorError> {
    let core = ram_core(Some(&key)).await;

    dbg!(serialize_public_key(core.lock().await.key_pair()));
    let address = format!("{hostname}:{port}");
    let listener = TcpListener::bind(&address).await?;
    let mut incoming = listener.incoming();
    let Some(Ok(stream)) = incoming.next().await else {
        panic!("No connections");
    };

    Ok(core.replicate(stream, false).await?)
}

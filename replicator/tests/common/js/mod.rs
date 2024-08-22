use super::{_run_make_from_with, git_root, join_paths};
use std::path::PathBuf;

pub static REL_PATH_TO_NODE_MODULES: &str = "./replicator/tests/common/js/node_modules";
pub static REL_PATH_TO_JS_DIR: &str = "./replicator/tests/common/js";

pub fn require_js_data() -> Result<(), Box<dyn std::error::Error>> {
    let _ = _run_make_from_with(REL_PATH_TO_JS_DIR, "node_modules")?;
    Ok(())
}

pub fn path_to_node_modules() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = join_paths!(git_root()?, &REL_PATH_TO_NODE_MODULES);
    Ok(p.into())
}

//! Build script for `league-eval`: records the repository git revision in `LEAGUE_EVAL_GIT_SHA`.

use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let workspace_git_head = Path::new(&manifest_dir).join("../../.git/HEAD");
    if workspace_git_head.exists() {
        println!("cargo:rerun-if-changed={}", workspace_git_head.display());
    }

    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=LEAGUE_EVAL_GIT_SHA={sha}");
}

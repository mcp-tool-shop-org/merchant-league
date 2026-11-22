//! Build script for `league-eval`: records the verified repository git revision in
//! `LEAGUE_EVAL_GIT_SHA` and watches git HEAD/ref paths in both checkouts and worktrees.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git_output(manifest_dir: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .current_dir(manifest_dir)
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn resolve_git_dir(manifest_dir: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        manifest_dir.join(p)
    }
}

fn emit_watch_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn is_hex40(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn main() {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if let Some(git_dir_raw) = git_output(&manifest_dir, &["rev-parse", "--git-dir"]) {
        let git_dir = resolve_git_dir(&manifest_dir, &git_dir_raw);
        let head_path = git_dir.join("HEAD");
        emit_watch_if_exists(&head_path);

        let common_dir = git_output(&manifest_dir, &["rev-parse", "--git-common-dir"])
            .map(|raw| resolve_git_dir(&manifest_dir, &raw))
            .unwrap_or_else(|| git_dir.clone());

        if let Ok(head_contents) = fs::read_to_string(&head_path)
            && let Some(symref) = head_contents.trim().strip_prefix("ref:")
        {
            let ref_rel = symref.trim();
            if !ref_rel.is_empty() {
                emit_watch_if_exists(&git_dir.join(ref_rel));
                emit_watch_if_exists(&common_dir.join(ref_rel));
            }
        }
        emit_watch_if_exists(&common_dir.join("packed-refs"));
    }

    let sha = git_output(&manifest_dir, &["rev-parse", "HEAD"])
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| is_hex40(s))
        .unwrap_or_default();

    println!("cargo:rustc-env=LEAGUE_EVAL_GIT_SHA={sha}");
}

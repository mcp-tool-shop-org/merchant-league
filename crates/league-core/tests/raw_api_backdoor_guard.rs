//! Must items 3 & 4 guard: keeps raw Auditor hint APIs and live-Sweep preview back doors
//! unreachable as ordinary public API in `league-core`, enforces the single
//! `#[cfg(feature = "engine-internals")] #[doc(hidden)] pub mod engine_internals` escape hatch,
//! and blocks seat-facing crates (`league-tui`, `league-relay`, etc.) from enabling or importing
//! `engine-internals` or depending on `league-core` where forbidden.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use league_core::engine_internals::SweepMode;

const FORBIDDEN_RAW_FNS: [&str; 7] = [
    "private_hints_for_auditor",
    "preview_bundle",
    "preview_bundle_live",
    "preview_bundle_in_mode",
    "preview_bundle_with_context",
    "preview_bundle_with_context_live",
    "preview_bundle_with_context_and_mode",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Pound,
    Bang,
    Semi,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Other(char),
}

const fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

const fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn tokenize_rust(source: &str) -> Vec<Tok> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Line comment.
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Nested block comment.
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1_usize;
            while i < len && depth > 0 {
                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Normal string or byte/c string (`"..."`, `b"..."`, `c"..."`).
        if b == b'"' || (matches!(b, b'b' | b'c') && i + 1 < len && bytes[i + 1] == b'"') {
            i += if b == b'"' { 1 } else { 2 };
            let start = i;
            let mut end = i;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    end = i;
                } else if bytes[i] == b'"' {
                    end = i;
                    i += 1;
                    break;
                } else {
                    i += 1;
                    end = i;
                }
            }
            tokens.push(Tok::Str(source[start..end].to_owned()));
            continue;
        }
        // Raw string (`r"..."`, `r#"..."#`, `br#"..."#`, `cr#"..."#`) vs raw ident (`r#ident`).
        let raw_start = if b == b'r' {
            Some(i + 1)
        } else if matches!(b, b'b' | b'c') && i + 1 < len && bytes[i + 1] == b'r' {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut j) = raw_start {
            let mut hashes = 0_usize;
            while j < len && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                j += 1;
                let start = j;
                let mut end = j;
                while j < len {
                    if bytes[j] == b'"' {
                        let mut matched = 0_usize;
                        while matched < hashes
                            && j + 1 + matched < len
                            && bytes[j + 1 + matched] == b'#'
                        {
                            matched += 1;
                        }
                        if matched == hashes {
                            end = j;
                            j += 1 + hashes;
                            break;
                        }
                    }
                    j += 1;
                    end = j;
                }
                tokens.push(Tok::Str(source[start..end].to_owned()));
                i = j;
                continue;
            } else if b == b'r' && hashes == 1 && j < len && is_ident_start(bytes[j]) {
                let ident_start = j;
                while j < len && is_ident_continue(bytes[j]) {
                    j += 1;
                }
                tokens.push(Tok::Ident(source[ident_start..j].to_owned()));
                i = j;
                continue;
            }
        }
        // Byte / char literals vs lifetime (`'a`).
        if b == b'b' && i + 1 < len && bytes[i + 1] == b'\'' {
            i += 2;
            if i < len && bytes[i] == b'\\' {
                i = (i + 2).min(len);
            }
            while i < len && bytes[i] != b'\'' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }
        if b == b'\'' {
            if i + 2 < len && bytes[i + 1] == b'\\' {
                let mut j = i + 3;
                while j < len && bytes[j] != b'\'' && j - i <= 8 {
                    j += 1;
                }
                if j < len && bytes[j] == b'\'' {
                    i = j + 1;
                    continue;
                }
            } else if i + 1 < len {
                let mut chars = source[i + 1..].char_indices();
                if let Some((_, ch)) = chars.next() {
                    let after_ch = i + 1 + ch.len_utf8();
                    if after_ch < len && bytes[after_ch] == b'\'' {
                        i = after_ch + 1;
                        continue;
                    }
                }
            }
            tokens.push(Tok::Other('\''));
            i += 1;
            continue;
        }

        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            tokens.push(Tok::Ident(source[start..i].to_owned()));
            continue;
        }

        let tok = match b {
            b'#' => Tok::Pound,
            b'!' => Tok::Bang,
            b';' => Tok::Semi,
            b'{' => Tok::LBrace,
            b'}' => Tok::RBrace,
            b'(' => Tok::LParen,
            b')' => Tok::RParen,
            b'[' => Tok::LBracket,
            b']' => Tok::RBracket,
            other => Tok::Other(other as char),
        };
        tokens.push(tok);
        i += 1;
    }
    tokens
}

fn collect_rs_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        let file_type = entry
            .file_type()
            .unwrap_or_else(|err| panic!("failed to read file type {}: {err}", path.display()));
        if file_type.is_dir() {
            collect_rs_files_recursive(&path, out);
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[derive(Default, Clone)]
struct AttrFlags {
    cfg_test: bool,
    cfg_engine_internals: bool,
    doc_hidden: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Root {
        public: bool,
    },
    Module {
        public: bool,
        is_engine_internals: bool,
        cfg_test: bool,
    },
    ImplTurnEngine {
        cfg_test: bool,
    },
    Block {
        cfg_test: bool,
    },
}

#[derive(Clone, Copy)]
enum Visibility {
    Private,
    CrateOrRestricted,
    Public,
}

fn parse_tok_attr(tokens: &[Tok], mut idx: usize, flags: &mut AttrFlags) -> usize {
    idx += 1;
    let is_inner = idx < tokens.len() && matches!(tokens[idx], Tok::Bang);
    if is_inner {
        idx += 1;
    }
    if idx >= tokens.len() || !matches!(tokens[idx], Tok::LBracket) {
        return idx;
    }
    let start = idx + 1;
    let mut depth = 1_usize;
    idx += 1;
    while idx < tokens.len() && depth > 0 {
        match tokens[idx] {
            Tok::LBracket => depth += 1,
            Tok::RBracket => depth -= 1,
            _ => {}
        }
        idx += 1;
    }
    if is_inner {
        return idx;
    }
    let end = idx.saturating_sub(1);
    if start >= end {
        return idx;
    }
    let inner = &tokens[start..end];
    if let Some(Tok::Ident(head)) = inner.first() {
        if head == "cfg" {
            if inner
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "test"))
            {
                flags.cfg_test = true;
            }
            if inner
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "feature"))
                && inner
                    .iter()
                    .any(|t| matches!(t, Tok::Str(s) if s == "engine-internals"))
            {
                flags.cfg_engine_internals = true;
            }
        } else if head == "doc"
            && inner
                .iter()
                .any(|t| matches!(t, Tok::Ident(s) if s == "hidden"))
        {
            flags.doc_hidden = true;
        }
    }
    idx
}

fn scan_file_public_api(
    rel_path: &str,
    source: &str,
    is_lib_rs: bool,
) -> Result<(bool, Vec<String>), String> {
    let tokens = tokenize_rust(source);
    let mut scopes = vec![ScopeKind::Root { public: true }];
    let mut attrs = AttrFlags::default();
    let mut seen_engine_internals_mod = false;
    let mut visited_public_fns = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let in_cfg_test = scopes.iter().any(|s| match s {
            ScopeKind::Root { .. } => false,
            ScopeKind::Module { cfg_test, .. }
            | ScopeKind::ImplTurnEngine { cfg_test }
            | ScopeKind::Block { cfg_test } => *cfg_test,
        });
        let in_public_non_internal_mod = scopes.iter().all(|s| match s {
            ScopeKind::Root { public } => *public,
            ScopeKind::Module {
                public,
                is_engine_internals,
                ..
            } => *public && !*is_engine_internals,
            ScopeKind::ImplTurnEngine { .. } => true,
            ScopeKind::Block { .. } => false,
        });
        let in_impl_turn_engine = matches!(scopes.last(), Some(ScopeKind::ImplTurnEngine { .. }));
        let in_engine_internals_mod = matches!(
            scopes.last(),
            Some(ScopeKind::Module {
                is_engine_internals: true,
                ..
            })
        );

        match &tokens[i] {
            Tok::Pound => {
                i = parse_tok_attr(&tokens, i, &mut attrs);
                continue;
            }
            Tok::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::LBrace => {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            Tok::Semi => {
                attrs = AttrFlags::default();
                i += 1;
                continue;
            }
            _ => {}
        }

        if in_cfg_test || attrs.cfg_test {
            if matches!(
                tokens[i],
                Tok::Ident(ref s)
                    if matches!(s.as_str(), "mod" | "fn" | "impl" | "struct" | "enum" | "trait")
            ) {
                let mut j = i + 1;
                while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                    j += 1;
                }
                if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                    scopes.push(ScopeKind::Block { cfg_test: true });
                    attrs = AttrFlags::default();
                    i = j + 1;
                    continue;
                }
                attrs = AttrFlags::default();
                i = (j + 1).min(tokens.len());
                continue;
            }
            i += 1;
            continue;
        }

        // Parse optional visibility (`pub` vs `pub(...)` vs private).
        let (vis, after_vis) = if matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            if i + 1 < tokens.len() && matches!(tokens[i + 1], Tok::LParen) {
                let mut j = i + 2;
                let mut depth = 1_usize;
                while j < tokens.len() && depth > 0 {
                    match tokens[j] {
                        Tok::LParen => depth += 1,
                        Tok::RParen => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                (Visibility::CrateOrRestricted, j)
            } else {
                (Visibility::Public, i + 1)
            }
        } else {
            (Visibility::Private, i)
        };

        // Track `impl TurnEngine { ... }`.
        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "impl") {
            let mut j = after_vis + 1;
            let mut type_idents = Vec::new();
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                if let Tok::Ident(ref s) = tokens[j] {
                    type_idents.push(s.as_str());
                }
                j += 1;
            }
            let is_turn_engine =
                type_idents.last().copied() == Some("TurnEngine") && !type_idents.contains(&"for");
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(if is_turn_engine {
                    ScopeKind::ImplTurnEngine {
                        cfg_test: attrs.cfg_test,
                    }
                } else {
                    ScopeKind::Block {
                        cfg_test: attrs.cfg_test,
                    }
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check `mod <name> { ... }` or `mod <name>;`.
        if matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "mod")
            && let Some(Tok::Ident(mod_name)) = tokens.get(after_vis + 1)
        {
            let is_pub = matches!(vis, Visibility::Public);
            let mut j = after_vis + 2;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            let is_inline = j < tokens.len() && matches!(tokens[j], Tok::LBrace);

            if mod_name == "engine_internals" {
                if !is_lib_rs || !is_pub || !attrs.cfg_engine_internals || !attrs.doc_hidden {
                    return Err(format!(
                        "{rel_path}: `mod engine_internals` must be `#[cfg(feature = \"engine-internals\")] #[doc(hidden)] pub mod engine_internals` in lib.rs"
                    ));
                }
                seen_engine_internals_mod = true;
            } else if attrs.cfg_engine_internals {
                return Err(format!(
                    "{rel_path}: forbidden second `#[cfg(feature = \"engine-internals\")]` module `{mod_name}` (only `engine_internals` in lib.rs is allowed)"
                ));
            }

            if is_inline {
                scopes.push(ScopeKind::Module {
                    public: is_pub,
                    is_engine_internals: mod_name == "engine_internals",
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check `pub use ...;`.
        if matches!(vis, Visibility::Public)
            && matches!(tokens.get(after_vis), Some(Tok::Ident(s)) if s == "use")
        {
            let mut j = after_vis + 1;
            let mut reexported = Vec::new();
            while j < tokens.len() && !matches!(tokens[j], Tok::Semi) {
                if let Tok::Ident(ref s) = tokens[j] {
                    reexported.push(s.as_str());
                }
                j += 1;
            }
            if in_public_non_internal_mod {
                for name in reexported {
                    if name == "SweepMode"
                        || name == "engine_internals"
                        || FORBIDDEN_RAW_FNS.contains(&name)
                    {
                        return Err(format!(
                            "{rel_path}: public `pub use` re-exports restricted symbol `{name}` outside `engine_internals`"
                        ));
                    }
                }
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check `[pub] [const|async|unsafe|extern "..."] fn <name>`.
        let mut fn_kw = after_vis;
        loop {
            match tokens.get(fn_kw) {
                Some(Tok::Ident(s))
                    if matches!(s.as_str(), "const" | "async" | "unsafe" | "extern") =>
                {
                    fn_kw += 1;
                    if s == "extern" && matches!(tokens.get(fn_kw), Some(Tok::Str(_))) {
                        fn_kw += 1;
                    }
                }
                _ => break,
            }
        }
        if matches!(tokens.get(fn_kw), Some(Tok::Ident(s)) if s == "fn")
            && let Some(Tok::Ident(fn_name)) = tokens.get(fn_kw + 1)
        {
            if matches!(vis, Visibility::Public) {
                if in_impl_turn_engine && FORBIDDEN_RAW_FNS.contains(&fn_name.as_str()) {
                    return Err(format!(
                        "{rel_path}: `TurnEngine::{fn_name}` must remain `pub(crate)`, never `pub` (even behind `engine-internals`, because `--workspace` feature unification would expose it)"
                    ));
                }
                if in_public_non_internal_mod && !in_engine_internals_mod {
                    visited_public_fns.push(fn_name.clone());
                    if FORBIDDEN_RAW_FNS.contains(&fn_name.as_str()) {
                        return Err(format!(
                            "{rel_path}: raw API `pub fn {fn_name}` must not be exposed as ordinary public API"
                        ));
                    }
                }
                if attrs.cfg_engine_internals && !in_engine_internals_mod {
                    return Err(format!(
                        "{rel_path}: `pub fn {fn_name}` uses `#[cfg(feature = \"engine-internals\")]` outside `engine_internals`"
                    ));
                }
            }
            let mut j = fn_kw + 2;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        // Check `pub enum SweepMode` / `pub struct SweepMode` / `pub type SweepMode`.
        if matches!(vis, Visibility::Public)
            && matches!(
                tokens.get(after_vis),
                Some(Tok::Ident(s)) if matches!(s.as_str(), "enum" | "struct" | "type")
            )
            && let Some(Tok::Ident(item_name)) = tokens.get(after_vis + 1)
        {
            if item_name == "SweepMode" && in_public_non_internal_mod && !in_engine_internals_mod {
                return Err(format!(
                    "{rel_path}: `SweepMode` must not be publicly reachable outside `engine_internals`"
                ));
            }
            let mut j = after_vis + 2;
            while j < tokens.len() && !matches!(tokens[j], Tok::LBrace | Tok::Semi) {
                j += 1;
            }
            if j < tokens.len() && matches!(tokens[j], Tok::LBrace) {
                scopes.push(ScopeKind::Block {
                    cfg_test: attrs.cfg_test,
                });
                attrs = AttrFlags::default();
                i = j + 1;
                continue;
            }
            attrs = AttrFlags::default();
            i = (j + 1).min(tokens.len());
            continue;
        }

        if !matches!(tokens[i], Tok::Ident(ref s) if s == "pub") {
            attrs = AttrFlags::default();
        }
        i += 1;
    }

    Ok((seen_engine_internals_mod, visited_public_fns))
}

fn scan_league_core_src_tree(src_root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_rs_files_recursive(src_root, &mut files);
    files.sort();

    let mut visited = Vec::with_capacity(files.len());
    let mut saw_engine_internals = false;
    let mut observation_checked = false;
    let mut saw_preview_what_if = false;
    let mut saw_preview_seat_bundle = false;

    for path in files {
        let rel = path
            .strip_prefix(src_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
            .unwrap_or_else(|| path.display().to_string());
        let source = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let is_lib_rs = rel == "lib.rs";
        let (has_ei, pub_fns) = scan_file_public_api(&rel, &source, is_lib_rs)?;
        if has_ei {
            saw_engine_internals = true;
        }
        if rel == "observation.rs" {
            observation_checked = true;
            saw_preview_what_if = pub_fns.iter().any(|f| f == "preview_what_if");
            saw_preview_seat_bundle = pub_fns.iter().any(|f| f == "preview_seat_bundle");
            let toks = tokenize_rust(&source);
            for w in toks.windows(2) {
                if matches!(&w[0], Tok::Ident(a) if a == "SweepMode")
                    && matches!(&w[1], Tok::Ident(b) if b == "Live")
                {
                    return Err("observation.rs references SweepMode::Live".to_owned());
                }
            }
        }
        visited.push(rel);
    }

    if !saw_engine_internals {
        return Err("lib.rs is missing `#[cfg(feature = \"engine-internals\")] #[doc(hidden)] pub mod engine_internals`".to_owned());
    }
    if !observation_checked || !saw_preview_what_if || !saw_preview_seat_bundle {
        return Err(
            "observation.rs must keep public `preview_what_if` and `preview_seat_bundle`"
                .to_owned(),
        );
    }

    Ok(visited)
}

fn check_workspace_manifests_and_crate_sources(
    workspace_crates_dir: &Path,
) -> Result<Vec<String>, String> {
    let entries = fs::read_dir(workspace_crates_dir)
        .map_err(|err| format!("failed to read {}: {err}", workspace_crates_dir.display()))?;
    let mut crate_dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("dir entry error: {err}"))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .map_err(|err| format!("file_type error {}: {err}", path.display()))?;
        if ft.is_dir() && path.join("Cargo.toml").is_file() {
            crate_dirs.push(path);
        }
    }
    crate_dirs.sort();

    let mut checked_crates = Vec::new();
    for crate_dir in crate_dirs {
        let crate_name = crate_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        let manifest = fs::read_to_string(crate_dir.join("Cargo.toml"))
            .map_err(|err| format!("failed to read {crate_name}/Cargo.toml: {err}"))?;

        if crate_name == "league-core" {
            let mut section = String::new();
            let mut saw_feature_def = false;
            for raw_line in manifest.lines() {
                let line = raw_line.split('#').next().unwrap_or("").trim();
                if line.starts_with('[') && line.ends_with(']') {
                    section = line.to_owned();
                    continue;
                }
                if section == "[features]" {
                    if line.starts_with("engine-internals") {
                        saw_feature_def = true;
                    }
                    if line.starts_with("default") && line.contains("engine-internals") {
                        return Err(
                            "league-core/Cargo.toml must not enable `engine-internals` in `default` features".to_owned(),
                        );
                    }
                } else if line.contains("engine-internals") && section != "[dev-dependencies]" {
                    return Err(format!(
                        "league-core/Cargo.toml enables `engine-internals` in unexpected section `{section}`"
                    ));
                }
            }
            if !saw_feature_def {
                return Err(
                    "league-core/Cargo.toml must define non-default feature `engine-internals = []`"
                        .to_owned(),
                );
            }
        } else if crate_name == "league-standalone" {
            if !manifest.contains("engine-internals") {
                return Err(
                    "league-standalone/Cargo.toml is expected to opt into `league-core` `engine-internals`".to_owned(),
                );
            }
        } else {
            for raw_line in manifest.lines() {
                let code_part = raw_line.split('#').next().unwrap_or("");
                if code_part.contains("engine-internals") {
                    return Err(format!(
                        "{crate_name}/Cargo.toml must not enable `engine-internals`"
                    ));
                }
                if crate_name == "league-relay" && code_part.contains("league-core") {
                    return Err(
                        "league-relay/Cargo.toml must not depend on `league-core`".to_owned()
                    );
                }
            }
            let mut rs_files = Vec::new();
            collect_rs_files_recursive(&crate_dir, &mut rs_files);
            rs_files.sort();
            for rs_path in rs_files {
                let src = fs::read_to_string(&rs_path)
                    .map_err(|err| format!("failed to read {}: {err}", rs_path.display()))?;
                for tok in tokenize_rust(&src) {
                    if let Tok::Ident(ident) = tok
                        && (ident == "engine_internals"
                            || ident == "SweepMode"
                            || FORBIDDEN_RAW_FNS.contains(&ident.as_str()))
                    {
                        return Err(format!(
                            "{} references forbidden engine-internal symbol `{ident}`",
                            rs_path.display()
                        ));
                    }
                }
            }
        }
        checked_crates.push(crate_name);
    }

    let seen: BTreeSet<&str> = checked_crates.iter().map(String::as_str).collect();
    for expected in [
        "league-core",
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
        "league-standalone",
        "league-tui",
    ] {
        if !seen.contains(expected) {
            return Err(format!(
                "expected workspace crate `{expected}` was not scanned"
            ));
        }
    }
    Ok(checked_crates)
}

fn write_minimal_clean_core_tree(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("lib.rs"),
        r#"
pub mod engine;
pub mod observation;
#[cfg(feature = "engine-internals")]
#[doc(hidden)]
pub mod engine_internals {
    pub use crate::engine::sweep_mode::SweepMode;
    pub fn private_hints_for_auditor() {}
    pub fn preview_bundle_live() {}
}
"#,
    )?;
    fs::write(
        root.join("engine.rs"),
        r#"
pub(crate) mod sweep_mode {
    pub enum SweepMode { Live, PreviewSteps0To7 }
}
pub struct TurnEngine;
impl TurnEngine {
    pub fn observe_seat(&self) {}
    pub(crate) fn private_hints_for_auditor(&self) {}
    pub(crate) fn preview_bundle_live(&self) {}
}
"#,
    )?;
    fs::write(
        root.join("observation.rs"),
        r#"
pub fn preview_what_if() {}
pub fn preview_seat_bundle() {}
"#,
    )?;
    Ok(())
}

fn write_minimal_workspace_tree(root: &Path) -> std::io::Result<()> {
    for name in [
        "league-core",
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
        "league-standalone",
        "league-tui",
    ] {
        fs::create_dir_all(root.join(name).join("src"))?;
        fs::write(root.join(name).join("src/lib.rs"), "// clean stub\n")?;
    }
    fs::write(
        root.join("league-core/Cargo.toml"),
        "[package]\nname = \"league-core\"\n[features]\nengine-internals = []\n[dev-dependencies]\nleague-core = { path = \".\", features = [\"engine-internals\"] }\n",
    )?;
    fs::write(
        root.join("league-standalone/Cargo.toml"),
        "[dependencies]\nleague-core = { path = \"../league-core\", features = [\"engine-internals\"] }\n",
    )?;
    for name in [
        "league-gen",
        "league-ledger",
        "league-relay",
        "league-season",
        "league-tui",
    ] {
        fs::write(
            root.join(name).join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n"),
        )?;
    }
    Ok(())
}

#[test]
fn raw_api_backdoors_stay_shut_and_seat_crates_cannot_enable_engine_internals() {
    let core_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = core_dir.join("src");
    let scanned_files = match scan_league_core_src_tree(&src_dir) {
        Ok(files) => files,
        Err(err) => panic!("raw API back-door guard failed on clean league-core/src: {err}"),
    };
    for expected in [
        "commit.rs",
        "contracts.rs",
        "engine.rs",
        "error.rs",
        "lib.rs",
        "observation.rs",
        "predicates.rs",
        "replay.rs",
        "scoring.rs",
        "types.rs",
    ] {
        assert!(
            scanned_files.iter().any(|f| f == expected),
            "expected {expected} in scanned files: {scanned_files:?}"
        );
    }

    let crates_dir = core_dir
        .parent()
        .unwrap_or_else(|| panic!("expected crates/ parent directory"));
    let checked_crates = match check_workspace_manifests_and_crate_sources(crates_dir) {
        Ok(crates) => crates,
        Err(err) => panic!("workspace engine-internals isolation check failed: {err}"),
    };
    assert!(checked_crates.contains(&"league-tui".to_owned()));
    assert!(checked_crates.contains(&"league-relay".to_owned()));

    assert_eq!(SweepMode::default(), SweepMode::PreviewSteps0To7);

    let tmp_root =
        std::env::temp_dir().join(format!("ml_raw_api_backdoor_guard_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp_root);

    // Control A: `pub fn private_hints_for_auditor` restored on `TurnEngine` -> Err.
    let case_a = tmp_root.join("case_a");
    if let Err(err) = write_minimal_clean_core_tree(&case_a) {
        panic!("setup case_a: {err}");
    }
    assert!(scan_league_core_src_tree(&case_a).is_ok());
    if let Err(err) = fs::write(
        case_a.join("engine.rs"),
        "pub struct TurnEngine;\nimpl TurnEngine {\n    pub fn private_hints_for_auditor(&self) {}\n}\n",
    ) {
        panic!("write case_a: {err}");
    }
    assert!(scan_league_core_src_tree(&case_a).is_err());

    // Control B: `pub const fn preview_bundle` restored on `TurnEngine` -> Err.
    let case_b = tmp_root.join("case_b");
    if let Err(err) = write_minimal_clean_core_tree(&case_b) {
        panic!("setup case_b: {err}");
    }
    if let Err(err) = fs::write(
        case_b.join("engine.rs"),
        "pub struct TurnEngine;\nimpl TurnEngine {\n    pub const fn preview_bundle(&self) {}\n}\n",
    ) {
        panic!("write case_b: {err}");
    }
    assert!(scan_league_core_src_tree(&case_b).is_err());

    // Control C: `#[cfg(feature = "engine-internals")] pub fn preview_bundle` on `TurnEngine` -> Err.
    let case_c = tmp_root.join("case_c");
    if let Err(err) = write_minimal_clean_core_tree(&case_c) {
        panic!("setup case_c: {err}");
    }
    if let Err(err) = fs::write(
        case_c.join("engine.rs"),
        "pub struct TurnEngine;\nimpl TurnEngine {\n    #[cfg(feature = \"engine-internals\")]\n    pub fn preview_bundle(&self) {}\n}\n",
    ) {
        panic!("write case_c: {err}");
    }
    assert!(scan_league_core_src_tree(&case_c).is_err());

    // Control D: `pub mod engine_internals` missing `#[cfg]` or `#[doc(hidden)]` -> Err.
    let case_d = tmp_root.join("case_d");
    if let Err(err) = write_minimal_clean_core_tree(&case_d) {
        panic!("setup case_d: {err}");
    }
    if let Err(err) = fs::write(
        case_d.join("lib.rs"),
        "pub mod engine;\npub mod observation;\npub mod engine_internals {}\n",
    ) {
        panic!("write case_d: {err}");
    }
    assert!(scan_league_core_src_tree(&case_d).is_err());

    // Control E: `pub use engine::sweep_mode::SweepMode;` at crate root -> Err.
    let case_e = tmp_root.join("case_e");
    if let Err(err) = write_minimal_clean_core_tree(&case_e) {
        panic!("setup case_e: {err}");
    }
    if let Err(err) = fs::write(
        case_e.join("lib.rs"),
        "pub mod engine;\npub mod observation;\npub use engine::sweep_mode::SweepMode;\n#[cfg(feature = \"engine-internals\")]\n#[doc(hidden)]\npub mod engine_internals {}\n",
    ) {
        panic!("write case_e: {err}");
    }
    assert!(scan_league_core_src_tree(&case_e).is_err());

    // Control F: `default = ["engine-internals"]` in `league-core/Cargo.toml` -> Err.
    let case_f = tmp_root.join("case_f");
    if let Err(err) = write_minimal_workspace_tree(&case_f) {
        panic!("setup case_f: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_f).is_ok());
    if let Err(err) = fs::write(
        case_f.join("league-core/Cargo.toml"),
        "[package]\nname = \"league-core\"\n[features]\ndefault = [\"engine-internals\"]\nengine-internals = []\n",
    ) {
        panic!("write case_f: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_f).is_err());

    // Control G: `league-tui/Cargo.toml` enables `engine-internals` or `league-relay` adds `league-core` -> Err.
    let case_g1 = tmp_root.join("case_g1");
    if let Err(err) = write_minimal_workspace_tree(&case_g1) {
        panic!("setup case_g1: {err}");
    }
    if let Err(err) = fs::write(
        case_g1.join("league-tui/Cargo.toml"),
        "[dependencies]\nleague-core = { path = \"../league-core\", features = [\"engine-internals\"] }\n",
    ) {
        panic!("write case_g1: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_g1).is_err());

    let case_g2 = tmp_root.join("case_g2");
    if let Err(err) = write_minimal_workspace_tree(&case_g2) {
        panic!("setup case_g2: {err}");
    }
    if let Err(err) = fs::write(
        case_g2.join("league-relay/Cargo.toml"),
        "[dependencies]\nleague-core = { path = \"../league-core\" }\n",
    ) {
        panic!("write case_g2: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_g2).is_err());

    // Control H: `league-tui/src/lib.rs` imports `engine_internals` -> Err; comment near-miss -> Ok.
    let case_h = tmp_root.join("case_h");
    if let Err(err) = write_minimal_workspace_tree(&case_h) {
        panic!("setup case_h: {err}");
    }
    if let Err(err) = fs::write(
        case_h.join("league-tui/src/lib.rs"),
        "// Note: engine_internals and preview_bundle stay out of comments\npub fn ok() {}\n",
    ) {
        panic!("write case_h ok: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_h).is_ok());

    if let Err(err) = fs::write(
        case_h.join("league-tui/src/lib.rs"),
        "use league_core::engine_internals;\n",
    ) {
        panic!("write case_h bad: {err}");
    }
    assert!(check_workspace_manifests_and_crate_sources(&case_h).is_err());

    let _ = fs::remove_dir_all(&tmp_root);
}

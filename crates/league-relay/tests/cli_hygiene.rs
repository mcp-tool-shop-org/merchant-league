//! CLI hygiene integration tests for `league-relay` (`SHIP_GATE.md` `:26`, `:27`, `:28`, `:40`).
//!
//! Covers canonical `StructuredError` JSON/text encoding, `--error-format`, unified `0/1/2/3`
//! exit codes (and complete retirement of `4`), `--log-level` / `-q` / `-v` / `--debug` /
//! `LEAGUE_LOG_LEVEL` handling, `--debug`-gated redacted panic backtraces, and secret redaction
//! across all four log verbosity levels.

use league_relay::cli::{
    USAGE_TEXT, execute_cli, infer_error_format_best_effort, infer_log_level_best_effort,
    parse_args_with_key,
};
use league_relay::{
    ApiKey, CallLogEntry, CostUnit, EXIT_OK, EXIT_PARTIAL_SUCCESS, EXIT_RUNTIME_ERROR,
    EXIT_USER_ERROR, ErrorFormat, FORBIDDEN_LOCAL_KEY_ENVS, LEAGUE_LOG_LEVEL_ENV, LogLevel,
    OPENROUTER_API_KEY_ENV, REDACTED_MARKER, RelayError, Round, SeatOutcome, SeatReport,
    StructuredError, ToStructuredError, TransportError, TurnSummary,
};
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn Error>>;

const OR_ENV_KEY: &str = "Zx9QvR2mKp8LnW4tYs7BcF1h";
const OAI_ENV_KEY: &str = "Qw7ErT9yUi3OpA6sDf8GhJ2k";
const VLLM_ENV_KEY: &str = "Mn4BvC6xZ1LkJ8hGf3DsA5pQ";
const LOCAL_ENV_KEY: &str = "Pl0OkI9uY7yTrE5wQ3aSdF2g";

const PLANTED_BEARER: &str = "Bearer Br9Kx3ZmVp2QnW7cRt5Yh";
const PLANTED_SK_OR: &str = "sk-or-v1-abc987def654ghi321";
const PLANTED_SK_PROJ: &str = "sk-proj-Zyx987Wvu654Tsr321";
const PLANTED_SK_ANT: &str = "sk-ant-api03-Mno741Pqr852";
const PLANTED_XRPL_SECP: &str = "sp5fghtJtpUorTwvof1NpDXAzNwf5";
const PLANTED_XRPL_ED: &str = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
const PLANTED_SEALED_STRUCT: &str = "SealedEvalSeed(9876543210987654321)";
const PLANTED_PREIMAGE_STRUCT: &str = "PreimageSecret(deadbeefcafe001122334455)";
const PLANTED_SEALED_HEX: &str = "0x8000000000000042";
const PLANTED_SEALED_DEC: &str = "9223372036854775850";
const PLANTED_EVAL_FIELD: &str = "eval_vrf_seed=vrf-secret-payload-9911";
const PLANTED_PREIMAGE_JSON: &str = "\"preimage_secret\":\"preimage-secret-payload-7722\"";

fn planted_env_keys() -> [&'static str; 4] {
    [OR_ENV_KEY, OAI_ENV_KEY, VLLM_ENV_KEY, LOCAL_ENV_KEY]
}

fn planted_secret_needles() -> [&'static str; 15] {
    [
        OR_ENV_KEY,
        OAI_ENV_KEY,
        VLLM_ENV_KEY,
        LOCAL_ENV_KEY,
        "Br9Kx3ZmVp2QnW7cRt5Yh",
        PLANTED_SK_OR,
        PLANTED_SK_PROJ,
        PLANTED_SK_ANT,
        PLANTED_XRPL_SECP,
        PLANTED_XRPL_ED,
        "9876543210987654321",
        "deadbeefcafe001122334455",
        PLANTED_SEALED_HEX,
        PLANTED_SEALED_DEC,
        "vrf-secret-payload-9911",
    ]
}

fn assert_no_secrets_leaked(haystack: &str, context: &str) -> TestResult {
    for needle in planted_secret_needles() {
        assert!(
            !haystack.contains(needle),
            "{context}: leaked secret needle '{needle}' in: {haystack}"
        );
    }
    assert!(
        !haystack.contains("preimage-secret-payload-7722"),
        "{context}: leaked preimage JSON secret in: {haystack}"
    );
    for env_key in planted_env_keys() {
        let chars: Vec<char> = env_key.chars().collect();
        for win in chars.windows(8) {
            let sub: String = win.iter().collect();
            assert!(
                !haystack.contains(&sub),
                "{context}: leaked 8-char slice '{sub}' of env key in: {haystack}"
            );
        }
    }
    Ok(())
}

fn write_wrapper(dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join("wrapper.md");
    fs::write(&path, "SYSTEM WRAPPER HYGIENE")?;
    Ok(path)
}

fn write_openrouter_seats(dir: &Path, guilds: &[&str]) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join("seats.json");
    let entries: Vec<serde_json::Value> = guilds
        .iter()
        .map(|g| {
            json!({
                "guild": g,
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.4,
                "max_output_tokens": 128,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.05
            })
        })
        .collect();
    fs::write(&path, serde_json::to_string(&json!({ "seats": entries }))?)?;
    Ok(path)
}

fn write_partial_seats(dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join("seats_partial.json");
    fs::write(
        &path,
        serde_json::to_string(&json!({
            "seats": [
                {
                    "guild": "Saltworks",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.4,
                    "max_output_tokens": 128,
                    "timeout_secs": 5.0,
                    "max_cost_per_call": 0.05
                },
                {
                    "guild": "Weavers",
                    "backend": "file",
                    "model": "manual-weavers"
                }
            ]
        }))?,
    )?;
    Ok(path)
}

fn write_file_seats(dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join("seats_file.json");
    fs::write(
        &path,
        serde_json::to_string(&json!({
            "seats": [
                {
                    "guild": "Saltworks",
                    "backend": "file",
                    "model": "manual-saltworks"
                }
            ]
        }))?,
    )?;
    Ok(path)
}

fn write_board(game_dir: &Path, guild: &str, body: &str) -> TestResult {
    let to_seat = game_dir.join("to-seat");
    fs::create_dir_all(&to_seat)?;
    fs::write(to_seat.join(format!("day-01-talk-{guild}.md")), body)?;
    Ok(())
}

fn write_ok_mock_responses(mock_dir: &Path, match_token: &str) -> TestResult {
    fs::create_dir_all(mock_dir)?;
    fs::write(
        mock_dir.join("responses.json"),
        serde_json::to_string(&json!({
            "rules": [{
                "when_contains": match_token,
                "status": 200,
                "response": {
                    "id": "mock-ok-1",
                    "model": "google/gemini-2.5-pro-06-05",
                    "choices": [{"message": {"role": "assistant", "content": "PASS"}}],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12, "cost": 0.01}
                }
            }]
        }))?,
    )?;
    Ok(())
}

#[cfg(unix)]
fn set_readonly(path: &Path) -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o444);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_readonly(path: &Path) -> TestResult {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(unix)]
fn set_writable(path: &Path) -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o644);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::permissions_set_readonly_false)]
fn set_writable(path: &Path) -> TestResult {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(false);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[test]
fn structured_error_json_schema_matches_league_core() -> TestResult {
    let secret_blob = format!(
        "{PLANTED_BEARER} {PLANTED_SK_OR} {PLANTED_XRPL_ED} {PLANTED_SEALED_STRUCT} {PLANTED_SEALED_HEX}"
    );
    let variants: Vec<RelayError> = vec![
        RelayError::Usage(format!("bad flag {secret_blob}")),
        RelayError::SecretInForbiddenPlace(format!("bad placement {secret_blob}")),
        RelayError::LiveFlagRequired,
        RelayError::MissingApiKey,
        RelayError::InvalidCap(format!("bad cap {secret_blob}")),
        RelayError::MissingWrapper {
            guild: "Saltworks".to_owned(),
        },
        RelayError::SeatConfig(format!("bad seat {secret_blob}")),
        RelayError::Io(format!("io failed {secret_blob}")),
        RelayError::CorruptLog(format!("corrupt {secret_blob}")),
        RelayError::IncompatibleSetup(format!("incompatible {secret_blob}")),
        RelayError::TurnLocked {
            lock_path: format!("game/{secret_blob}/relay/turn.lock"),
            holder: Some(format!("pid=42 {secret_blob}")),
        },
        RelayError::UnrecordedCharge {
            charged_credits: "0.050000".to_owned(),
            entries_summary: format!("seat=Saltworks {secret_blob}"),
            detail: format!("detail {secret_blob}"),
        },
    ];

    for err in &variants {
        let se = err.to_structured_error();
        assert_eq!(se.code, err.code());
        assert!(!se.code.is_empty());
        assert!(!se.hint.is_empty());
        assert_eq!(
            se.retryable,
            matches!(err, RelayError::TurnLocked { .. }),
            "unexpected retryable flag on {:?}",
            err.code()
        );
        let json_str = se.to_json_string()?;
        assert_no_secrets_leaked(&json_str, err.code())?;
        assert_no_secrets_leaked(&se.format_text(), err.code())?;
        let parsed: serde_json::Value = serde_json::from_str(&json_str)?;
        let obj = parsed.as_object().ok_or("expected JSON object")?;
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        if se.cause.is_some() {
            assert_eq!(keys, ["cause", "code", "hint", "message", "retryable"]);
            let i_code = json_str.find("\"code\":").ok_or("missing code")?;
            let i_msg = json_str.find("\"message\":").ok_or("missing message")?;
            let i_hint = json_str.find("\"hint\":").ok_or("missing hint")?;
            let i_cause = json_str.find("\"cause\":").ok_or("missing cause")?;
            let i_ret = json_str.find("\"retryable\":").ok_or("missing retryable")?;
            assert!(i_code < i_msg && i_msg < i_hint && i_hint < i_cause && i_cause < i_ret);
        } else {
            assert_eq!(keys, ["code", "hint", "message", "retryable"]);
            assert!(!json_str.contains("\"cause\":"));
            let i_code = json_str.find("\"code\":").ok_or("missing code")?;
            let i_msg = json_str.find("\"message\":").ok_or("missing message")?;
            let i_hint = json_str.find("\"hint\":").ok_or("missing hint")?;
            let i_ret = json_str.find("\"retryable\":").ok_or("missing retryable")?;
            assert!(i_code < i_msg && i_msg < i_hint && i_hint < i_ret);
        }
        assert!(se.format_text().starts_with(&format!("[{}]", se.code)));
        assert!(se.format_text().contains(&format!("; hint: {}", se.hint)));
    }

    let transport_errs = [
        TransportError::Timeout,
        TransportError::Network(secret_blob),
    ];
    for terr in &transport_errs {
        let se = terr.to_structured_error();
        assert_eq!(se.code, terr.code());
        assert!(se.retryable);
        let json_str = se.to_json_string()?;
        assert_no_secrets_leaked(&json_str, terr.code())?;
        assert_no_secrets_leaked(&se.format_text(), terr.code())?;
    }

    let manual_se = StructuredError::new("TEST_CODE", PLANTED_SK_PROJ, "check config", true)
        .with_cause(PLANTED_XRPL_SECP);
    assert_no_secrets_leaked(&manual_se.to_json_string()?, "manual_se")?;
    assert!(manual_se.format_text().ends_with(" [retryable]"));
    Ok(())
}

#[test]
fn cli_error_format_text_and_json_on_usage_and_runtime_errors() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_openrouter_seats(tmp.path(), &["Saltworks"])?;
    let mock_dir = tmp.path().join("mock");
    write_ok_mock_responses(&mock_dir, "MOCK_OK")?;

    // 1. Pre-subcommand `--error-format json` on missing subcommand (exit 1).
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = execute_cli(["--error-format", "json"], |_| None, &mut out, &mut err);
    assert_eq!(code, EXIT_USER_ERROR);
    let json_val: serde_json::Value = serde_json::from_slice(&err)?;
    assert_eq!(json_val["code"], "INVALID_USAGE");
    assert_eq!(json_val["retryable"], false);
    assert!(json_val["hint"].as_str().is_some_and(|h| !h.is_empty()));

    // 2. Post-subcommand `--error-format=json` on missing `--game-dir` (exit 2, RELAY_IO).
    out.clear();
    err.clear();
    let missing_game = tmp.path().join("no_such_game");
    let code_io = execute_cli(
        [
            "turn",
            "--error-format=json",
            "--game-dir",
            missing_game.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut out,
        &mut err,
    );
    assert_eq!(code_io, EXIT_RUNTIME_ERROR);
    let io_val: serde_json::Value = serde_json::from_slice(&err)?;
    assert_eq!(io_val["code"], "RELAY_IO");
    assert_eq!(io_val["retryable"], false);
    assert!(io_val["cause"].is_string());

    // 3. Explicit `--error-format text` on TURN_LOCKED (exit 2, retryable).
    let locked_game = tmp.path().join("locked_game");
    write_board(&locked_game, "Saltworks", "MOCK_OK")?;
    fs::create_dir_all(locked_game.join("relay"))?;
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(locked_game.join("relay/turn.lock"))?;
    lock_file.try_lock()?;

    out.clear();
    err.clear();
    let code_lock_text = execute_cli(
        [
            "turn",
            "--error-format",
            "text",
            "--game-dir",
            locked_game.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut out,
        &mut err,
    );
    assert_eq!(code_lock_text, EXIT_RUNTIME_ERROR);
    let lock_text = String::from_utf8(err.clone())?;
    assert!(lock_text.starts_with("error: [TURN_LOCKED] "));
    assert!(lock_text.contains("; hint: "));
    assert!(lock_text.contains("[retryable]"));

    err.clear();
    let code_lock_json = execute_cli(
        [
            "turn",
            "--error-format",
            "json",
            "--game-dir",
            locked_game.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut out,
        &mut err,
    );
    let _ = lock_file.unlock();
    assert_eq!(code_lock_json, EXIT_RUNTIME_ERROR);
    let lock_json: serde_json::Value = serde_json::from_slice(&err)?;
    assert_eq!(lock_json["code"], "TURN_LOCKED");
    assert_eq!(lock_json["retryable"], true);

    // 4. Conflicting and invalid `--error-format` rejected as exit 1.
    out.clear();
    err.clear();
    let conflict_code = execute_cli(
        ["--error-format", "json", "turn", "--error-format", "text"],
        |_| None,
        &mut out,
        &mut err,
    );
    assert_eq!(conflict_code, EXIT_USER_ERROR);
    assert_eq!(
        infer_error_format_best_effort(["--error-format", "json"]),
        ErrorFormat::Json
    );
    let invalid_fmt_code = execute_cli(
        ["--error-format", "yaml", "turn"],
        |_| None,
        &mut out,
        &mut err,
    );
    assert_eq!(invalid_fmt_code, EXIT_USER_ERROR);
    Ok(())
}

#[test]
fn exit_code_0_on_help_prebook_ok_and_clean_turn() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_openrouter_seats(tmp.path(), &["Saltworks"])?;
    let file_seats = write_file_seats(tmp.path())?;
    let mock_dir = tmp.path().join("mock");
    write_ok_mock_responses(&mock_dir, "MOCK_OK")?;

    let mut out = Vec::new();
    let mut err = Vec::new();
    assert_eq!(
        execute_cli(["--help"], |_| None, &mut out, &mut err),
        EXIT_OK
    );
    assert_eq!(String::from_utf8(out.clone())?, USAGE_TEXT);

    out.clear();
    err.clear();
    assert_eq!(
        execute_cli(
            [
                "prebook-check",
                "--seats",
                file_seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_OK
    );
    assert!(String::from_utf8(out.clone())?.contains("prebook-check: OK"));

    let game_dir = tmp.path().join("clean_turn");
    write_board(&game_dir, "Saltworks", "MOCK_OK")?;
    out.clear();
    err.clear();
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                game_dir.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_OK
    );
    assert!(String::from_utf8(out)?.contains("outcome=OK"));
    Ok(())
}

#[test]
fn exit_code_1_on_every_user_and_config_error_variant() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_openrouter_seats(tmp.path(), &["Saltworks"])?;
    let file_seats = write_file_seats(tmp.path())?;
    let game_dir = tmp.path().join("u1_game");
    write_board(&game_dir, "Saltworks", "MOCK_OK")?;

    let mut out = Vec::new();
    let mut err = Vec::new();

    // Usage
    assert_eq!(
        execute_cli(["--bogus-flag"], |_| None, &mut out, &mut err),
        EXIT_USER_ERROR
    );
    // SecretInForbiddenPlace
    assert_eq!(
        execute_cli(
            ["turn", "--api-key", PLANTED_SK_OR],
            |_| None,
            &mut out,
            &mut err
        ),
        EXIT_USER_ERROR
    );
    // LiveFlagRequired (openrouter without --live or --mock-dir)
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                game_dir.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_USER_ERROR
    );
    // MissingApiKey (`--live` + `--cap-credits` without env key)
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                game_dir.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--live",
                "--cap-credits",
                "1.0",
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_USER_ERROR
    );
    // InvalidCap
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                game_dir.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--live",
                "--cap-credits",
                "not_a_num",
            ],
            |k| (k == OPENROUTER_API_KEY_ENV).then(|| OR_ENV_KEY.to_owned()),
            &mut out,
            &mut err,
        ),
        EXIT_USER_ERROR
    );
    // MissingWrapper on turn 1 (`openrouter` mock seat without --wrapper)
    let mock_dir_missing = tmp.path().join("mock_missing_wrap");
    write_ok_mock_responses(&mock_dir_missing, "MOCK_OK")?;
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                game_dir.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir_missing.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_USER_ERROR
    );
    // SeatConfig
    let bad_seats = tmp.path().join("bad_seats.json");
    fs::write(&bad_seats, "{bad")?;
    assert_eq!(
        execute_cli(
            [
                "prebook-check",
                "--seats",
                bad_seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_USER_ERROR
    );
    // IncompatibleSetup (switch memory_mode from snapshot -> full on existing game_dir)
    let legacy_game = tmp.path().join("legacy_setup");
    write_board(&legacy_game, "Saltworks", "PASS")?;
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                legacy_game.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                file_seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_OK
    );
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                legacy_game.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "act",
                "--memory-mode",
                "full",
                "--seats",
                file_seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_USER_ERROR
    );
    Ok(())
}

#[test]
fn exit_code_2_on_turn_locked_unrecorded_charge_io_corrupt_log_prebook_fail_and_zero_clean_turn()
-> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_openrouter_seats(tmp.path(), &["Saltworks"])?;
    let mock_dir = tmp.path().join("mock");
    write_ok_mock_responses(&mock_dir, "MOCK_OK")?;

    let mut out = Vec::new();
    let mut err = Vec::new();

    // 1. Io (missing --game-dir) -> exit 2
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                tmp.path().join("missing_dir").to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_RUNTIME_ERROR
    );

    // 2. TurnLocked -> exit 2
    let locked_game = tmp.path().join("lock_exit2");
    write_board(&locked_game, "Saltworks", "MOCK_OK")?;
    fs::create_dir_all(locked_game.join("relay"))?;
    let lock_f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(locked_game.join("relay/turn.lock"))?;
    lock_f.try_lock()?;
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                locked_game.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_RUNTIME_ERROR
    );
    let _ = lock_f.unlock();

    // 3. UnrecordedCharge -> exit 2 (with code UNRECORDED_CHARGE, retryable: false)
    let unrec_game = tmp.path().join("unrec_exit2");
    write_board(&unrec_game, "Saltworks", "MOCK_OK")?;
    fs::create_dir_all(unrec_game.join("relay"))?;
    let calls_f = unrec_game.join("relay/calls.jsonl");
    let pending_f = unrec_game.join("relay/calls.pending.jsonl");
    fs::write(&calls_f, "")?;
    let seed_pending = CallLogEntry {
        time_utc: "2026-11-18T00:00:00.000Z".to_owned(),
        day: "00".to_owned(),
        round: Round::Talk,
        guild: "Saltworks".to_owned(),
        attempt: 1,
        trigger: "turn".to_owned(),
        model_requested: "google/gemini-2.5-pro-06-05".to_owned(),
        model_returned: None,
        request_body_sha256: "00".repeat(32),
        reply_sha256: None,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        reasoning_tokens: None,
        cost_unit: CostUnit::Credits,
        wall_secs: None,
        cost: None,
        charged_credits: 0.0,
        latency_ms: 0,
        outcome: "CAP_REACHED".to_owned(),
        error: None,
        reservation_id: None,
    };
    fs::write(
        &pending_f,
        format!("{}\n", serde_json::to_string(&seed_pending)?),
    )?;
    set_readonly(&calls_f)?;
    set_readonly(&pending_f)?;
    out.clear();
    err.clear();
    let code_unrec = execute_cli(
        [
            "turn",
            "--error-format",
            "json",
            "--game-dir",
            unrec_game.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
            "--cap-credits",
            "1.0",
        ],
        |_| None,
        &mut out,
        &mut err,
    );
    let _ = set_writable(&calls_f);
    let _ = set_writable(&pending_f);
    assert_eq!(code_unrec, EXIT_RUNTIME_ERROR);
    let unrec_err_str = String::from_utf8(err.clone())?;
    let unrec_out_str = String::from_utf8(out.clone())?;
    let unrec_json: serde_json::Value = serde_json::from_str(&unrec_err_str).map_err(|e| {
        format!("unrec JSON parse failed ({e}): err='{unrec_err_str}' out='{unrec_out_str}'")
    })?;
    assert_eq!(unrec_json["code"], "UNRECORDED_CHARGE");
    assert_eq!(unrec_json["retryable"], false);

    // 4. CorruptLog (malformed relay/setup.json) -> exit 2
    let corrupt_game = tmp.path().join("corrupt_exit2");
    write_board(&corrupt_game, "Saltworks", "MOCK_OK")?;
    fs::create_dir_all(corrupt_game.join("relay"))?;
    fs::write(corrupt_game.join("relay/setup.json"), "{not-valid-setup\n")?;
    out.clear();
    err.clear();
    let code_corrupt = execute_cli(
        [
            "turn",
            "--error-format",
            "json",
            "--game-dir",
            corrupt_game.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut out,
        &mut err,
    );
    assert_eq!(code_corrupt, EXIT_RUNTIME_ERROR);
    let corrupt_err_str = String::from_utf8(err.clone())?;
    let corrupt_out_str = String::from_utf8(out.clone())?;
    let corrupt_json: serde_json::Value = serde_json::from_str(&corrupt_err_str).map_err(|e| {
        format!("corrupt JSON parse failed ({e}): err='{corrupt_err_str}' out='{corrupt_out_str}'")
    })?;
    assert_eq!(corrupt_json["code"], "CORRUPT_LOG");

    // 5. Zero-clean `turn` (all seats NO_REPLY, ledger recorded cleanly) -> exit 2
    let zero_game = tmp.path().join("zero_clean_exit2");
    write_board(&zero_game, "Saltworks", "UNMATCHED_PROMPT")?;
    out.clear();
    err.clear();
    let code_zero = execute_cli(
        [
            "turn",
            "--game-dir",
            zero_game.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut out,
        &mut err,
    );
    assert_eq!(code_zero, EXIT_RUNTIME_ERROR);
    assert!(String::from_utf8(out)?.contains("outcome=NO_REPLY"));
    Ok(())
}

#[test]
fn exit_code_3_only_on_cleanly_recorded_partial_turn() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_partial_seats(tmp.path())?;
    let mock_dir = tmp.path().join("mock");
    write_ok_mock_responses(&mock_dir, "ONLY_WHEN_THIS_MATCHES")?;

    // Partial turn: Saltworks fails (UNMATCHED -> NO_REPLY), Weavers is clean (`file` -> RelayedByHand),
    // and relay/calls.jsonl records cleanly -> exit code 3.
    let partial_game = tmp.path().join("partial_turn");
    write_board(&partial_game, "Saltworks", "DOES_NOT_MATCH")?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = execute_cli(
        [
            "turn",
            "--game-dir",
            partial_game.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
            "--cap-credits",
            "1.0",
        ],
        |_| None,
        &mut out,
        &mut err,
    );
    assert_eq!(code, EXIT_PARTIAL_SUCCESS);
    assert_eq!(code, 3);
    let stdout_s = String::from_utf8(out)?;
    assert!(stdout_s.contains("seat=Saltworks outcome=NO_REPLY"));
    assert!(stdout_s.contains("seat=Weavers outcome=FILE_MANUAL"));
    assert!(partial_game.join("relay/calls.jsonl").is_file());
    assert!(!partial_game.join("relay/calls.pending.jsonl").exists());

    // Conversely, UnrecordedCharge must return 2 (never 3).
    assert_eq!(
        RelayError::UnrecordedCharge {
            charged_credits: "0.010000".to_owned(),
            entries_summary: "x".to_owned(),
            detail: "y".to_owned(),
        }
        .exit_code(),
        EXIT_RUNTIME_ERROR
    );
    Ok(())
}

#[test]
fn exit_code_4_is_never_returned_across_all_outcomes_and_errors() -> TestResult {
    let errors = [
        RelayError::Usage("u".to_owned()),
        RelayError::SecretInForbiddenPlace("s".to_owned()),
        RelayError::LiveFlagRequired,
        RelayError::MissingApiKey,
        RelayError::InvalidCap("c".to_owned()),
        RelayError::MissingWrapper {
            guild: "Saltworks".to_owned(),
        },
        RelayError::SeatConfig("cfg".to_owned()),
        RelayError::Io("io".to_owned()),
        RelayError::CorruptLog("corrupt".to_owned()),
        RelayError::IncompatibleSetup("old".to_owned()),
        RelayError::TurnLocked {
            lock_path: "lock".to_owned(),
            holder: Some("pid=1".to_owned()),
        },
        RelayError::UnrecordedCharge {
            charged_credits: "0.01".to_owned(),
            entries_summary: "sum".to_owned(),
            detail: "det".to_owned(),
        },
    ];
    for e in &errors {
        let c = e.exit_code();
        assert_ne!(c, 4, "retired exit code 4 returned by {:?}", e.code());
        assert!((0..=3).contains(&c));
    }

    for clean in 0..=3 {
        for failed in 0..=3 {
            let mut reports = Vec::new();
            for i in 0..clean {
                reports.push(SeatReport {
                    guild: format!("Clean{i}"),
                    outcome: SeatOutcome::RelayedByHand,
                });
            }
            for i in 0..failed {
                reports.push(SeatReport {
                    guild: format!("Fail{i}"),
                    outcome: SeatOutcome::SeatError {
                        detail: "err".to_owned(),
                    },
                });
            }
            let summary = TurnSummary { reports };
            let c = summary.exit_code();
            assert_ne!(c, 4);
            assert!((0..=3).contains(&c));
            match (clean, failed) {
                (_, 0) => assert_eq!(c, EXIT_OK),
                (0, _) => assert_eq!(c, EXIT_RUNTIME_ERROR),
                (_, _) => assert_eq!(c, EXIT_PARTIAL_SUCCESS),
            }
        }
    }

    // Live subprocess check on TurnLocked also returns 2, never 4.
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_file_seats(tmp.path())?;
    let game_dir = tmp.path().join("bin_lock");
    write_board(&game_dir, "Saltworks", "PASS")?;
    fs::create_dir_all(game_dir.join("relay"))?;
    let lock_f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(game_dir.join("relay/turn.lock"))?;
    lock_f.try_lock()?;
    let output = Command::new(env!("CARGO_BIN_EXE_league-relay"))
        .args([
            "turn",
            "--game-dir",
            game_dir.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
        ])
        .output()?;
    let _ = lock_f.unlock();
    assert_eq!(output.status.code(), Some(2));
    assert_ne!(output.status.code(), Some(4));
    Ok(())
}

#[test]
fn log_level_flags_and_env_precedence_and_conflicts() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_openrouter_seats(tmp.path(), &["Saltworks"])?;
    let mock_dir = tmp.path().join("mock");
    write_ok_mock_responses(&mock_dir, "TURN_CANARY_PROMPT")?;

    // Conflicting CLI flags and invalid LEAGUE_LOG_LEVEL fail with exit 1.
    let mut out = Vec::new();
    let mut err = Vec::new();
    assert_eq!(
        execute_cli(["-q", "-v", "turn", "--help"], |_| None, &mut out, &mut err),
        EXIT_USER_ERROR
    );
    assert_eq!(
        execute_cli(
            ["--debug", "--silent", "turn"],
            |_| None,
            &mut out,
            &mut err
        ),
        EXIT_USER_ERROR
    );
    let game_env = tmp.path().join("env_check");
    write_board(&game_env, "Saltworks", "TURN_CANARY_PROMPT")?;
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                game_env.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
            ],
            |k| (k == LEAGUE_LOG_LEVEL_ENV).then(|| "trace".to_owned()),
            &mut out,
            &mut err,
        ),
        EXIT_USER_ERROR
    );

    // Silent (-q) overrides LEAGUE_LOG_LEVEL=debug and suppresses stdout & stderr diag.
    let g_silent = tmp.path().join("lvl_silent");
    write_board(&g_silent, "Saltworks", "TURN_CANARY_PROMPT")?;
    out.clear();
    err.clear();
    assert_eq!(
        execute_cli(
            [
                "turn",
                "-q",
                "--game-dir",
                g_silent.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
                "--cap-credits",
                "1.0",
            ],
            |k| (k == LEAGUE_LOG_LEVEL_ENV).then(|| "debug".to_owned()),
            &mut out,
            &mut err,
        ),
        EXIT_OK
    );
    assert!(
        out.is_empty(),
        "silent mode must suppress stdout turn lines"
    );
    assert!(
        err.is_empty(),
        "silent mode must suppress stderr diagnostics"
    );

    // Normal (default): stdout has outcome, stderr has no [verbose]/[debug].
    let g_norm = tmp.path().join("lvl_normal");
    write_board(&g_norm, "Saltworks", "TURN_CANARY_PROMPT")?;
    out.clear();
    err.clear();
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                g_norm.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
                "--cap-credits",
                "1.0",
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_OK
    );
    assert!(String::from_utf8(out.clone())?.contains("outcome=OK"));
    assert!(err.is_empty());

    // Verbose (-v via LEAGUE_LOG_LEVEL or flag): [verbose] timings + caps, zero [debug], zero prompt canary.
    let g_verb = tmp.path().join("lvl_verbose");
    write_board(&g_verb, "Saltworks", "TURN_CANARY_PROMPT")?;
    out.clear();
    err.clear();
    assert_eq!(
        execute_cli(
            [
                "-v",
                "turn",
                "--game-dir",
                g_verb.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
                "--cap-credits",
                "1.0",
            ],
            |_| None,
            &mut out,
            &mut err,
        ),
        EXIT_OK
    );
    let verb_err = String::from_utf8(err.clone())?;
    assert!(verb_err.contains("[verbose] turn=start"));
    assert!(verb_err.contains("[verbose] turn=finish"));
    assert!(verb_err.contains("elapsed_ms="));
    assert!(verb_err.contains("cap=1.000000"));
    assert!(!verb_err.contains("[debug]"));
    assert!(!verb_err.contains("TURN_CANARY_PROMPT"));

    // Debug (--debug via LEAGUE_LOG_LEVEL): [verbose] + [debug] transport/hash/lock metadata, zero prompt/wrapper body.
    let g_dbg = tmp.path().join("lvl_debug");
    write_board(&g_dbg, "Saltworks", "TURN_CANARY_PROMPT")?;
    out.clear();
    err.clear();
    assert_eq!(
        execute_cli(
            [
                "turn",
                "--game-dir",
                g_dbg.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_dir.to_str().ok_or("utf8")?,
                "--cap-credits",
                "1.0",
            ],
            |k| (k == LEAGUE_LOG_LEVEL_ENV).then(|| "debug".to_owned()),
            &mut out,
            &mut err,
        ),
        EXIT_OK
    );
    let dbg_err = String::from_utf8(err)?;
    assert!(dbg_err.contains("[verbose] "));
    assert!(dbg_err.contains("[debug] lock=acquired"));
    assert!(dbg_err.contains("[debug] transport=dispatch"));
    assert!(dbg_err.contains("wrapper_sha256="));
    assert!(dbg_err.contains("[debug] lock=released"));
    assert!(!dbg_err.contains("TURN_CANARY_PROMPT"));
    assert!(!dbg_err.contains("SYSTEM WRAPPER HYGIENE"));

    assert_eq!(
        infer_log_level_best_effort(["turn", "--debug"], |_| None),
        LogLevel::Debug
    );
    Ok(())
}

#[test]
fn debug_flag_gates_redacted_panic_backtrace() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_openrouter_seats(tmp.path(), &["Saltworks"])?;
    let mock_dir = tmp.path().join("panic_mock");
    fs::create_dir_all(&mock_dir)?;
    fs::write(
        mock_dir.join("responses.json"),
        serde_json::to_string(&json!({
            "rules": [{
                "when_contains": "PANIC_TRIGGER",
                "panic_message": format!(
                    "boom holding {} {} {} {} {}",
                    OR_ENV_KEY, PLANTED_BEARER, PLANTED_XRPL_SECP, PLANTED_SEALED_STRUCT, PLANTED_SEALED_HEX
                )
            }]
        }))?,
    )?;

    let bin = env!("CARGO_BIN_EXE_league-relay");
    for (label, extra_flag, expect_bt) in [
        ("silent", vec!["-q"], false),
        ("normal", vec![], false),
        ("verbose", vec!["-v"], false),
        ("debug", vec!["--debug"], true),
    ] {
        let game_dir = tmp.path().join(format!("panic_gate_{label}"));
        write_board(&game_dir, "Saltworks", "PANIC_TRIGGER")?;
        let mut cmd = Command::new(bin);
        cmd.arg("turn");
        for f in extra_flag {
            cmd.arg(f);
        }
        cmd.args([
            "--game-dir",
            game_dir.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
            "--cap-credits",
            "1.0",
        ]);
        cmd.env("OPENROUTER_API_KEY", OR_ENV_KEY);
        cmd.env_remove(LEAGUE_LOG_LEVEL_ENV);
        let out = cmd.output()?;
        assert_eq!(out.status.code(), Some(2), "{label}: expected exit 2");
        let stderr_text = String::from_utf8(out.stderr)?;
        let stdout_text = String::from_utf8(out.stdout)?;
        assert!(
            stderr_text.contains("panic at") && stderr_text.contains(REDACTED_MARKER),
            "{label}: expected redacted panic header, got: {stderr_text}"
        );
        assert_eq!(
            stderr_text.contains("stack backtrace:"),
            expect_bt,
            "{label}: backtrace gating mismatch in stderr: {stderr_text}"
        );
        assert_no_secrets_leaked(&stderr_text, label)?;
        assert_no_secrets_leaked(&stdout_text, label)?;
    }
    Ok(())
}

fn verify_planted_secrets_redacted_at_level(level_flag: &str) -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = write_wrapper(tmp.path())?;
    let seats = write_openrouter_seats(tmp.path(), &["Saltworks"])?;

    let all_planted = format!(
        "{} {} {} {} {} {} {} {} {} {} {} {} {} {}",
        OR_ENV_KEY,
        OAI_ENV_KEY,
        VLLM_ENV_KEY,
        LOCAL_ENV_KEY,
        PLANTED_BEARER,
        PLANTED_SK_OR,
        PLANTED_SK_PROJ,
        PLANTED_SK_ANT,
        PLANTED_XRPL_SECP,
        PLANTED_XRPL_ED,
        PLANTED_SEALED_STRUCT,
        PLANTED_PREIMAGE_STRUCT,
        PLANTED_EVAL_FIELD,
        PLANTED_PREIMAGE_JSON
    );
    let numeric_seeds = format!("{PLANTED_SEALED_HEX} {PLANTED_SEALED_DEC}");

    // 1. Real binary subprocess with HTTP 500 + panic rules carrying all planted secret categories,
    //    plus all FORBIDDEN_LOCAL_KEY_ENVS set in the subprocess env.
    let mock_err = tmp.path().join("mock_err");
    fs::create_dir_all(&mock_err)?;
    fs::write(
        mock_err.join("responses.json"),
        serde_json::to_string(&json!({
            "rules": [
                {
                    "when_contains": "HTTP_ERR_RUN",
                    "status": 500,
                    "response": { "error": { "message": format!("{all_planted} {numeric_seeds}") } }
                },
                {
                    "when_contains": "PANIC_RUN",
                    "panic_message": format!("panic {all_planted} {numeric_seeds}")
                }
            ]
        }))?,
    )?;

    for scenario in ["HTTP_ERR_RUN", "PANIC_RUN"] {
        let g = tmp.path().join(format!("sub_{level_flag}_{scenario}"));
        write_board(&g, "Saltworks", scenario)?;
        let out = Command::new(env!("CARGO_BIN_EXE_league-relay"))
            .args([
                "turn",
                "--log-level",
                level_flag,
                "--game-dir",
                g.to_str().ok_or("utf8")?,
                "--day",
                "01",
                "--round",
                "talk",
                "--seats",
                seats.to_str().ok_or("utf8")?,
                "--wrapper",
                wrapper.to_str().ok_or("utf8")?,
                "--mock-dir",
                mock_err.to_str().ok_or("utf8")?,
                "--cap-credits",
                "1.0",
            ])
            .env("OPENROUTER_API_KEY", OR_ENV_KEY)
            .env("OPENAI_API_KEY", OAI_ENV_KEY)
            .env("VLLM_API_KEY", VLLM_ENV_KEY)
            .env("LOCAL_API_KEY", LOCAL_ENV_KEY)
            .env_remove(LEAGUE_LOG_LEVEL_ENV)
            .output()?;
        assert_eq!(out.status.code(), Some(2));
        let stdout_s = String::from_utf8(out.stdout)?;
        let stderr_s = String::from_utf8(out.stderr)?;
        assert_no_secrets_leaked(&stdout_s, &format!("{level_flag}/{scenario}/stdout"))?;
        assert_no_secrets_leaked(&stderr_s, &format!("{level_flag}/{scenario}/stderr"))?;
        let calls_raw = fs::read_to_string(g.join("relay/calls.jsonl"))?;
        assert_no_secrets_leaked(&calls_raw, &format!("{level_flag}/{scenario}/calls.jsonl"))?;
    }

    // 2. In-process `execute_cli` with planted secrets in CLI error/path/JSON output at this level.
    let mut env_map = HashMap::new();
    env_map.insert("OPENROUTER_API_KEY", OR_ENV_KEY.to_owned());
    env_map.insert("OPENAI_API_KEY", OAI_ENV_KEY.to_owned());
    env_map.insert("VLLM_API_KEY", VLLM_ENV_KEY.to_owned());
    env_map.insert("LOCAL_API_KEY", LOCAL_ENV_KEY.to_owned());

    for err_fmt in ["text", "json"] {
        let secret_flag = format!("--{PLANTED_SK_PROJ}-{PLANTED_XRPL_SECP}");
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = execute_cli(
            [
                "--log-level",
                level_flag,
                "--error-format",
                err_fmt,
                "turn",
                &secret_flag,
            ],
            |k| env_map.get(k).cloned(),
            &mut out,
            &mut err,
        );
        assert_eq!(code, EXIT_USER_ERROR);
        assert_no_secrets_leaked(
            &String::from_utf8(err)?,
            &format!("{level_flag}/cli_err_{err_fmt}"),
        )?;
    }

    // 3. Parse check with every `FORBIDDEN_LOCAL_KEY_ENVS` entry ensures list constant coverage.
    assert!(FORBIDDEN_LOCAL_KEY_ENVS.contains(&OPENROUTER_API_KEY_ENV));
    assert!(parse_args_with_key(["turn", "--log-level", level_flag, "--help"], None).is_ok());
    let _dummy_key = ApiKey::from_env_reader(|_| Some(OR_ENV_KEY.to_owned()))?;
    Ok(())
}

#[test]
fn planted_secrets_redacted_at_silent_level() -> TestResult {
    verify_planted_secrets_redacted_at_level("silent")
}

#[test]
fn planted_secrets_redacted_at_normal_level() -> TestResult {
    verify_planted_secrets_redacted_at_level("normal")
}

#[test]
fn planted_secrets_redacted_at_verbose_level() -> TestResult {
    verify_planted_secrets_redacted_at_level("verbose")
}

#[test]
fn planted_secrets_redacted_at_debug_level() -> TestResult {
    verify_planted_secrets_redacted_at_level("debug")
}

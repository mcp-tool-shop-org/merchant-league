//! Integration and unit-level contract tests for `league-relay`.
//!
//! Every test runs offline against mock [`Transport`] implementations or
//! `--mock-dir` fixtures in temporary directories. No network socket is opened
//! and no real API key is used.

use league_relay::cli::{CliCommand, execute_cli, parse_args, parse_args_with_key};
use league_relay::{
    ApiKey, CallLogEntry, LiveGate, MAX_REDACTION_SCAN_BYTES, RawHttpResponse, RelayError, Round,
    SeatFile, SeatOutcome, TranscriptEntry, Transport, TransportError, TurnConfig, UreqTransport,
    cap_utf8_bytes, install_redacting_panic_hook, redact_and_truncate, redact_secrets, run_turn,
    sha256_hex, truncate_after_redaction,
};
use serde_json::{Value as Json, json};
use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn Error>>;

const PLANTED_KEY: &str = "sk-or-v1-planted-fake-secret-998877665544332211";

struct ScriptedTransport {
    calls: AtomicUsize,
    recorded_bodies: Mutex<Vec<String>>,
    script: Mutex<VecDeque<ScriptStep>>,
}

enum ScriptStep {
    Respond(RawHttpResponse),
    Fail(TransportError),
    SleepThenTimeout(Duration),
    SleepThenRespond(Duration, RawHttpResponse),
    PanicWith(String),
    PanicStatic,
    PanicCode,
}

impl ScriptedTransport {
    fn new(steps: Vec<ScriptStep>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            recorded_bodies: Mutex::new(Vec::new()),
            script: Mutex::new(VecDeque::from(steps)),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn bodies(&self) -> Result<Vec<String>, Box<dyn Error>> {
        let guard = self
            .recorded_bodies
            .lock()
            .map_err(|_| "recorded_bodies lock poisoned")?;
        Ok(guard.clone())
    }
}

impl Transport for ScriptedTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut rec) = self.recorded_bodies.lock() {
            rec.push(body.to_owned());
        }
        let step = {
            let mut guard = self
                .script
                .lock()
                .map_err(|_| TransportError::Network("script lock poisoned".to_owned()))?;
            guard.pop_front()
        };
        match step {
            Some(ScriptStep::Respond(r)) => Ok(r),
            Some(ScriptStep::Fail(e)) => Err(e),
            Some(ScriptStep::SleepThenTimeout(dur)) => {
                thread::sleep(dur);
                Err(TransportError::Timeout)
            }
            Some(ScriptStep::SleepThenRespond(dur, resp)) => {
                thread::sleep(dur);
                Ok(resp)
            }
            Some(ScriptStep::PanicWith(msg)) => panic!("{msg}"),
            Some(ScriptStep::PanicStatic) => panic!("static transport panic"),
            Some(ScriptStep::PanicCode) => std::panic::panic_any(7_i32),
            None => Err(TransportError::Network("script exhausted".to_owned())),
        }
    }
}

fn ok_completion_json(content: &str, model_returned: &str, cost: Option<f64>) -> RawHttpResponse {
    let mut usage = json!({
        "prompt_tokens": 110,
        "completion_tokens": 25,
        "total_tokens": 135
    });
    if let Some(c) = cost
        && let Some(obj) = usage.as_object_mut()
    {
        obj.insert("cost".to_owned(), json!(c));
    }
    let payload = json!({
        "id": "gen-mock-1",
        "model": model_returned,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }
        ],
        "usage": usage
    });
    RawHttpResponse {
        status: 200,
        body: payload.to_string(),
    }
}

fn single_openrouter_seats(guild: &str, timeout_secs: f64) -> Result<SeatFile, Box<dyn Error>> {
    let raw = json!({
        "seats": [
            {
                "guild": guild,
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.4,
                "max_output_tokens": 512,
                "timeout_secs": timeout_secs,
                "max_cost_per_call": 0.05
            }
        ]
    })
    .to_string();
    Ok(SeatFile::from_json_str(&raw, None)?)
}

fn write_to_seat(
    game_dir: &Path,
    day: &str,
    round: &str,
    guild: &str,
    text: &str,
) -> Result<(), Box<dyn Error>> {
    let dir = game_dir.join("to-seat");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("day-{day}-{round}-{guild}.md")), text)?;
    Ok(())
}

fn read_calls_log(game_dir: &Path) -> Result<Vec<CallLogEntry>, Box<dyn Error>> {
    let raw = fs::read_to_string(game_dir.join("relay").join("calls.jsonl"))?;
    let mut out = Vec::new();
    for line in raw.lines() {
        if !line.trim().is_empty() {
            out.push(serde_json::from_str::<CallLogEntry>(line)?);
        }
    }
    Ok(out)
}

fn read_transcript_log(
    game_dir: &Path,
    guild: &str,
) -> Result<Vec<TranscriptEntry>, Box<dyn Error>> {
    let raw = fs::read_to_string(
        game_dir
            .join("relay")
            .join("seats")
            .join(format!("{guild}.jsonl")),
    )?;
    let mut out = Vec::new();
    for line in raw.lines() {
        if !line.trim().is_empty() {
            out.push(serde_json::from_str::<TranscriptEntry>(line)?);
        }
    }
    Ok(out)
}

#[test]
fn seat_file_validates_pinned_slugs_and_manual_file_seats() -> TestResult {
    let valid = json!({
        "seats": [
            {
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.7,
                "max_output_tokens": 1024,
                "timeout_secs": 90,
                "max_cost_per_call": 0.05
            },
            {
                "guild": "Smiths",
                "backend": "file",
                "model": "claude-sonnet-5-5"
            }
        ]
    })
    .to_string();
    let parsed = SeatFile::from_json_str(&valid, None)?;
    assert_eq!(parsed.seats.len(), 2);
    assert!(parsed.has_openrouter_seat());

    // Bare array form is also accepted.
    let array_only = json!([
        {
            "guild": "Dyers",
            "backend": "file"
        }
    ])
    .to_string();
    let manual_only = SeatFile::from_json_str(&array_only, None)?;
    assert!(!manual_only.has_openrouter_seat());

    // Floating aliases and unpinned forms must be rejected.
    for bad_slug in [
        "openrouter/auto",
        "google/auto",
        "google/gemini-latest",
        "google/latest",
        "google/gemini-2.5-pro:free",
        "google/gemini-2.5-pro:nitro",
        "gemini-2.5-pro",
        "google/",
        "/gemini",
        "google/gemini/extra",
        "google/gemini 2.5",
    ] {
        let bad_json = json!({
            "seats": [{
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": bad_slug,
                "temperature": 0.5,
                "max_output_tokens": 256,
                "timeout_secs": 30,
                "max_cost_per_call": 0.05
            }]
        })
        .to_string();
        assert!(
            SeatFile::from_json_str(&bad_json, None).is_err(),
            "expected slug '{bad_slug}' to be rejected"
        );
    }

    // Duplicate guild and path traversal guild names must be rejected.
    let dup = json!({
        "seats": [
            {"guild": "Smiths", "backend": "file"},
            {"guild": "Smiths", "backend": "file"}
        ]
    })
    .to_string();
    assert!(SeatFile::from_json_str(&dup, None).is_err());

    let traversal = json!({
        "seats": [{"guild": "../Smiths", "backend": "file"}]
    })
    .to_string();
    assert!(SeatFile::from_json_str(&traversal, None).is_err());

    Ok(())
}

#[test]
fn first_turn_wrapper_and_multi_turn_transcript_replay_order() -> TestResult {
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("game");
    let wrapper_path = tmp.path().join("wrapper.md");
    fs::write(&wrapper_path, "SEAT BRIEF HEADER\nPlay only by the rules.")?;

    let seats = single_openrouter_seats("Saltworks", 5.0)?;

    // Without --wrapper on turn 1, run_turn must fail before calling transport.
    write_to_seat(&game_dir, "01", "talk", "Saltworks", "DAY 01 PUBLIC STATE")?;
    let missing_wrapper_cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Talk,
        seats.clone(),
        None,
        false,
        None,
    );
    let unused_transport = ScriptedTransport::new(vec![]);
    let err = run_turn(&missing_wrapper_cfg, &unused_transport, None);
    assert!(matches!(err, Err(RelayError::MissingWrapper { .. })));
    assert_eq!(unused_transport.call_count(), 0);

    // Turn 1 (day 01 talk): wrapper is prepended, no system message is sent.
    let t1_transport = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "To Weavers: trade salt at Varn?",
        "google/gemini-2.5-pro-06-05",
        Some(0.0021),
    ))]);
    let t1_cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Talk,
        seats.clone(),
        Some(wrapper_path.clone()),
        false,
        Some(1.0),
    );
    let s1 = run_turn(&t1_cfg, &t1_transport, None)?;
    assert!(s1.all_clean());

    let t1_bodies = t1_transport.bodies()?;
    assert_eq!(t1_bodies.len(), 1);
    let req1: Json = serde_json::from_str(&t1_bodies[0])?;
    let msgs1 = req1["messages"].as_array().ok_or("messages not array")?;
    assert_eq!(msgs1.len(), 1);
    assert_eq!(msgs1[0]["role"], "user");
    assert_eq!(
        msgs1[0]["content"],
        "SEAT BRIEF HEADER\nPlay only by the rules.\n\nDAY 01 PUBLIC STATE"
    );

    // Turn 2 (day 01 act): even with wrapper_path still passed, wrapper is NOT duplicated;
    // history replays [user_1, assistant_1, user_2] with zero system messages.
    write_to_seat(&game_dir, "01", "act", "Saltworks", "DAY 01 ACT PROMPT")?;
    let t2_transport = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "TRAVEL: go to Ostra\n",
        "google/gemini-2.5-pro-06-05",
        Some(0.0019),
    ))]);
    let t2_cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Act,
        seats.clone(),
        Some(wrapper_path),
        false,
        Some(1.0),
    );
    let s2 = run_turn(&t2_cfg, &t2_transport, None)?;
    assert!(s2.all_clean());

    let t2_bodies = t2_transport.bodies()?;
    let req2: Json = serde_json::from_str(&t2_bodies[0])?;
    let msgs2 = req2["messages"].as_array().ok_or("messages not array")?;
    assert_eq!(msgs2.len(), 3);
    for m in msgs2 {
        assert_ne!(m["role"], "system");
    }
    assert_eq!(msgs2[0]["role"], "user");
    assert_eq!(
        msgs2[0]["content"],
        "SEAT BRIEF HEADER\nPlay only by the rules.\n\nDAY 01 PUBLIC STATE"
    );
    assert_eq!(msgs2[1]["role"], "assistant");
    assert_eq!(msgs2[1]["content"], "To Weavers: trade salt at Varn?");
    assert_eq!(msgs2[2]["role"], "user");
    assert_eq!(msgs2[2]["content"], "DAY 01 ACT PROMPT");

    // Turn 3 (day 02 talk): wrapper_path omitted; works because transcript already exists.
    write_to_seat(&game_dir, "02", "talk", "Saltworks", "DAY 02 TALK PROMPT")?;
    let t3_transport = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "To Dyers: 2 salt ready.",
        "google/gemini-2.5-pro-06-05",
        Some(0.0025),
    ))]);
    let t3_cfg = TurnConfig::new(
        game_dir.clone(),
        "02".to_owned(),
        Round::Talk,
        seats,
        None,
        false,
        Some(1.0),
    );
    let s3 = run_turn(&t3_cfg, &t3_transport, None)?;
    assert!(s3.all_clean());

    let transcript = read_transcript_log(&game_dir, "Saltworks")?;
    assert_eq!(transcript.len(), 6);
    assert_eq!(transcript[4].day, "02");
    assert_eq!(transcript[4].role, "user");
    assert_eq!(transcript[5].role, "assistant");

    Ok(())
}

#[test]
fn verbatim_write_preserves_exact_trailing_newlines_and_bytes() -> TestResult {
    let cases = [
        ("01", "talk", "TRAVEL: stay"),
        ("01", "act", "TRAVEL: go to Ostra\n"),
        (
            "02",
            "act",
            "OFFERS: post 6 coin for 2 salt\r\nTRAVEL: stay\n\n",
        ),
    ];

    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;
    let seats = single_openrouter_seats("Weavers", 5.0)?;

    for (day, round_str, exact_reply) in cases {
        write_to_seat(&game_dir, day, round_str, "Weavers", "PROMPT BLOCK")?;
        let transport = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
            exact_reply,
            "google/gemini-2.5-pro-06-05",
            Some(0.001),
        ))]);
        let cfg = TurnConfig::new(
            game_dir.clone(),
            day.to_owned(),
            Round::parse(round_str)?,
            seats.clone(),
            Some(wrapper.clone()),
            false,
            None,
        );
        let summary = run_turn(&cfg, &transport, None)?;
        assert!(summary.all_clean());

        let written_bytes = fs::read(
            game_dir
                .join("from-seat")
                .join(format!("day-{day}-{round_str}-Weavers.md")),
        )?;
        assert_eq!(written_bytes, exact_reply.as_bytes());
    }

    let calls = read_calls_log(&game_dir)?;
    assert_eq!(calls.len(), 3);
    for (idx, (_, _, exact_reply)) in cases.iter().enumerate() {
        assert_eq!(
            calls[idx].reply_sha256.as_deref(),
            Some(sha256_hex(exact_reply.as_bytes()).as_str())
        );
    }

    Ok(())
}

#[test]
fn delivery_failures_resend_once_identically_as_harness_then_no_reply() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    // 1. Transport error on attempt 1 -> harness re-send succeeds on attempt 2 with identical body.
    let game_recover = tmp.path().join("recover");
    write_to_seat(&game_recover, "01", "talk", "Saltworks", "STATE 1")?;
    let seats = single_openrouter_seats("Saltworks", 5.0)?;
    let recover_transport = ScriptedTransport::new(vec![
        ScriptStep::Fail(TransportError::Network("connection reset".to_owned())),
        ScriptStep::Respond(ok_completion_json(
            "To Weavers: hello",
            "google/gemini-2.5-pro-06-05",
            Some(0.0012),
        )),
    ]);
    let recover_cfg = TurnConfig::new(
        game_recover.clone(),
        "01".to_owned(),
        Round::Talk,
        seats.clone(),
        Some(wrapper.clone()),
        false,
        Some(1.0),
    );
    let recover_summary = run_turn(&recover_cfg, &recover_transport, None)?;
    assert!(recover_summary.all_clean());
    assert_eq!(recover_transport.call_count(), 2);
    let bodies = recover_transport.bodies()?;
    assert_eq!(bodies[0], bodies[1]);

    let recover_calls = read_calls_log(&game_recover)?;
    assert_eq!(recover_calls.len(), 2);
    assert_eq!(recover_calls[0].attempt, 1);
    assert_eq!(recover_calls[0].outcome, "harness");
    assert_eq!(recover_calls[1].attempt, 2);
    assert_eq!(recover_calls[1].trigger, "harness");
    assert_eq!(recover_calls[1].outcome, "OK");
    assert_eq!(
        recover_calls[0].request_body_sha256,
        recover_calls[1].request_body_sha256
    );

    // 2. HTTP 502 then empty reply -> second failure writes no from-seat file and logs NO_REPLY.
    let game_fail = tmp.path().join("double_fail");
    write_to_seat(&game_fail, "01", "act", "Saltworks", "ACT STATE")?;
    let fail_transport = ScriptedTransport::new(vec![
        ScriptStep::Respond(RawHttpResponse {
            status: 502,
            body: "upstream gateway error".to_owned(),
        }),
        ScriptStep::Respond(ok_completion_json(
            "   \n\t  ",
            "google/gemini-2.5-pro-06-05",
            Some(0.0004),
        )),
        // Extra step proves a 3rd call is never attempted.
        ScriptStep::Respond(ok_completion_json(
            "TRAVEL: stay",
            "google/gemini-2.5-pro-06-05",
            Some(0.0004),
        )),
    ]);
    let fail_cfg = TurnConfig::new(
        game_fail.clone(),
        "01".to_owned(),
        Round::Act,
        seats,
        Some(wrapper.clone()),
        false,
        Some(1.0),
    );
    let fail_summary = run_turn(&fail_cfg, &fail_transport, None)?;
    assert!(!fail_summary.all_clean());
    assert_eq!(fail_transport.call_count(), 2);
    assert!(
        !game_fail
            .join("from-seat")
            .join("day-01-act-Saltworks.md")
            .exists()
    );
    assert!(
        !game_fail
            .join("relay")
            .join("seats")
            .join("Saltworks.jsonl")
            .exists()
    );

    let fail_calls = read_calls_log(&game_fail)?;
    assert_eq!(fail_calls.len(), 2);
    assert_eq!(fail_calls[0].outcome, "harness");
    assert_eq!(fail_calls[1].outcome, "NO_REPLY");

    // 3. Wall-clock timeout enforced across both attempts -> harness then NO_REPLY.
    let game_timeout = tmp.path().join("timeout");
    write_to_seat(&game_timeout, "01", "talk", "Saltworks", "SLOW STATE")?;
    let fast_timeout_seats = single_openrouter_seats("Saltworks", 0.04)?;
    let slow_transport = ScriptedTransport::new(vec![
        ScriptStep::SleepThenTimeout(Duration::from_millis(130)),
        ScriptStep::SleepThenTimeout(Duration::from_millis(130)),
    ]);
    let timeout_cfg = TurnConfig::new(
        game_timeout.clone(),
        "01".to_owned(),
        Round::Talk,
        fast_timeout_seats,
        Some(wrapper),
        false,
        None,
    );
    let timeout_summary = run_turn(&timeout_cfg, &slow_transport, None)?;
    assert!(matches!(
        timeout_summary.reports[0].outcome,
        SeatOutcome::NoReply { .. }
    ));
    let timeout_calls = read_calls_log(&game_timeout)?;
    assert_eq!(timeout_calls.len(), 2);
    assert_eq!(timeout_calls[0].outcome, "harness");
    assert_eq!(timeout_calls[0].error.as_deref(), Some("timeout"));
    assert_eq!(timeout_calls[1].outcome, "NO_REPLY");

    Ok(())
}

#[test]
fn wrong_form_reply_is_not_retried() -> TestResult {
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;
    write_to_seat(&game_dir, "01", "act", "Tanners", "ACT PROMPT")?;

    let seats = single_openrouter_seats("Tanners", 5.0)?;
    let wrong_form = "I forgot the action template; my caravan waits here.";
    let transport = ScriptedTransport::new(vec![
        ScriptStep::Respond(ok_completion_json(
            wrong_form,
            "google/gemini-2.5-pro-06-05",
            Some(0.0015),
        )),
        ScriptStep::Respond(ok_completion_json(
            "TRAVEL: stay",
            "google/gemini-2.5-pro-06-05",
            Some(0.0015),
        )),
    ]);

    let cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Act,
        seats,
        Some(wrapper),
        false,
        Some(1.0),
    );
    let summary = run_turn(&cfg, &transport, None)?;
    assert!(summary.all_clean());
    assert_eq!(transport.call_count(), 1);
    let saved = fs::read_to_string(game_dir.join("from-seat").join("day-01-act-Tanners.md"))?;
    assert_eq!(saved, wrong_form);

    Ok(())
}

struct ConcurrentProofTransport {
    arrivals: (Mutex<usize>, Condvar),
    saw_both_in_flight: Mutex<bool>,
}

impl ConcurrentProofTransport {
    fn new() -> Self {
        Self {
            arrivals: (Mutex::new(0), Condvar::new()),
            saw_both_in_flight: Mutex::new(false),
        }
    }
}

impl Transport for ConcurrentProofTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        let (lock, cvar) = &self.arrivals;
        if let Ok(mut count) = lock.lock() {
            *count += 1;
            if *count == 2 {
                if let Ok(mut both) = self.saw_both_in_flight.lock() {
                    *both = true;
                }
                cvar.notify_all();
            } else if *count == 1 {
                while *count < 2 {
                    let (next, res) = cvar
                        .wait_timeout(count, Duration::from_secs(2))
                        .map_err(|_| TransportError::Network("condvar poisoned".to_owned()))?;
                    count = next;
                    if res.timed_out() {
                        break;
                    }
                }
            }
        }
        if body.contains("CONCURRENT_SEAT_OK") {
            return Ok(ok_completion_json(
                "TRAVEL: stay",
                "google/gemini-2.5-pro-06-05",
                Some(0.002),
            ));
        }
        Err(TransportError::Network(
            "simulated upstream outage".to_owned(),
        ))
    }
}

#[test]
fn seats_run_concurrently_and_one_seat_failure_never_stops_others() -> TestResult {
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    write_to_seat(&game_dir, "01", "act", "Saltworks", "CONCURRENT_SEAT_OK")?;
    write_to_seat(
        &game_dir,
        "01",
        "act",
        "Weavers",
        "CONCURRENT_SEAT_FAIL_ONCE",
    )?;
    // Dyers intentionally has no to-seat file to verify per-seat I/O isolation.

    let seats_raw = json!({
        "seats": [
            {
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.05
            },
            {
                "guild": "Weavers",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.05
            },
            {
                "guild": "Dyers",
                "backend": "openrouter",
                "model": "openai/gpt-oss-120b",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.05
            },
            {
                "guild": "Smiths",
                "backend": "file",
                "model": "claude-sonnet-5-5"
            }
        ]
    })
    .to_string();
    let seats = SeatFile::from_json_str(&seats_raw, None)?;
    let transport = ConcurrentProofTransport::new();
    let cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Act,
        seats,
        Some(wrapper),
        false,
        Some(1.0),
    );

    let summary = run_turn(&cfg, &transport, None)?;
    let both_in_flight = *transport
        .saw_both_in_flight
        .lock()
        .map_err(|_| "lock poisoned")?;
    assert!(
        both_in_flight,
        "expected seats to enter transport concurrently"
    );
    assert!(!summary.all_clean());
    assert_eq!(summary.reports.len(), 4);
    assert!(matches!(
        summary.reports[0].outcome,
        SeatOutcome::Delivered { attempts: 1, .. }
    ));
    assert!(matches!(
        summary.reports[1].outcome,
        SeatOutcome::NoReply { .. }
    ));
    assert!(matches!(
        summary.reports[2].outcome,
        SeatOutcome::SeatError { .. }
    ));
    assert_eq!(summary.reports[3].outcome, SeatOutcome::RelayedByHand);

    assert!(
        game_dir
            .join("from-seat")
            .join("day-01-act-Saltworks.md")
            .exists()
    );
    assert!(
        !game_dir
            .join("from-seat")
            .join("day-01-act-Weavers.md")
            .exists()
    );

    Ok(())
}

#[test]
fn spend_cap_sums_calls_jsonl_and_refuses_with_cap_reached() -> TestResult {
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;
    let seats = single_openrouter_seats("Saltworks", 5.0)?;

    // Turn 1 costs 0.05, Turn 2 costs 0.05 -> cumulative sum 0.10 == cap 0.10.
    write_to_seat(&game_dir, "01", "talk", "Saltworks", "T1")?;
    let t1 = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "Msg 1",
        "google/gemini-2.5-pro-06-05",
        Some(0.05),
    ))]);
    run_turn(
        &TurnConfig::new(
            game_dir.clone(),
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper),
            true,
            Some(0.10),
        ),
        &t1,
        None,
    )?;

    write_to_seat(&game_dir, "01", "act", "Saltworks", "T2")?;
    let t2 = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "TRAVEL: stay",
        "google/gemini-2.5-pro-06-05",
        Some(0.05),
    ))]);
    run_turn(
        &TurnConfig::new(
            game_dir.clone(),
            "01".to_owned(),
            Round::Act,
            seats.clone(),
            None,
            true,
            Some(0.10),
        ),
        &t2,
        None,
    )?;

    // Turn 3 must be refused pre-call with CAP_REACHED and write nothing.
    write_to_seat(&game_dir, "02", "talk", "Saltworks", "T3")?;
    let t3 = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "Should never be called",
        "google/gemini-2.5-pro-06-05",
        Some(0.01),
    ))]);
    let summary = run_turn(
        &TurnConfig::new(
            game_dir.clone(),
            "02".to_owned(),
            Round::Talk,
            seats,
            None,
            true,
            Some(0.10),
        ),
        &t3,
        None,
    )?;
    assert_eq!(t3.call_count(), 0);
    assert!(matches!(
        summary.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));
    assert!(
        !game_dir
            .join("from-seat")
            .join("day-02-talk-Saltworks.md")
            .exists()
    );
    let transcript = read_transcript_log(&game_dir, "Saltworks")?;
    assert_eq!(transcript.len(), 4); // Still only turns 1 and 2.

    let calls = read_calls_log(&game_dir)?;
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].outcome, "CAP_REACHED");

    Ok(())
}

struct CostUnknownSequencedTransport {
    seat_a_returned: (Mutex<bool>, Condvar),
}

impl CostUnknownSequencedTransport {
    fn new() -> Self {
        Self {
            seat_a_returned: (Mutex::new(false), Condvar::new()),
        }
    }
}

impl Transport for CostUnknownSequencedTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        if body.contains("SEAT_A_NO_COST") {
            let resp = ok_completion_json("Seat A reply", "google/gemini-2.5-pro-06-05", None);
            let (lock, cvar) = &self.seat_a_returned;
            if let Ok(mut done) = lock.lock() {
                *done = true;
                cvar.notify_all();
            }
            return Ok(resp);
        }
        // Seat B waits until Seat A has returned its costless response, then takes
        // one transient failure so its retry hits the ledger after Seat A recorded cost=None.
        let (lock, cvar) = &self.seat_a_returned;
        if let Ok(mut done) = lock.lock() {
            while !*done {
                let (next_guard, timeout_res) = cvar
                    .wait_timeout(done, Duration::from_secs(2))
                    .map_err(|_| TransportError::Network("condvar poisoned".to_owned()))?;
                done = next_guard;
                if timeout_res.timed_out() {
                    break;
                }
            }
        }
        // Small yield so Seat A finishes committing its call log line under the ledger mutex.
        thread::sleep(Duration::from_millis(15));
        Err(TransportError::Network(
            "transient drop before retry".to_owned(),
        ))
    }
}

#[test]
fn missing_response_cost_logs_tokens_and_fails_closed_for_rest_of_turn() -> TestResult {
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    write_to_seat(&game_dir, "01", "talk", "Saltworks", "SEAT_A_NO_COST")?;
    write_to_seat(&game_dir, "01", "talk", "Weavers", "SEAT_B_AFTER")?;

    let seats_raw = json!({
        "seats": [
            {
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.2,
                "max_output_tokens": 256,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.05
            },
            {
                "guild": "Weavers",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.2,
                "max_output_tokens": 256,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.05
            }
        ]
    })
    .to_string();
    let seats = SeatFile::from_json_str(&seats_raw, None)?;
    let transport = CostUnknownSequencedTransport::new();
    let cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Talk,
        seats,
        Some(wrapper),
        true,
        Some(5.0),
    );

    let summary = run_turn(&cfg, &transport, None)?;
    assert!(matches!(
        summary.reports[0].outcome,
        SeatOutcome::Delivered { .. }
    ));
    assert!(matches!(
        summary.reports[1].outcome,
        SeatOutcome::CostUnknownRefused { .. }
    ));
    assert!(
        !game_dir
            .join("from-seat")
            .join("day-01-talk-Weavers.md")
            .exists()
    );

    let calls = read_calls_log(&game_dir)?;
    let salt_call = calls
        .iter()
        .find(|c| c.guild == "Saltworks")
        .ok_or("missing Saltworks call")?;
    assert_eq!(salt_call.cost, None);
    assert_eq!(salt_call.prompt_tokens, Some(110));
    assert_eq!(salt_call.completion_tokens, Some(25));
    assert_eq!(salt_call.total_tokens, Some(135));
    assert!(calls.iter().any(|c| c.outcome == "COST_UNKNOWN"));

    Ok(())
}

#[test]
fn live_gate_refuses_network_without_live_flag_key_and_cap() -> TestResult {
    let key_ok = ApiKey::from_env_reader(|_| Some(PLANTED_KEY.to_owned()));
    assert!(matches!(
        LiveGate::acquire(false, Some(1.0), key_ok),
        Err(RelayError::LiveFlagRequired)
    ));

    let key_missing = ApiKey::from_env_reader(|_| None);
    assert!(matches!(
        LiveGate::acquire(true, Some(1.0), key_missing),
        Err(RelayError::MissingApiKey)
    ));

    let key_blank = ApiKey::from_env_reader(|_| Some("   ".to_owned()));
    assert!(matches!(
        LiveGate::acquire(true, Some(1.0), key_blank),
        Err(RelayError::MissingApiKey)
    ));

    let key_ok_2 = ApiKey::from_env_reader(|_| Some(PLANTED_KEY.to_owned()));
    assert!(matches!(
        LiveGate::acquire(true, None, key_ok_2),
        Err(RelayError::InvalidCap(_))
    ));

    Ok(())
}

#[test]
fn planted_key_is_redacted_across_debug_errors_panics_cli_and_logs() -> TestResult {
    let key = ApiKey::from_env_reader(|_| Some(PLANTED_KEY.to_owned()))?;
    install_redacting_panic_hook(Some(key.clone()));
    assert!(!format!("{key:?}").contains(PLANTED_KEY));

    let gate = LiveGate::acquire(true, Some(2.5), Ok(key.clone()))?;
    assert!(!format!("{gate:?}").contains(PLANTED_KEY));

    let live_transport = UreqTransport::new(gate);
    assert!(!format!("{live_transport:?}").contains(PLANTED_KEY));

    // Direct redaction helper check on raw key, Bearer header, and sk-or- tokens.
    let raw_leak = format!("header Authorization: Bearer {PLANTED_KEY} and custom_secret_token");
    let scrubbed = redact_secrets(&raw_leak, Some(&key));
    assert!(!scrubbed.contains(PLANTED_KEY));

    // CLI forbidden key flags and unknown flags with inline secrets must never echo the key.
    for bad_args in [
        vec!["turn", "--api-key", PLANTED_KEY],
        vec!["turn", &format!("--api-key={PLANTED_KEY}")],
        vec!["turn", &format!("--mystery-flag={PLANTED_KEY}")],
        vec![&format!("subcmd={PLANTED_KEY}")],
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = execute_cli(
            bad_args,
            |_| Some(PLANTED_KEY.to_owned()),
            &mut out,
            &mut err,
        );
        assert_eq!(code, 1);
        let stderr_text = String::from_utf8(err)?;
        assert!(
            !stderr_text.contains(PLANTED_KEY),
            "CLI stderr leaked key: {stderr_text}"
        );
    }

    // Seat file containing a key or malformed JSON containing a key must never echo it.
    let seat_with_key = json!({
        "seats": [{
            "guild": "Saltworks",
            "backend": "openrouter",
            "api_key": PLANTED_KEY
        }]
    })
    .to_string();
    let seat_err = SeatFile::from_json_str(&seat_with_key, Some(&key));
    assert!(matches!(
        seat_err,
        Err(RelayError::SecretInForbiddenPlace(_))
    ));
    let seat_err_str = format!("{seat_err:?}");
    assert!(!seat_err_str.contains(PLANTED_KEY));

    // Transport error, HTTP error body, invalid JSON body, and panic payload carrying the key.
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("redaction_game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;
    write_to_seat(&game_dir, "01", "talk", "Saltworks", "PROMPT")?;
    write_to_seat(&game_dir, "01", "talk", "Weavers", "PROMPT")?;

    let seats_raw = json!({
        "seats": [
            {
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.1,
                "max_output_tokens": 128,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.05
            },
            {
                "guild": "Weavers",
                "backend": "openrouter",
                "model": "openai/gpt-oss-120b",
                "temperature": 0.1,
                "max_output_tokens": 128,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.05
            }
        ]
    })
    .to_string();
    let seats = SeatFile::from_json_str(&seats_raw, Some(&key))?;

    let leaky_transport = ScriptedTransport::new(vec![
        ScriptStep::Fail(TransportError::Network(format!(
            "proxy rejected Bearer {PLANTED_KEY}"
        ))),
        ScriptStep::Respond(RawHttpResponse {
            status: 401,
            body: format!("invalid key {PLANTED_KEY}"),
        }),
        ScriptStep::Respond(RawHttpResponse {
            status: 200,
            body: format!("not-json-body-with-{PLANTED_KEY}"),
        }),
        ScriptStep::PanicWith(format!("fatal crash holding {PLANTED_KEY}")),
    ]);

    let cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Talk,
        seats,
        Some(wrapper),
        false,
        None,
    );
    let summary = run_turn(&cfg, &leaky_transport, Some(&key))?;
    let summary_text = format!("{summary:?}\n{}", summary.format_paths_and_hashes());
    assert!(!summary_text.contains(PLANTED_KEY));

    let calls_raw = fs::read_to_string(game_dir.join("relay").join("calls.jsonl"))?;
    assert!(!calls_raw.contains(PLANTED_KEY));
    assert!(calls_raw.contains("<REDACTED_API_KEY>"));

    Ok(())
}

#[test]
fn cli_binary_mock_turn_and_live_refusal_end_to_end() -> TestResult {
    let help = parse_args(["--help"])?;
    assert_eq!(help, CliCommand::Help);

    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("invented-game");
    let mock_dir = tmp.path().join("mock");
    fs::create_dir_all(&mock_dir)?;

    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "INVENTED WRAPPER TEXT")?;

    let seats_path = tmp.path().join("seats.json");
    fs::write(
        &seats_path,
        serde_json::to_string_pretty(&json!({
            "seats": [
                {
                    "guild": "Saltworks",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.6,
                    "max_output_tokens": 512,
                    "timeout_secs": 30.0,
                    "max_cost_per_call": 0.05
                },
                {
                    "guild": "Smiths",
                    "backend": "file",
                    "model": "claude-sonnet-5-5"
                }
            ]
        }))?,
    )?;

    write_to_seat(
        &game_dir,
        "01",
        "talk",
        "Saltworks",
        "INVENTED DAY 01 TALK FOR SALTWORKS",
    )?;

    fs::write(
        mock_dir.join("responses.json"),
        serde_json::to_string_pretty(&json!({
            "rules": [
                {
                    "when_contains": "SALTWORKS",
                    "status": 200,
                    "response": {
                        "id": "gen-cli-1",
                        "model": "google/gemini-2.5-pro-06-05",
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": "To Weavers: meet at Ostra.\n"
                            }
                        }],
                        "usage": {
                            "prompt_tokens": 64,
                            "completion_tokens": 9,
                            "total_tokens": 73,
                            "cost": 0.0008
                        }
                    }
                }
            ]
        }))?,
    )?;

    let bin = env!("CARGO_BIN_EXE_league-relay");

    // 1. Without --live and without --mock-dir (even with a planted env key), binary refuses.
    let refused = Command::new(bin)
        .args([
            "turn",
            "--game-dir",
            game_dir.to_str().ok_or("non-utf8 path")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats_path.to_str().ok_or("non-utf8 path")?,
            "--wrapper",
            wrapper.to_str().ok_or("non-utf8 path")?,
        ])
        .env("OPENROUTER_API_KEY", PLANTED_KEY)
        .output()?;
    assert!(!refused.status.success());
    let refused_stderr = String::from_utf8(refused.stderr)?;
    assert!(refused_stderr.contains("refusing network call"));
    assert!(!refused_stderr.contains(PLANTED_KEY));

    // 2. With --mock-dir (and no env key), binary runs full two-seat turn cleanly.
    let ok_run = Command::new(bin)
        .args([
            "turn",
            "--game-dir",
            game_dir.to_str().ok_or("non-utf8 path")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats_path.to_str().ok_or("non-utf8 path")?,
            "--wrapper",
            wrapper.to_str().ok_or("non-utf8 path")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("non-utf8 path")?,
            "--cap-credits",
            "1.0",
        ])
        .env_remove("OPENROUTER_API_KEY")
        .output()?;
    assert!(
        ok_run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&ok_run.stderr)
    );
    let stdout_text = String::from_utf8(ok_run.stdout)?;
    assert!(stdout_text.contains("seat=Saltworks outcome=OK"));
    assert!(stdout_text.contains("seat=Smiths outcome=FILE_MANUAL"));

    Ok(())
}

#[test]
fn cli_validation_corrupt_logs_and_all_display_branches() -> TestResult {
    use league_relay::{MockDirTransport, prepend_wrapper, validate_day};

    // Wrapper boundary rules.
    assert_eq!(prepend_wrapper("", "BODY"), "BODY");
    assert_eq!(prepend_wrapper("HEAD", "BODY"), "HEAD\n\nBODY");
    assert_eq!(prepend_wrapper("HEAD\n", "BODY"), "HEAD\n\nBODY");
    assert_eq!(prepend_wrapper("HEAD\n\n", "BODY"), "HEAD\n\nBODY");

    // Day validation.
    for bad_day in ["", "1", "00", "100", "ab"] {
        assert!(validate_day(bad_day).is_err());
    }
    validate_day("01")?;
    validate_day("13")?;

    // TransportError Display.
    assert_eq!(TransportError::Timeout.to_string(), "request timed out");
    assert!(
        TransportError::Network("eof".to_owned())
            .to_string()
            .contains("eof")
    );

    // Every RelayError Display arm.
    for err in [
        RelayError::Usage("u".to_owned()),
        RelayError::SecretInForbiddenPlace("s".to_owned()),
        RelayError::LiveFlagRequired,
        RelayError::MissingApiKey,
        RelayError::InvalidCap("c".to_owned()),
        RelayError::MissingWrapper {
            guild: "Saltworks".to_owned(),
        },
        RelayError::SeatConfig("sc".to_owned()),
        RelayError::Io("io".to_owned()),
        RelayError::CorruptLog("cl".to_owned()),
    ] {
        assert!(!err.to_string().is_empty());
    }

    // CLI edge cases.
    assert!(parse_args::<[&str; 0], _>([]).is_err());
    assert_eq!(parse_args(["turn", "--help"])?, CliCommand::Help);
    assert!(parse_args(["turn", "--live=yes"]).is_err());
    assert!(parse_args(["turn", "--game-dir"]).is_err());
    assert!(parse_args(["turn", "--game-dir="]).is_err());
    assert!(
        parse_args([
            "turn",
            "--game-dir=g",
            "--day=01",
            "--round=talk",
            "--seats=s.json",
            "--live",
            "--mock-dir=m"
        ])
        .is_err()
    );
    assert!(
        parse_args([
            "turn",
            "--game-dir=g",
            "--day=01",
            "--round=bad",
            "--seats=s.json"
        ])
        .is_err()
    );
    assert!(
        parse_args([
            "turn",
            "--game-dir=g",
            "--day=01",
            "--round=talk",
            "--seats=s.json",
            "--cap-credits=-1"
        ])
        .is_err()
    );
    assert!(
        parse_args([
            "turn",
            "--game-dir=g",
            "--day=01",
            "--round=talk",
            "--seats=s.json",
            "--cap-credits=nope"
        ])
        .is_err()
    );

    // execute_cli --help and manual-only file seats.
    let mut help_out = Vec::new();
    let mut help_err = Vec::new();
    assert_eq!(
        execute_cli(["--help"], |_| None, &mut help_out, &mut help_err),
        0
    );
    assert!(!help_out.is_empty());

    let tmp = TempDir::new()?;
    let manual_seats_path = tmp.path().join("manual_seats.json");
    fs::write(
        &manual_seats_path,
        r#"{"seats":[{"guild":"Smiths","backend":"file"}]}"#,
    )?;
    let mut m_out = Vec::new();
    let mut m_err = Vec::new();
    let m_code = execute_cli(
        [
            "turn",
            "--game-dir",
            tmp.path().to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            manual_seats_path.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut m_out,
        &mut m_err,
    );
    assert_eq!(m_code, 0);
    assert!(String::from_utf8(m_out)?.contains("FILE_MANUAL"));

    // Seat validation edge branches.
    for bad_seat_json in [
        r#"{"seats":[]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter"}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b"}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b","temperature":3.0,"max_output_tokens":10,"timeout_secs":5,"max_cost_per_call":0.05}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b","temperature":0.5,"max_output_tokens":0,"timeout_secs":5,"max_cost_per_call":0.05}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b","temperature":0.5,"max_output_tokens":10,"timeout_secs":0,"max_cost_per_call":0.05}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b","temperature":0.5,"max_output_tokens":10,"timeout_secs":5}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b","temperature":0.5,"max_output_tokens":10,"timeout_secs":5,"max_cost_per_call":0}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b!c","temperature":0.5,"max_output_tokens":10,"timeout_secs":5,"max_cost_per_call":0.05}]}"#,
    ] {
        assert!(SeatFile::from_json_str(bad_seat_json, None).is_err());
    }

    // Corrupt transcript role and unpaired line, plus existing from-seat refusal and provider error object.
    let game_edge = tmp.path().join("edge_game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;
    let seats = single_openrouter_seats("Saltworks", 2.0)?;

    // Provider in-body error on HTTP 200 triggers retry then NO_REPLY.
    write_to_seat(&game_edge, "01", "talk", "Saltworks", "BLOCK")?;
    let provider_err_transport = ScriptedTransport::new(vec![
        ScriptStep::Respond(RawHttpResponse {
            status: 200,
            body: r#"{"error":{"message":"upstream overloaded"}}"#.to_owned(),
        }),
        ScriptStep::Respond(RawHttpResponse {
            status: 200,
            body: r#"{"error":{"message":"still overloaded"}}"#.to_owned(),
        }),
    ]);
    let edge_summary = run_turn(
        &TurnConfig::new(
            game_edge.clone(),
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper.clone()),
            false,
            None,
        ),
        &provider_err_transport,
        None,
    )?;
    let formatted_no_reply = edge_summary.format_paths_and_hashes();
    assert!(formatted_no_reply.contains("outcome=NO_REPLY"));

    // Existing from-seat file refuses overwrite.
    fs::create_dir_all(game_edge.join("from-seat"))?;
    fs::write(
        game_edge.join("from-seat").join("day-01-talk-Saltworks.md"),
        "existing",
    )?;
    let dup_summary = run_turn(
        &TurnConfig::new(
            game_edge.clone(),
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper.clone()),
            false,
            None,
        ),
        &provider_err_transport,
        None,
    )?;
    assert!(
        dup_summary
            .format_paths_and_hashes()
            .contains("outcome=ERROR")
    );

    // Corrupt transcript system role rejected.
    let bad_tx_game = tmp.path().join("bad_tx");
    fs::create_dir_all(bad_tx_game.join("relay").join("seats"))?;
    fs::write(
        bad_tx_game
            .join("relay")
            .join("seats")
            .join("Saltworks.jsonl"),
        r#"{"day":"01","round":"talk","role":"system","content":"nope"}"#,
    )?;
    assert!(
        run_turn(
            &TurnConfig::new(
                bad_tx_game,
                "01".to_owned(),
                Round::Act,
                seats.clone(),
                Some(wrapper.clone()),
                false,
                None,
            ),
            &provider_err_transport,
            None,
        )
        .is_err()
    );

    // Corrupt calls.jsonl rejected.
    let bad_calls_game = tmp.path().join("bad_calls");
    fs::create_dir_all(bad_calls_game.join("relay"))?;
    fs::write(
        bad_calls_game.join("relay").join("calls.jsonl"),
        "not-valid-json\n",
    )?;
    assert!(
        run_turn(
            &TurnConfig::new(
                bad_calls_game,
                "01".to_owned(),
                Round::Talk,
                seats,
                Some(wrapper),
                false,
                Some(1.0),
            ),
            &provider_err_transport,
            None,
        )
        .is_err()
    );

    // MockDirTransport missing file error.
    assert!(MockDirTransport::load(&tmp.path().join("missing_mock_dir")).is_err());

    Ok(())
}

#[test]
fn additional_branch_and_cli_runner_coverage() -> TestResult {
    use league_relay::MockDirTransport;
    use league_relay::cli::NoopManualTransport;
    use league_relay::transport::map_ureq_error;

    // 1. ApiKey::from_env wrapper, LiveGate::cap_credits, and unplanted token redaction.
    let _ = ApiKey::from_env();
    let gate = LiveGate::acquire(
        true,
        Some(1.75),
        ApiKey::from_env_reader(|_| Some(PLANTED_KEY.to_owned())),
    )?;
    assert!((gate.cap_credits() - 1.75).abs() < f64::EPSILON);

    let unplanted = redact_secrets(
        "Bearer plain_secret_tok and sk-or-v1-unplanted_secret_99 and Bearer   ",
        None,
    );
    assert!(!unplanted.contains("plain_secret_tok"));
    assert!(!unplanted.contains("unplanted_secret_99"));
    assert!(unplanted.contains("<REDACTED_API_KEY>"));

    // 2. map_ureq_error and NoopManualTransport.
    assert_eq!(
        map_ureq_error(ureq::Error::Timeout(ureq::Timeout::Global), gate.api_key()),
        TransportError::Timeout
    );
    let mapped_io = map_ureq_error(
        ureq::Error::Io(std::io::Error::other(format!(
            "socket broken for Bearer {PLANTED_KEY}"
        ))),
        gate.api_key(),
    );
    assert!(matches!(mapped_io, TransportError::Network(_)));
    assert!(!mapped_io.to_string().contains(PLANTED_KEY));

    assert!(
        NoopManualTransport
            .post_chat_completions(
                "https://openrouter.ai/api/v1/chat/completions",
                "{}",
                Duration::from_secs(1),
            )
            .is_err()
    );

    // 3. SeatFile::load missing file, malformed JSON/shapes, and missing numeric fields.
    let tmp = TempDir::new()?;
    assert!(SeatFile::load(&tmp.path().join("does_not_exist.json"), None).is_err());
    for bad_raw in [
        "{not-json",
        "[123]",
        r#"{"unknown_top_level":[]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b","temperature":0.5}]}"#,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"a/b","temperature":0.5,"max_output_tokens":64}]}"#,
    ] {
        assert!(SeatFile::from_json_str(bad_raw, None).is_err());
    }

    // 4. MockDirTransport invalid JSON, default status=200, and unmatched rule.
    let bad_mock_dir = tmp.path().join("bad_mock");
    fs::create_dir_all(&bad_mock_dir)?;
    fs::write(bad_mock_dir.join("responses.json"), "{bad")?;
    assert!(MockDirTransport::load(&bad_mock_dir).is_err());

    let mock_dir = tmp.path().join("ok_mock");
    fs::create_dir_all(&mock_dir)?;
    fs::write(
        mock_dir.join("responses.json"),
        serde_json::to_string(&json!({
            "rules": [{
                "when_contains": "MATCH_ME",
                "response": {
                    "id": "gen-default-status",
                    "model": "google/gemini-2.5-pro-06-05",
                    "choices": [{"message": {"content": "TRAVEL: stay"}}],
                    "usage": {"prompt_tokens": 12, "completion_tokens": 4, "total_tokens": 16, "cost": 0.0005}
                }
            }]
        }))?,
    )?;
    let loaded_mock = MockDirTransport::load(&mock_dir)?;
    let matched =
        loaded_mock.post_chat_completions("http://mock", "MATCH_ME", Duration::from_secs(1))?;
    assert_eq!(matched.status, 200);
    assert!(
        loaded_mock
            .post_chat_completions("http://mock", "NO_SUCH_RULE", Duration::from_secs(1))
            .is_err()
    );

    // 5. parse_args missing required flags and in-process execute_cli exit codes 0, 1, 2.
    assert!(parse_args(["turn"]).is_err());
    assert!(parse_args(["turn", "--game-dir=g"]).is_err());
    assert!(parse_args(["turn", "--game-dir=g", "--day=01"]).is_err());
    assert!(parse_args(["turn", "--game-dir=g", "--day=01", "--round=talk"]).is_err());

    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER\r\n\r\n")?;
    let seats_path = tmp.path().join("seats.json");
    fs::write(
        &seats_path,
        r#"{"seats":[{"guild":"Saltworks","backend":"openrouter","model":"google/gemini-2.5-pro-06-05","temperature":0.4,"max_output_tokens":128,"timeout_secs":2.0,"max_cost_per_call":0.05}]}"#,
    )?;

    let cli_game_ok = tmp.path().join("cli_game_ok");
    write_to_seat(&cli_game_ok, "01", "talk", "Saltworks", "MATCH_ME")?;
    let mut out_ok = Vec::new();
    let mut err_ok = Vec::new();
    let code_ok = execute_cli(
        [
            "turn",
            "--game-dir",
            cli_game_ok.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats_path.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut out_ok,
        &mut err_ok,
    );
    assert_eq!(code_ok, 0);
    assert!(String::from_utf8(out_ok)?.contains("outcome=OK"));

    // Unmatched rule causes NO_REPLY -> exit code 2.
    let cli_game_fail = tmp.path().join("cli_game_fail");
    write_to_seat(
        &cli_game_fail,
        "01",
        "talk",
        "Saltworks",
        "UNMATCHED_PROMPT",
    )?;
    let mut out_fail = Vec::new();
    let mut err_fail = Vec::new();
    let code_fail = execute_cli(
        [
            "turn",
            "--game-dir",
            cli_game_fail.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats_path.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
        ],
        |_| None,
        &mut out_fail,
        &mut err_fail,
    );
    assert_eq!(code_fail, 2);
    assert!(String::from_utf8(out_fail)?.contains("outcome=NO_REPLY"));

    // Refusal without --live on openrouter seat -> exit code 1.
    let mut out_ref = Vec::new();
    let mut err_ref = Vec::new();
    let code_ref = execute_cli(
        [
            "turn",
            "--game-dir",
            cli_game_ok.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "act",
            "--seats",
            seats_path.to_str().ok_or("utf8")?,
        ],
        |_| Some(PLANTED_KEY.to_owned()),
        &mut out_ref,
        &mut err_ref,
    );
    assert_eq!(code_ref, 1);

    // 6. run_turn validation, CallLedger pre-existing unknown/negative cost, JSON non-200,
    //    static/non-string panics, corrupt/unpaired transcripts, and format_paths_and_hashes arms.
    let seats = SeatFile::load(&seats_path, None)?;
    let dummy_tx = ScriptedTransport::new(vec![]);
    assert!(matches!(
        run_turn(
            &TurnConfig::new(
                tmp.path().join("g1"),
                "01".to_owned(),
                Round::Talk,
                seats.clone(),
                Some(wrapper.clone()),
                true,
                None,
            ),
            &dummy_tx,
            None,
        ),
        Err(RelayError::InvalidCap(_))
    ));
    assert!(matches!(
        run_turn(
            &TurnConfig::new(
                tmp.path().join("g2"),
                "01".to_owned(),
                Round::Talk,
                seats.clone(),
                Some(wrapper.clone()),
                false,
                Some(-0.1),
            ),
            &dummy_tx,
            None,
        ),
        Err(RelayError::InvalidCap(_))
    ));
    assert!(matches!(
        run_turn(
            &TurnConfig::new(
                tmp.path().join("g3"),
                "01".to_owned(),
                Round::Talk,
                seats.clone(),
                Some(tmp.path().join("missing_wrapper.md")),
                false,
                None,
            ),
            &dummy_tx,
            None,
        ),
        Err(RelayError::Io(_))
    ));

    // Pre-existing costless call on same (day, round) + blank line -> COST_UNKNOWN formatted output.
    let unknown_game = tmp.path().join("unknown_existing");
    write_to_seat(&unknown_game, "01", "talk", "Saltworks", "PROMPT")?;
    fs::create_dir_all(unknown_game.join("relay"))?;
    let prior_unknown = CallLogEntry {
        time_utc: "2026-11-15T00:00:00.000Z".to_owned(),
        day: "01".to_owned(),
        round: Round::Talk,
        guild: "Weavers".to_owned(),
        attempt: 1,
        trigger: "turn".to_owned(),
        model_requested: "google/gemini-2.5-pro-06-05".to_owned(),
        model_returned: Some("google/gemini-2.5-pro-06-05".to_owned()),
        request_body_sha256: "00".repeat(32),
        reply_sha256: Some("11".repeat(32)),
        prompt_tokens: Some(10),
        completion_tokens: Some(5),
        total_tokens: Some(15),
        cost: None,
        charged_credits: 0.05,
        latency_ms: 12,
        outcome: "OK".to_owned(),
        error: None,
    };
    fs::write(
        unknown_game.join("relay").join("calls.jsonl"),
        format!("\n{}\n", serde_json::to_string(&prior_unknown)?),
    )?;
    let unk_summary = run_turn(
        &TurnConfig::new(
            unknown_game,
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper.clone()),
            true,
            Some(2.0),
        ),
        &dummy_tx,
        None,
    )?;
    assert!(
        unk_summary
            .format_paths_and_hashes()
            .contains("outcome=COST_UNKNOWN")
    );

    // Pre-existing call reaching cap -> CAP_REACHED formatted output.
    let cap_game = tmp.path().join("cap_existing");
    write_to_seat(&cap_game, "01", "talk", "Saltworks", "PROMPT")?;
    fs::create_dir_all(cap_game.join("relay"))?;
    let mut prior_capped = prior_unknown.clone();
    prior_capped.cost = Some(2.0);
    prior_capped.charged_credits = 2.0;
    fs::write(
        cap_game.join("relay").join("calls.jsonl"),
        format!("{}\n", serde_json::to_string(&prior_capped)?),
    )?;
    let cap_summary = run_turn(
        &TurnConfig::new(
            cap_game,
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper.clone()),
            true,
            Some(2.0),
        ),
        &dummy_tx,
        None,
    )?;
    assert!(
        cap_summary
            .format_paths_and_hashes()
            .contains("outcome=CAP_REACHED")
    );

    // Negative cost in calls.jsonl -> SeatError.
    let neg_game = tmp.path().join("neg_cost_game");
    write_to_seat(&neg_game, "01", "talk", "Saltworks", "PROMPT")?;
    fs::create_dir_all(neg_game.join("relay"))?;
    let mut neg_entry = prior_unknown;
    neg_entry.day = "01".to_owned();
    neg_entry.round = Round::Act;
    neg_entry.cost = Some(-0.5);
    fs::write(
        neg_game.join("relay").join("calls.jsonl"),
        format!("{}\n", serde_json::to_string(&neg_entry)?),
    )?;
    let neg_summary = run_turn(
        &TurnConfig::new(
            neg_game,
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper.clone()),
            true,
            Some(2.0),
        ),
        &dummy_tx,
        None,
    )?;
    assert!(matches!(
        neg_summary.reports[0].outcome,
        SeatOutcome::SeatError { .. }
    ));

    // HTTP 429 with JSON body + static/&str and non-string panics.
    install_redacting_panic_hook(Some(gate.api_key().clone()));
    let panic_game = tmp.path().join("panic_variants");
    write_to_seat(&panic_game, "01", "talk", "Saltworks", "P1")?;
    let panic_tx_1 = ScriptedTransport::new(vec![
        ScriptStep::Respond(RawHttpResponse {
            status: 429,
            body: r#"{"model":"google/gemini-2.5-pro-06-05","usage":{"prompt_tokens":8,"total_tokens":8}}"#.to_owned(),
        }),
        ScriptStep::PanicStatic,
    ]);
    let p1_summary = run_turn(
        &TurnConfig::new(
            panic_game,
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper.clone()),
            false,
            None,
        ),
        &panic_tx_1,
        None,
    )?;
    assert!(matches!(
        p1_summary.reports[0].outcome,
        SeatOutcome::NoReply { .. }
    ));

    let panic_game_2 = tmp.path().join("panic_code");
    write_to_seat(&panic_game_2, "01", "talk", "Saltworks", "P2")?;
    let panic_tx_2 = ScriptedTransport::new(vec![ScriptStep::PanicCode, ScriptStep::PanicCode]);
    let p2_summary = run_turn(
        &TurnConfig::new(
            panic_game_2,
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper.clone()),
            false,
            None,
        ),
        &panic_tx_2,
        None,
    )?;
    assert!(matches!(
        p2_summary.reports[0].outcome,
        SeatOutcome::NoReply { .. }
    ));

    // Transcript with invalid JSON and transcript ending on unpaired user turn.
    let invalid_tx_game = tmp.path().join("invalid_tx_json");
    fs::create_dir_all(invalid_tx_game.join("relay").join("seats"))?;
    fs::write(
        invalid_tx_game
            .join("relay")
            .join("seats")
            .join("Saltworks.jsonl"),
        "\n{not-json}\n",
    )?;
    assert!(
        run_turn(
            &TurnConfig::new(
                invalid_tx_game,
                "01".to_owned(),
                Round::Act,
                seats.clone(),
                Some(wrapper.clone()),
                false,
                None,
            ),
            &dummy_tx,
            None,
        )
        .is_err()
    );

    let unpaired_tx_game = tmp.path().join("unpaired_tx");
    fs::create_dir_all(unpaired_tx_game.join("relay").join("seats"))?;
    fs::write(
        unpaired_tx_game
            .join("relay")
            .join("seats")
            .join("Saltworks.jsonl"),
        r#"{"day":"01","round":"talk","role":"user","content":"lonely"}"#,
    )?;
    assert!(
        run_turn(
            &TurnConfig::new(
                unpaired_tx_game,
                "01".to_owned(),
                Round::Act,
                seats,
                Some(wrapper),
                false,
                None,
            ),
            &dummy_tx,
            None,
        )
        .is_err()
    );

    Ok(())
}

struct SlowPricedTransport {
    calls: AtomicUsize,
    pause: Duration,
    cost_per_call: f64,
}

impl SlowPricedTransport {
    fn new(pause: Duration, cost_per_call: f64) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            pause,
            cost_per_call,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Transport for SlowPricedTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        _body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.pause);
        Ok(ok_completion_json(
            "TRAVEL: stay",
            "google/gemini-2.5-pro-06-05",
            Some(self.cost_per_call),
        ))
    }
}

struct OverEstimateSequencedTransport {
    calls: AtomicUsize,
    seat_a_returned: (Mutex<bool>, Condvar),
}

impl OverEstimateSequencedTransport {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            seat_a_returned: (Mutex::new(false), Condvar::new()),
        }
    }
}

impl Transport for OverEstimateSequencedTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if body.contains("SEAT_A_OVER_ESTIMATE") {
            let resp = ok_completion_json(
                "Saltworks reply",
                "google/gemini-2.5-pro-06-05",
                Some(0.025),
            );
            let (lock, cvar) = &self.seat_a_returned;
            if let Ok(mut done) = lock.lock() {
                *done = true;
                cvar.notify_all();
            }
            return Ok(resp);
        }
        let (lock, cvar) = &self.seat_a_returned;
        if let Ok(mut done) = lock.lock() {
            while !*done {
                let (next_guard, timeout_res) = cvar
                    .wait_timeout(done, Duration::from_secs(2))
                    .map_err(|_| TransportError::Network("condvar poisoned".to_owned()))?;
                done = next_guard;
                if timeout_res.timed_out() {
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(15));
        Err(TransportError::Network(
            "transient drop before over-estimate check".to_owned(),
        ))
    }
}

#[test]
fn m1_cap_reservation_blocks_concurrent_overshoot_resends_and_cost_over_estimate() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    // Part A: 4-seat reproducer from review (cap = 0.01, max_cost_per_call = 0.01, each call 0.01).
    // At most floor(0.01 / 0.01) = 1 call may start; recorded sum must be 0.01, not 0.04.
    let four_seats_raw = json!({
        "seats": [
            {
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.01
            },
            {
                "guild": "Weavers",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.01
            },
            {
                "guild": "Dyers",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.01
            },
            {
                "guild": "Tanners",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.01
            }
        ]
    })
    .to_string();
    let four_seats = SeatFile::from_json_str(&four_seats_raw, None)?;

    let game_cap_01 = tmp.path().join("four_seat_cap_01");
    for g in ["Saltworks", "Weavers", "Dyers", "Tanners"] {
        write_to_seat(&game_cap_01, "01", "act", g, "PROMPT")?;
    }
    let tx_01 = SlowPricedTransport::new(Duration::from_millis(45), 0.01);
    let summary_01 = run_turn(
        &TurnConfig::new(
            game_cap_01.clone(),
            "01".to_owned(),
            Round::Act,
            four_seats.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &tx_01,
        None,
    )?;
    assert_eq!(tx_01.call_count(), 1);
    let delivered_01 = summary_01
        .reports
        .iter()
        .filter(|r| matches!(r.outcome, SeatOutcome::Delivered { .. }))
        .count();
    let capped_01 = summary_01
        .reports
        .iter()
        .filter(|r| matches!(r.outcome, SeatOutcome::CapReached { .. }))
        .count();
    assert_eq!(delivered_01, 1);
    assert_eq!(capped_01, 3);
    let calls_01 = read_calls_log(&game_cap_01)?;
    let sum_01: f64 = calls_01.iter().filter_map(|c| c.cost).sum();
    assert!((sum_01 - 0.01).abs() < 1e-9);
    assert_eq!(
        calls_01
            .iter()
            .filter(|c| c.outcome == "CAP_REACHED")
            .count(),
        3
    );

    // Same 4 seats with cap = 0.025 -> floor(0.025 / 0.01) = 2 calls allowed.
    let game_cap_025 = tmp.path().join("four_seat_cap_025");
    for g in ["Saltworks", "Weavers", "Dyers", "Tanners"] {
        write_to_seat(&game_cap_025, "01", "act", g, "PROMPT")?;
    }
    let tx_025 = SlowPricedTransport::new(Duration::from_millis(45), 0.01);
    let summary_025 = run_turn(
        &TurnConfig::new(
            game_cap_025.clone(),
            "01".to_owned(),
            Round::Act,
            four_seats,
            Some(wrapper.clone()),
            true,
            Some(0.025),
        ),
        &tx_025,
        None,
    )?;
    assert_eq!(tx_025.call_count(), 2);
    assert_eq!(
        summary_025
            .reports
            .iter()
            .filter(|r| matches!(r.outcome, SeatOutcome::Delivered { .. }))
            .count(),
        2
    );

    // Part B: A harness re-send also reserves max_cost_per_call before calling.
    let one_seat_01 = SeatFile::from_json_str(
        &json!({
            "seats": [{
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.2,
                "max_output_tokens": 128,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.01
            }]
        })
        .to_string(),
        None,
    )?;
    let resend_game = tmp.path().join("resend_reserve_game");
    write_to_seat(&resend_game, "01", "talk", "Saltworks", "RESEND_PROMPT")?;
    let resend_tx = ScriptedTransport::new(vec![
        ScriptStep::Respond(ok_completion_json(
            "   ",
            "google/gemini-2.5-pro-06-05",
            Some(0.008),
        )),
        ScriptStep::Respond(ok_completion_json(
            "Should not be sent",
            "google/gemini-2.5-pro-06-05",
            Some(0.008),
        )),
    ]);
    let resend_summary = run_turn(
        &TurnConfig::new(
            resend_game.clone(),
            "01".to_owned(),
            Round::Talk,
            one_seat_01.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.015),
        ),
        &resend_tx,
        None,
    )?;
    assert_eq!(resend_tx.call_count(), 1);
    assert!(matches!(
        resend_summary.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));
    let resend_calls = read_calls_log(&resend_game)?;
    assert_eq!(resend_calls.len(), 2);
    assert_eq!(resend_calls[0].outcome, "harness");
    assert_eq!(resend_calls[1].attempt, 2);
    assert_eq!(resend_calls[1].trigger, "harness");
    assert_eq!(resend_calls[1].outcome, "CAP_REACHED");

    // Part C: Returned cost exceeding max_cost_per_call logs COST_OVER_ESTIMATE and fails closed.
    let over_game = tmp.path().join("over_estimate_game");
    write_to_seat(
        &over_game,
        "01",
        "talk",
        "Saltworks",
        "SEAT_A_OVER_ESTIMATE",
    )?;
    write_to_seat(&over_game, "01", "talk", "Weavers", "SEAT_B_AFTER_OVER")?;
    let two_seats_01 = SeatFile::from_json_str(
        &json!({
            "seats": [
                {
                    "guild": "Saltworks",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.2,
                    "max_output_tokens": 128,
                    "timeout_secs": 3.0,
                    "max_cost_per_call": 0.01
                },
                {
                    "guild": "Weavers",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.2,
                    "max_output_tokens": 128,
                    "timeout_secs": 3.0,
                    "max_cost_per_call": 0.01
                }
            ]
        })
        .to_string(),
        None,
    )?;
    let over_tx = OverEstimateSequencedTransport::new();
    let over_summary = run_turn(
        &TurnConfig::new(
            over_game.clone(),
            "01".to_owned(),
            Round::Talk,
            two_seats_01,
            Some(wrapper.clone()),
            true,
            Some(1.0),
        ),
        &over_tx,
        None,
    )?;
    assert!(matches!(
        over_summary.reports[0].outcome,
        SeatOutcome::Delivered { .. }
    ));
    assert!(matches!(
        over_summary.reports[1].outcome,
        SeatOutcome::CostOverEstimateRefused { .. }
    ));
    assert!(
        over_summary
            .format_paths_and_hashes()
            .contains("seat=Weavers outcome=COST_OVER_ESTIMATE")
    );
    let over_calls = read_calls_log(&over_game)?;
    let salt_over = over_calls
        .iter()
        .find(|c| c.guild == "Saltworks")
        .ok_or("missing Saltworks over-estimate entry")?;
    assert_eq!(salt_over.cost, Some(0.025));
    assert_eq!(salt_over.outcome, "COST_OVER_ESTIMATE");

    // Re-opening CallLedger on the same (day, round) also sees COST_OVER_ESTIMATE and refuses pre-call.
    write_to_seat(&over_game, "01", "talk", "Dyers", "LATER_SEAT")?;
    let dyers_seat = SeatFile::from_json_str(
        &json!({
            "seats": [{
                "guild": "Dyers",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.2,
                "max_output_tokens": 128,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.01
            }]
        })
        .to_string(),
        None,
    )?;
    let unused_tx = ScriptedTransport::new(vec![]);
    let reopened = run_turn(
        &TurnConfig::new(
            over_game,
            "01".to_owned(),
            Round::Talk,
            dyers_seat,
            Some(wrapper),
            true,
            Some(1.0),
        ),
        &unused_tx,
        None,
    )?;
    assert_eq!(unused_tx.call_count(), 0);
    assert!(matches!(
        reopened.reports[0].outcome,
        SeatOutcome::CostOverEstimateRefused { .. }
    ));

    Ok(())
}

fn assert_no_8char_key_piece(haystack: &str, secret: &str, surface: &str) {
    let chars: Vec<char> = secret.chars().collect();
    for window in chars.windows(8) {
        let piece: String = window.iter().collect();
        assert!(
            !haystack.contains(&piece),
            "{surface} leaked 8-char key substring '{piece}': {haystack}"
        );
    }
}

#[test]
fn m2_redacts_before_truncation_and_scrubs_8_char_key_substrings() -> TestResult {
    // Non-sk-or key so prefix token matching cannot hide truncation or substring bugs.
    const STRADDLE_KEY: &str = "OrKey9876AlphaBravoCharlieDeltaEcho4321";
    let key = ApiKey::from_env_reader(|_| Some(STRADDLE_KEY.to_owned()))?;

    // Short (<8 char) key branch and truncate_after_redaction short-input branch.
    let short_key = ApiKey::from_env_reader(|_| Some("Short77".to_owned()))?;
    assert_eq!(
        redact_secrets("token=Short77!", Some(&short_key)),
        "token=<REDACTED_API_KEY>!"
    );
    assert_eq!(truncate_after_redaction("short", 20), "short");

    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("m2_game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;
    write_to_seat(&game_dir, "01", "talk", "Saltworks", "M2_PROMPT_FAIL")?;
    write_to_seat(&game_dir, "01", "talk", "Weavers", "M2_PROMPT_OK")?;

    let seats = SeatFile::from_json_str(
        &json!({
            "seats": [
                {
                    "guild": "Saltworks",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.2,
                    "max_output_tokens": 128,
                    "timeout_secs": 3.0,
                    "max_cost_per_call": 0.05
                },
                {
                    "guild": "Weavers",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.2,
                    "max_output_tokens": 128,
                    "timeout_secs": 3.0,
                    "max_cost_per_call": 0.05
                }
            ]
        })
        .to_string(),
        Some(&key),
    )?;

    // Attempt 1 plants the full key starting at char 194, straddling the 200-char HTTP error cut.
    // Attempt 2 plants only a 10-char interior slice of the key at char 195, also straddling char 200.
    let body_full_straddle = format!("{}{STRADDLE_KEY} trailing_detail", "a".repeat(194));
    let partial_slice = &STRADDLE_KEY[9..19]; // "AlphaBravo" (10 chars)
    let body_partial_straddle = format!("{}{partial_slice} trailing_detail", "b".repeat(195));

    struct M2Transport {
        body_1: String,
        body_2: String,
        salt_calls: AtomicUsize,
    }

    impl Transport for M2Transport {
        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            if body.contains("M2_PROMPT_OK") {
                return Ok(ok_completion_json(
                    "To Saltworks: greetings.",
                    "google/gemini-2.5-pro-06-05",
                    Some(0.001),
                ));
            }
            let n = self.salt_calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(RawHttpResponse {
                    status: 500,
                    body: self.body_1.clone(),
                })
            } else {
                Ok(RawHttpResponse {
                    status: 502,
                    body: self.body_2.clone(),
                })
            }
        }
    }

    let m2_tx = M2Transport {
        body_1: body_full_straddle,
        body_2: body_partial_straddle,
        salt_calls: AtomicUsize::new(0),
    };
    let summary = run_turn(
        &TurnConfig::new(
            game_dir.clone(),
            "01".to_owned(),
            Round::Talk,
            seats,
            Some(wrapper),
            false,
            Some(1.0),
        ),
        &m2_tx,
        Some(&key),
    )?;

    let calls_raw = fs::read_to_string(game_dir.join("relay").join("calls.jsonl"))?;
    let weavers_tx_raw =
        fs::read_to_string(game_dir.join("relay").join("seats").join("Weavers.jsonl"))?;
    let summary_both = format!("{}\n{summary:?}", summary.format_paths_and_hashes());

    assert_no_8char_key_piece(&calls_raw, STRADDLE_KEY, "calls.jsonl");
    assert_no_8char_key_piece(&weavers_tx_raw, STRADDLE_KEY, "Weavers transcript");
    assert_no_8char_key_piece(&summary_both, STRADDLE_KEY, "TurnSummary Display/Debug");

    let calls_parsed = read_calls_log(&game_dir)?;
    for salt_entry in calls_parsed.iter().filter(|c| c.guild == "Saltworks") {
        let err_msg = salt_entry.error.as_deref().unwrap_or("");
        assert!(
            err_msg.contains("<REDACTED_API_KEY>"),
            "expected intact redaction marker in '{err_msg}'"
        );
    }

    // CLI shortened unknown flag straddling char 48 and key containing '=' straddling '='.
    let cli_straddle_flag = format!("--{}{}", "z".repeat(42), &STRADDLE_KEY[4..16]);
    let cli_err = parse_args_with_key(["turn", &cli_straddle_flag], Some(&key))
        .err()
        .ok_or("expected CLI error")?;
    assert_no_8char_key_piece(
        &format!("{cli_err}\n{cli_err:?}"),
        STRADDLE_KEY,
        "CLI RelayError Display/Debug",
    );

    let eq_secret = "KeyHead=Tail987654";
    let eq_key = ApiKey::from_env_reader(|_| Some(eq_secret.to_owned()))?;
    let eq_err = parse_args_with_key(["turn", &format!("--{eq_secret}")], Some(&eq_key))
        .err()
        .ok_or("expected CLI error")?;
    assert_no_8char_key_piece(
        &format!("{eq_err}\n{eq_err:?}"),
        eq_secret,
        "CLI '=' key RelayError Display/Debug",
    );

    // SeatFile and MockDirTransport error Display/Debug with partial key substring.
    let seat_bad = SeatFile::from_json_str(
        &format!(r#"{{"{}": []}}"#, &STRADDLE_KEY[15..27]),
        Some(&key),
    )
    .err()
    .ok_or("expected seat error")?;
    assert_no_8char_key_piece(
        &format!("{seat_bad}\n{seat_bad:?}"),
        STRADDLE_KEY,
        "SeatFile RelayError Display/Debug",
    );

    Ok(())
}

#[test]
fn s1_late_reply_after_timeout_logs_late_discarded_and_counts_against_cap() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    let fast_seat = SeatFile::from_json_str(
        &json!({
            "seats": [{
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.3,
                "max_output_tokens": 256,
                "timeout_secs": 0.03,
                "max_cost_per_call": 0.01
            }]
        })
        .to_string(),
        None,
    )?;

    // Case 1: cap = 0.01. Attempt 1 replies after timeout with cost 0.01 -> logged as
    // LATE_DISCARDED with cost and tokens, and counted against cap so the harness re-send
    // is refused pre-call with CAP_REACHED.
    let game_capped = tmp.path().join("late_capped");
    write_to_seat(&game_capped, "01", "talk", "Saltworks", "LATE_PROMPT_1")?;
    let tx_capped = ScriptedTransport::new(vec![
        ScriptStep::SleepThenRespond(
            Duration::from_millis(95),
            ok_completion_json("Late reply 1", "google/gemini-2.5-pro-06-05", Some(0.01)),
        ),
        ScriptStep::Respond(ok_completion_json(
            "Should never be sent",
            "google/gemini-2.5-pro-06-05",
            Some(0.01),
        )),
    ]);
    let summary_capped = run_turn(
        &TurnConfig::new(
            game_capped.clone(),
            "01".to_owned(),
            Round::Talk,
            fast_seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &tx_capped,
        None,
    )?;
    assert_eq!(tx_capped.call_count(), 1);
    assert!(matches!(
        summary_capped.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));
    assert!(
        !game_capped
            .join("from-seat")
            .join("day-01-talk-Saltworks.md")
            .exists()
    );
    let calls_capped = read_calls_log(&game_capped)?;
    assert_eq!(calls_capped.len(), 2);
    assert_eq!(calls_capped[0].outcome, "LATE_DISCARDED");
    assert_eq!(calls_capped[0].cost, Some(0.01));
    assert_eq!(calls_capped[0].prompt_tokens, Some(110));
    assert_eq!(calls_capped[0].completion_tokens, Some(25));
    assert_eq!(calls_capped[0].total_tokens, Some(135));
    assert_eq!(calls_capped[0].reply_sha256, None);
    assert_eq!(calls_capped[1].attempt, 2);
    assert_eq!(calls_capped[1].trigger, "harness");
    assert_eq!(calls_capped[1].outcome, "CAP_REACHED");

    // Case 2: cap = 0.02. Attempt 1 replies late (LATE_DISCARDED, 0.01), Attempt 2 replies on time (OK, 0.01).
    let game_retry_ok = tmp.path().join("late_then_ok");
    write_to_seat(&game_retry_ok, "01", "talk", "Saltworks", "LATE_PROMPT_2")?;
    let tx_retry_ok = ScriptedTransport::new(vec![
        ScriptStep::SleepThenRespond(
            Duration::from_millis(95),
            ok_completion_json("Late reply", "google/gemini-2.5-pro-06-05", Some(0.01)),
        ),
        ScriptStep::Respond(ok_completion_json(
            "TRAVEL: stay",
            "google/gemini-2.5-pro-06-05",
            Some(0.01),
        )),
    ]);
    let summary_ok = run_turn(
        &TurnConfig::new(
            game_retry_ok.clone(),
            "01".to_owned(),
            Round::Talk,
            fast_seat,
            Some(wrapper),
            true,
            Some(0.02),
        ),
        &tx_retry_ok,
        None,
    )?;
    assert!(summary_ok.all_clean());
    assert_eq!(tx_retry_ok.call_count(), 2);
    let calls_ok = read_calls_log(&game_retry_ok)?;
    assert_eq!(calls_ok.len(), 2);
    assert_eq!(calls_ok[0].outcome, "LATE_DISCARDED");
    assert_eq!(calls_ok[1].outcome, "OK");
    assert_eq!(
        fs::read_to_string(
            game_retry_ok
                .join("from-seat")
                .join("day-01-talk-Saltworks.md")
        )?,
        "TRAVEL: stay"
    );

    Ok(())
}

#[test]
fn s2_binary_panic_hook_redacts_planted_key_on_stderr() -> TestResult {
    const PANIC_SECRET: &str = "PlainPanicSecretKey_ABCDEFGH_987654321";

    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("panic_bin_game");
    let mock_dir = tmp.path().join("panic_mock");
    fs::create_dir_all(&mock_dir)?;
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    let seats_path = tmp.path().join("seats.json");
    fs::write(
        &seats_path,
        serde_json::to_string(&json!({
            "seats": [{
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.4,
                "max_output_tokens": 256,
                "timeout_secs": 5.0,
                "max_cost_per_call": 0.05
            }]
        }))?,
    )?;
    write_to_seat(&game_dir, "01", "talk", "Saltworks", "TRIGGER_PANIC_MOCK")?;

    fs::write(
        mock_dir.join("responses.json"),
        serde_json::to_string(&json!({
            "rules": [{
                "when_contains": "TRIGGER_PANIC_MOCK",
                "panic_message": format!("mock transport boom with Bearer {PLANTED_KEY} and {PANIC_SECRET}")
            }]
        }))?,
    )?;

    let bin = env!("CARGO_BIN_EXE_league-relay");
    let output = Command::new(bin)
        .args([
            "turn",
            "--game-dir",
            game_dir.to_str().ok_or("utf8")?,
            "--day",
            "01",
            "--round",
            "talk",
            "--seats",
            seats_path.to_str().ok_or("utf8")?,
            "--wrapper",
            wrapper.to_str().ok_or("utf8")?,
            "--mock-dir",
            mock_dir.to_str().ok_or("utf8")?,
            "--cap-credits",
            "1.0",
        ])
        .env("OPENROUTER_API_KEY", PANIC_SECRET)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr_text = String::from_utf8(output.stderr)?;
    let stdout_text = String::from_utf8(output.stdout)?;
    let calls_raw = fs::read_to_string(game_dir.join("relay").join("calls.jsonl"))?;

    assert!(
        stderr_text.contains("panic at") && stderr_text.contains("<REDACTED_API_KEY>"),
        "expected redacted panic hook output on stderr, got: {stderr_text}"
    );
    assert!(!stderr_text.contains(PLANTED_KEY));
    assert_no_8char_key_piece(&stderr_text, PANIC_SECRET, "binary panic stderr");
    assert_no_8char_key_piece(&stdout_text, PANIC_SECRET, "binary panic stdout");
    assert_no_8char_key_piece(&calls_raw, PANIC_SECRET, "binary panic calls.jsonl");

    Ok(())
}

#[test]
fn s3_guild_named_calls_uses_relay_seats_subdir_without_collision() -> TestResult {
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("calls_guild_game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    let seats = single_openrouter_seats("calls", 5.0)?;

    write_to_seat(&game_dir, "01", "talk", "calls", "DAY 01 TALK FOR CALLS")?;
    let t1 = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "To Smiths: hello from calls guild.",
        "google/gemini-2.5-pro-06-05",
        Some(0.002),
    ))]);
    let s1 = run_turn(
        &TurnConfig::new(
            game_dir.clone(),
            "01".to_owned(),
            Round::Talk,
            seats.clone(),
            Some(wrapper),
            false,
            Some(1.0),
        ),
        &t1,
        None,
    )?;
    assert!(s1.all_clean());

    write_to_seat(&game_dir, "01", "act", "calls", "DAY 01 ACT FOR CALLS")?;
    let t2 = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "TRAVEL: stay",
        "google/gemini-2.5-pro-06-05",
        Some(0.002),
    ))]);
    let s2 = run_turn(
        &TurnConfig::new(
            game_dir.clone(),
            "01".to_owned(),
            Round::Act,
            seats,
            None,
            false,
            Some(1.0),
        ),
        &t2,
        None,
    )?;
    assert!(s2.all_clean());

    let seat_transcript = read_transcript_log(&game_dir, "calls")?;
    assert_eq!(seat_transcript.len(), 4);
    assert_eq!(seat_transcript[0].role, "user");
    assert_eq!(seat_transcript[1].role, "assistant");
    assert_eq!(seat_transcript[2].role, "user");
    assert_eq!(seat_transcript[3].role, "assistant");

    let call_ledger = read_calls_log(&game_dir)?;
    assert_eq!(call_ledger.len(), 2);
    assert!(
        call_ledger
            .iter()
            .all(|c| c.guild == "calls" && c.outcome == "OK")
    );

    Ok(())
}

#[test]
fn s4_guild_names_enforce_ascii_letter_prefix_and_case_insensitive_uniqueness() -> TestResult {
    // Case-insensitive duplicates are rejected on NTFS and everywhere else.
    for (g1, g2) in [("Saltworks", "saltworks"), ("Weavers", "WEAVERS")] {
        let dup_case = json!({
            "seats": [
                {"guild": g1, "backend": "file"},
                {"guild": g2, "backend": "file"}
            ]
        })
        .to_string();
        assert!(
            SeatFile::from_json_str(&dup_case, None).is_err(),
            "expected case-colliding guilds '{g1}' and '{g2}' to be rejected"
        );
    }

    // Names not matching [A-Za-z][A-Za-z0-9_-]* are rejected.
    for invalid_guild in [
        "",
        "1Saltworks",
        "-Saltworks",
        "_Saltworks",
        "Salt.works",
        "Salt/works",
        "Salt works",
    ] {
        let bad = json!({
            "seats": [{"guild": invalid_guild, "backend": "file"}]
        })
        .to_string();
        assert!(
            SeatFile::from_json_str(&bad, None).is_err(),
            "expected guild '{invalid_guild}' to be rejected"
        );
    }

    // Valid [A-Za-z][A-Za-z0-9_-]* names are accepted.
    let valid = json!({
        "seats": [
            {"guild": "Saltworks", "backend": "file"},
            {"guild": "calls", "backend": "file"},
            {"guild": "Guild_2-alpha", "backend": "file"},
            {"guild": "A", "backend": "file"}
        ]
    })
    .to_string();
    let parsed = SeatFile::from_json_str(&valid, None)?;
    assert_eq!(parsed.seats.len(), 4);

    Ok(())
}

#[test]
fn b1_ledger_write_failure_keeps_charge_and_stops_turn() -> TestResult {
    struct M1gTransientDirBlockTransport {
        calls_path: std::path::PathBuf,
        billed_calls: AtomicUsize,
        arrivals: (Mutex<usize>, Condvar),
        seat_a_returned: (Mutex<bool>, Condvar),
    }

    impl Transport for M1gTransientDirBlockTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            // Wait until both seats have reserved against cap = 0.02.
            let (arr_lock, arr_cvar) = &self.arrivals;
            if let Ok(mut n) = arr_lock.lock() {
                *n += 1;
                if *n >= 2 {
                    arr_cvar.notify_all();
                } else {
                    while *n < 2 {
                        let (next, res) = arr_cvar
                            .wait_timeout(n, Duration::from_secs(2))
                            .map_err(|_| TransportError::Network("condvar poisoned".to_owned()))?;
                        n = next;
                        if res.timed_out() {
                            break;
                        }
                    }
                }
            }

            self.billed_calls.fetch_add(1, Ordering::SeqCst);

            if body.contains("B1_SEAT_A") {
                // Block calls.jsonl creation/append by placing a directory at calls.jsonl.
                let _ = fs::create_dir_all(&self.calls_path);
                let resp = ok_completion_json(
                    "Saltworks reply",
                    "google/gemini-2.5-pro-06-05",
                    Some(0.01),
                );
                let (done_lock, done_cvar) = &self.seat_a_returned;
                if let Ok(mut done) = done_lock.lock() {
                    *done = true;
                    done_cvar.notify_all();
                }
                return Ok(resp);
            }

            // Seat B waits for Seat A to return and hit the failing ledger append,
            // then clears the directory block and returns a paid empty reply (0.01).
            let (done_lock, done_cvar) = &self.seat_a_returned;
            if let Ok(mut done) = done_lock.lock() {
                while !*done {
                    let (next, res) = done_cvar
                        .wait_timeout(done, Duration::from_secs(2))
                        .map_err(|_| TransportError::Network("condvar poisoned".to_owned()))?;
                    done = next;
                    if res.timed_out() {
                        break;
                    }
                }
            }
            thread::sleep(Duration::from_millis(30));
            let _ = fs::remove_dir(&self.calls_path);
            Ok(ok_completion_json(
                "   ",
                "google/gemini-2.5-pro-06-05",
                Some(0.01),
            ))
        }
    }

    let tmp = TempDir::new()?;
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    let two_seats = SeatFile::from_json_str(
        &json!({
            "seats": [
                {
                    "guild": "Saltworks",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.2,
                    "max_output_tokens": 128,
                    "timeout_secs": 3.0,
                    "max_cost_per_call": 0.01
                },
                {
                    "guild": "Weavers",
                    "backend": "openrouter",
                    "model": "google/gemini-2.5-pro-06-05",
                    "temperature": 0.2,
                    "max_output_tokens": 128,
                    "timeout_secs": 3.0,
                    "max_cost_per_call": 0.01
                }
            ]
        })
        .to_string(),
        None,
    )?;

    // Sub-case 1: Reviewer's M1g probe — cap = 0.02, two seats, max_cost_per_call = 0.01.
    // Seat A's append fails; Seat B clears the block and returns a paid empty reply.
    // Seat B's harness re-send MUST be refused fail-closed: <= 2 billed calls, and
    // recorded/charged sum >= real spend (0.02).
    let m1g_game = tmp.path().join("b1_m1g_game");
    write_to_seat(&m1g_game, "01", "talk", "Saltworks", "B1_SEAT_A")?;
    write_to_seat(&m1g_game, "01", "talk", "Weavers", "B1_SEAT_B")?;
    let calls_path = m1g_game.join("relay").join("calls.jsonl");
    let m1g_tx = M1gTransientDirBlockTransport {
        calls_path: calls_path.clone(),
        billed_calls: AtomicUsize::new(0),
        arrivals: (Mutex::new(0), Condvar::new()),
        seat_a_returned: (Mutex::new(false), Condvar::new()),
    };

    let m1g_summary = run_turn(
        &TurnConfig::new(
            m1g_game.clone(),
            "01".to_owned(),
            Round::Talk,
            two_seats.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.02),
        ),
        &m1g_tx,
        None,
    )?;

    let billed = m1g_tx.billed_calls.load(Ordering::SeqCst);
    assert_eq!(billed, 2, "expected at most 2 billed calls, got {billed}");
    assert!(matches!(
        m1g_summary.reports[0].outcome,
        SeatOutcome::SeatError { .. }
    ));
    assert!(matches!(
        m1g_summary.reports[1].outcome,
        SeatOutcome::SeatError { .. }
    ));
    let m1g_entries = read_calls_log(&m1g_game)?;
    let charged_total: f64 = m1g_entries.iter().map(|e| e.charged_credits).sum();
    let recorded_cost_total: f64 = m1g_entries.iter().filter_map(|e| e.cost).sum();
    assert!(
        charged_total + 1e-9 >= 0.02 && recorded_cost_total + 1e-9 >= 0.02,
        "charged={charged_total}, recorded={recorded_cost_total} must be >= real spend 0.02"
    );
    assert!(m1g_entries.iter().any(|e| e.outcome == "LEDGER_ERROR"));

    // Sub-case 2: Permanent ledger write failure stops any subsequent call in the turn even when cap = 1.0.
    struct PermanentBlockTransport {
        calls_path: std::path::PathBuf,
        calls: AtomicUsize,
    }
    impl Transport for PermanentBlockTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let _ = fs::create_dir_all(&self.calls_path);
            Ok(ok_completion_json(
                "   ",
                "google/gemini-2.5-pro-06-05",
                Some(0.01),
            ))
        }
    }

    let perm_game = tmp.path().join("b1_perm_game");
    let one_seat = SeatFile::from_json_str(
        &json!({
            "seats": [{
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.2,
                "max_output_tokens": 128,
                "timeout_secs": 3.0,
                "max_cost_per_call": 0.01
            }]
        })
        .to_string(),
        None,
    )?;
    write_to_seat(&perm_game, "01", "talk", "Saltworks", "B1_PERM")?;
    let perm_tx = PermanentBlockTransport {
        calls_path: perm_game.join("relay").join("calls.jsonl"),
        calls: AtomicUsize::new(0),
    };
    let perm_summary = run_turn(
        &TurnConfig::new(
            perm_game,
            "01".to_owned(),
            Round::Talk,
            one_seat,
            Some(wrapper),
            true,
            Some(1.0),
        ),
        &perm_tx,
        None,
    )?;
    assert_eq!(perm_tx.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        perm_summary.reports[0].outcome,
        SeatOutcome::SeatError { .. }
    ));

    Ok(())
}

#[test]
fn b2_sent_calls_without_cost_charge_max_and_rerun_agrees() -> TestResult {
    let tmp = TempDir::new()?;
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;

    let seat = SeatFile::from_json_str(
        &json!({
            "seats": [{
                "guild": "Saltworks",
                "backend": "openrouter",
                "model": "google/gemini-2.5-pro-06-05",
                "temperature": 0.2,
                "max_output_tokens": 128,
                "timeout_secs": 0.03,
                "max_cost_per_call": 0.01
            }]
        })
        .to_string(),
        None,
    )?;

    // Path (a): Late HTTP 200 with NO usage block (LATE_DISCARDED, cost: null) charges max_cost_per_call (0.01).
    let game_a = tmp.path().join("b2_path_a_late_no_usage");
    write_to_seat(&game_a, "01", "talk", "Saltworks", "B2_A")?;
    let tx_a = ScriptedTransport::new(vec![
        ScriptStep::SleepThenRespond(
            Duration::from_millis(95),
            RawHttpResponse {
                status: 200,
                body: r#"{"id":"gen-a","choices":[{"message":{"role":"assistant","content":"Late no usage"}}]}"#
                    .to_owned(),
            },
        ),
        ScriptStep::Respond(ok_completion_json(
            "Should be blocked by cap",
            "google/gemini-2.5-pro-06-05",
            Some(0.01),
        )),
    ]);
    let sum_a = run_turn(
        &TurnConfig::new(
            game_a.clone(),
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &tx_a,
        None,
    )?;
    assert_eq!(tx_a.call_count(), 1);
    assert!(matches!(
        sum_a.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));
    let calls_a = read_calls_log(&game_a)?;
    assert_eq!(calls_a.len(), 2);
    assert_eq!(calls_a[0].outcome, "LATE_DISCARDED");
    assert_eq!(calls_a[0].cost, None);
    assert!((calls_a[0].charged_credits - 0.01).abs() < 1e-9);
    assert_eq!(calls_a[1].outcome, "CAP_REACHED");
    assert!((calls_a[1].charged_credits - 0.0).abs() < 1e-9);

    // Re-run of the same turn from calls.jsonl sees charged_sum = 0.01 (CAP_REACHED at cap 0.01,
    // and allowed 1 call at cap 0.02 — agreeing with live that tokenless LATE_DISCARDED is NOT COST_UNKNOWN).
    let rerun_a_capped_tx = ScriptedTransport::new(vec![]);
    let rerun_a_capped = run_turn(
        &TurnConfig::new(
            game_a.clone(),
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &rerun_a_capped_tx,
        None,
    )?;
    assert_eq!(rerun_a_capped_tx.call_count(), 0);
    assert!(matches!(
        rerun_a_capped.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));

    let rerun_a_02_tx = ScriptedTransport::new(vec![ScriptStep::Respond(ok_completion_json(
        "Delivered on rerun",
        "google/gemini-2.5-pro-06-05",
        Some(0.01),
    ))]);
    let rerun_a_02 = run_turn(
        &TurnConfig::new(
            game_a.clone(),
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.02),
        ),
        &rerun_a_02_tx,
        None,
    )?;
    assert!(rerun_a_02.all_clean());
    assert_eq!(rerun_a_02_tx.call_count(), 1);

    // Path (b): On-time empty HTTP 200 with NO usage block charges max_cost_per_call (0.01).
    let game_b = tmp.path().join("b2_path_b_empty_no_usage");
    write_to_seat(&game_b, "01", "talk", "Saltworks", "B2_B")?;
    let tx_b = ScriptedTransport::new(vec![
        ScriptStep::Respond(RawHttpResponse {
            status: 200,
            body: r#"{"id":"gen-b","choices":[{"message":{"role":"assistant","content":"   "}}]}"#
                .to_owned(),
        }),
        ScriptStep::Respond(ok_completion_json(
            "Should be blocked by cap",
            "google/gemini-2.5-pro-06-05",
            Some(0.01),
        )),
    ]);
    let sum_b = run_turn(
        &TurnConfig::new(
            game_b.clone(),
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &tx_b,
        None,
    )?;
    assert_eq!(tx_b.call_count(), 1);
    assert!(matches!(
        sum_b.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));
    let calls_b = read_calls_log(&game_b)?;
    assert_eq!(calls_b[0].outcome, "harness");
    assert_eq!(calls_b[0].cost, None);
    assert!((calls_b[0].charged_credits - 0.01).abs() < 1e-9);
    assert_eq!(calls_b[1].outcome, "CAP_REACHED");

    let rerun_b_tx = ScriptedTransport::new(vec![]);
    let rerun_b = run_turn(
        &TurnConfig::new(
            game_b,
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &rerun_b_tx,
        None,
    )?;
    assert_eq!(rerun_b_tx.call_count(), 0);
    assert!(matches!(
        rerun_b.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));

    // Path (c): Client timeout before re-send charges max_cost_per_call (0.01), then cap binds.
    let game_c = tmp.path().join("b2_path_c_timeout");
    write_to_seat(&game_c, "01", "talk", "Saltworks", "B2_C")?;
    let tx_c = ScriptedTransport::new(vec![
        ScriptStep::Fail(TransportError::Timeout),
        ScriptStep::Respond(ok_completion_json(
            "Should be blocked by cap",
            "google/gemini-2.5-pro-06-05",
            Some(0.01),
        )),
    ]);
    let sum_c = run_turn(
        &TurnConfig::new(
            game_c.clone(),
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &tx_c,
        None,
    )?;
    assert_eq!(tx_c.call_count(), 1);
    assert!(matches!(
        sum_c.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));
    let calls_c = read_calls_log(&game_c)?;
    assert_eq!(calls_c[0].outcome, "harness");
    assert_eq!(calls_c[0].error.as_deref(), Some("timeout"));
    assert_eq!(calls_c[0].cost, None);
    assert!((calls_c[0].charged_credits - 0.01).abs() < 1e-9);
    assert_eq!(calls_c[1].outcome, "CAP_REACHED");

    let rerun_c_tx = ScriptedTransport::new(vec![]);
    let rerun_c = run_turn(
        &TurnConfig::new(
            game_c.clone(),
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(0.01),
        ),
        &rerun_c_tx,
        None,
    )?;
    assert_eq!(rerun_c_tx.call_count(), 0);
    assert!(matches!(
        rerun_c.reports[0].outcome,
        SeatOutcome::CapReached { .. }
    ));
    let calls_c_after = read_calls_log(&game_c)?;
    assert!(
        calls_c_after[2]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("charged 0.010000")
    );

    // Corrupt negative charged_credits and re-opening on existing COST_UNKNOWN refusal entry.
    let bad_charged_game = tmp.path().join("b2_neg_charged");
    write_to_seat(&bad_charged_game, "01", "talk", "Saltworks", "NEG")?;
    fs::create_dir_all(bad_charged_game.join("relay"))?;
    let mut bad_entry = calls_c[0].clone();
    bad_entry.charged_credits = -0.01;
    fs::write(
        bad_charged_game.join("relay").join("calls.jsonl"),
        format!("{}\n", serde_json::to_string(&bad_entry)?),
    )?;
    let bad_summary = run_turn(
        &TurnConfig::new(
            bad_charged_game,
            "01".to_owned(),
            Round::Talk,
            seat.clone(),
            Some(wrapper.clone()),
            true,
            Some(1.0),
        ),
        &rerun_c_tx,
        None,
    )?;
    assert!(matches!(
        bad_summary.reports[0].outcome,
        SeatOutcome::SeatError { .. }
    ));

    let prior_cu_game = tmp.path().join("b2_prior_cu");
    write_to_seat(&prior_cu_game, "01", "talk", "Saltworks", "CU")?;
    fs::create_dir_all(prior_cu_game.join("relay"))?;
    let mut cu_refusal = calls_c[1].clone();
    cu_refusal.outcome = "COST_UNKNOWN".to_owned();
    fs::write(
        prior_cu_game.join("relay").join("calls.jsonl"),
        format!("{}\n", serde_json::to_string(&cu_refusal)?),
    )?;
    let cu_summary = run_turn(
        &TurnConfig::new(
            prior_cu_game,
            "01".to_owned(),
            Round::Talk,
            seat,
            Some(wrapper),
            true,
            Some(1.0),
        ),
        &rerun_c_tx,
        None,
    )?;
    assert!(matches!(
        cu_summary.reports[0].outcome,
        SeatOutcome::CostUnknownRefused { .. }
    ));

    Ok(())
}

#[test]
fn c4_caps_error_body_at_4kib_before_redaction_and_truncation() -> TestResult {
    const BOUNDARY_KEY: &str = "Plain4KiBKey_AlphaBravoCharlieDelta9876";
    let key = ApiKey::from_env_reader(|_| Some(BOUNDARY_KEY.to_owned()))?;
    assert_eq!(MAX_REDACTION_SCAN_BYTES, 4096);

    // Multi-byte UTF-8 char straddling byte 4096 is cleanly backed up to a char boundary.
    let multibyte = format!("{}🦀tail", "x".repeat(4094));
    let capped_mb = cap_utf8_bytes(&multibyte, MAX_REDACTION_SCAN_BYTES);
    assert_eq!(capped_mb.len(), 4094);

    // 1. Key placed just inside the 4 KiB cut (ending right at byte 4096) -> scrubbed.
    let inside_prefix_len = MAX_REDACTION_SCAN_BYTES - BOUNDARY_KEY.len();
    let body_just_inside = format!(
        "{}{BOUNDARY_KEY}{}",
        "a".repeat(inside_prefix_len),
        "z".repeat(4096)
    );
    let scrubbed_inside = redact_secrets(&body_just_inside, Some(&key));
    assert!(scrubbed_inside.ends_with("<REDACTED_API_KEY>"));
    assert_no_8char_key_piece(&scrubbed_inside, BOUNDARY_KEY, "just-inside 4KiB redact");

    // 2. Key straddling the 4 KiB cut (12 chars inside 4096, rest outside) -> 12-char prefix (>=8) scrubbed, tail dropped before scan.
    let straddle_4k_prefix_len = MAX_REDACTION_SCAN_BYTES - 12;
    let body_straddle_4k = format!(
        "{}{BOUNDARY_KEY}{}",
        "b".repeat(straddle_4k_prefix_len),
        "z".repeat(4096)
    );
    let scrubbed_straddle_4k = redact_secrets(&body_straddle_4k, Some(&key));
    assert!(scrubbed_straddle_4k.ends_with("<REDACTED_API_KEY>"));
    assert_no_8char_key_piece(
        &scrubbed_straddle_4k,
        BOUNDARY_KEY,
        "straddling 4KiB redact",
    );

    // 3. Key placed just outside the 4 KiB cut (starting at byte 4096, and also with 4 chars inside < 8) -> dropped by 4 KiB cap, zero 8+ char piece present.
    let body_just_outside = format!(
        "{}{BOUNDARY_KEY}{}",
        "c".repeat(MAX_REDACTION_SCAN_BYTES),
        "z".repeat(8192)
    );
    let scrubbed_outside = redact_secrets(&body_just_outside, Some(&key));
    assert_eq!(scrubbed_outside.len(), MAX_REDACTION_SCAN_BYTES);
    assert_no_8char_key_piece(&scrubbed_outside, BOUNDARY_KEY, "just-outside 4KiB redact");
    let shortened_outside = redact_and_truncate(&body_just_outside, Some(&key), 200);
    assert_eq!(shortened_outside.chars().count(), 200);
    assert_no_8char_key_piece(&shortened_outside, BOUNDARY_KEY, "just-outside truncate");

    let body_4chars_inside = format!(
        "{}{BOUNDARY_KEY}{}",
        "d".repeat(MAX_REDACTION_SCAN_BYTES - 4),
        "z".repeat(4096)
    );
    let scrubbed_4chars = redact_secrets(&body_4chars_inside, Some(&key));
    assert_no_8char_key_piece(&scrubbed_4chars, BOUNDARY_KEY, "4-chars-inside 4KiB redact");

    // 4. End-to-end HTTP error turn with large (>12 KiB) bodies placing key just inside (Attempt 1) and just outside (Attempt 2) 4 KiB.
    let tmp = TempDir::new()?;
    let game_dir = tmp.path().join("c4_game");
    let wrapper = tmp.path().join("wrapper.md");
    fs::write(&wrapper, "WRAPPER")?;
    write_to_seat(&game_dir, "01", "talk", "Saltworks", "C4_PROMPT")?;
    let seat = single_openrouter_seats("Saltworks", 3.0)?;
    let tx = ScriptedTransport::new(vec![
        ScriptStep::Respond(RawHttpResponse {
            status: 500,
            body: body_just_inside,
        }),
        ScriptStep::Respond(RawHttpResponse {
            status: 502,
            body: body_just_outside,
        }),
    ]);
    let summary = run_turn(
        &TurnConfig::new(
            game_dir.clone(),
            "01".to_owned(),
            Round::Talk,
            seat,
            Some(wrapper),
            false,
            Some(1.0),
        ),
        &tx,
        Some(&key),
    )?;
    let calls_raw = fs::read_to_string(game_dir.join("relay").join("calls.jsonl"))?;
    let summary_text = format!("{}\n{summary:?}", summary.format_paths_and_hashes());
    assert_no_8char_key_piece(&calls_raw, BOUNDARY_KEY, "C4 calls.jsonl");
    assert_no_8char_key_piece(&summary_text, BOUNDARY_KEY, "C4 TurnSummary");

    Ok(())
}

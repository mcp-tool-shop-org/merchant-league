//! Subprocess CLI verification that `league-relay` Play mode refuses `--communication silent`.

use std::process::Command;

#[test]
fn league_relay_binary_refuses_communication_silent_in_play_mode() {
    let bin = env!("CARGO_BIN_EXE_league-relay");
    for sub in ["turn", "prebook-check"] {
        let output = match Command::new(bin)
            .args(["--error-format", "json", sub, "--communication", "silent"])
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .output()
        {
            Ok(out) => out,
            Err(e) => panic!("spawn league-relay {sub}: {e}"),
        };
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("SILENT_MARKET_EVAL_ONLY"),
            "expected SILENT_MARKET_EVAL_ONLY on {sub} stderr: {stderr}"
        );
    }
}

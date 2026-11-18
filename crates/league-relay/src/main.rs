//! Binary entrypoint for `league-relay`.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut keys = league_relay::collect_env_redaction_keys(|name| std::env::var(name).ok());
    if let Ok(primary) = league_relay::ApiKey::from_env() {
        keys.insert(0, primary);
    }
    let log_level =
        league_relay::cli::infer_log_level_best_effort(&args, |name| std::env::var(name).ok());
    league_relay::install_redacting_panic_hook_with_level(keys, log_level);
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let code = league_relay::cli::execute_cli(
        &args,
        |name| std::env::var(name).ok(),
        &mut stdout,
        &mut stderr,
    );
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

//! Binary entrypoint for `league-relay`.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    league_relay::install_redacting_panic_hook(league_relay::ApiKey::from_env().ok());
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let code = league_relay::cli::execute_cli(
        std::env::args().skip(1),
        |name| std::env::var(name).ok(),
        &mut stdout,
        &mut stderr,
    );
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

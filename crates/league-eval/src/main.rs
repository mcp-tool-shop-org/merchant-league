//! Binary entrypoint for `league-eval`.

#![forbid(unsafe_code)]

use league_relay::{ApiKey, install_redacting_panic_hook};

fn main() {
    let key = ApiKey::from_env().ok();
    install_redacting_panic_hook(key);
    let code = league_eval::cli::main_entry();
    std::process::exit(code);
}

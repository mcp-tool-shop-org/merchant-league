//! Binary entrypoint for the arm-blind decision scorer (`league-scorer`).

#![forbid(unsafe_code)]

fn main() {
    let code = league_scorer::cli::main_entry();
    std::process::exit(code);
}

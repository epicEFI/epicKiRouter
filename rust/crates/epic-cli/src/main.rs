//! `epic-cli` — the EpicRouter headless command-line binary (M3-T13).
//!
//! One subcommand in this milestone: `route` — the narrowed headless
//! DSN -> route -> SES + manifest pipeline (the `java -jar ... -de
//! board.dsn -do out.ses` face of `GlobalSettings.java:608-665`).
//!
//! Exit codes: 0 = routed and the session file has content; 1 = the
//! routing failed or the output is missing/empty; 2 = a usage/argument
//! error (nothing was routed).

use epic_cli::route::run_route;
use epic_engine::settings::parse_route_args;

const USAGE: &str = "usage: epic-cli route -de <board.dsn> -do <out.ses> \
                     [--result-json <manifest.json>] [--deterministic-budgets=on|off] \
                     [--router.<setting>=<value> ...] [-mp <passes>] [-mt <threads>] \
                     [-scoring-version <v1|v2|legacy|continuous|lower_bound>] \
                     [-oit <threshold>]";

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let exit = match argv.split_first() {
        Some((flag, _)) if flag == "--version" => {
            // The version face (M10-T5): single-sourced from the
            // workspace manifest via CARGO_PKG_VERSION — the same
            // source the result manifest's app_version reads.
            println!("epicrouter {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some((command, rest)) if command == "route" => match parse_route_args(rest) {
            Ok(args) if args.batch_mode() => match run_route(&args) {
                Ok(code) => code,
                Err(message) => {
                    eprintln!("Error: {message}");
                    1
                }
            },
            Ok(_) => {
                eprintln!("Error: route requires -de <board.dsn> and -do <out.ses>");
                eprintln!("{USAGE}");
                2
            }
            Err(message) => {
                eprintln!("Error: {message}");
                2
            }
        },
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(exit);
}

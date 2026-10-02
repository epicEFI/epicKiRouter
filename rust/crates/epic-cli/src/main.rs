//! `epic-cli` — the EpicRouter headless command-line binary (M3-T13).
//!
//! One subcommand in this milestone: `route` — the narrowed headless
//! DSN -> route -> SES + manifest pipeline (the `java -jar ... -de
//! board.dsn -do out.ses` face of `GlobalSettings.java:608-665`).
//!
//! Exit codes: 0 = routed and the session file has content; 1 = the
//! routing failed or the output is missing/empty; 2 = a usage/argument
//! error (nothing was routed).

use epic_cli::route::preflight_output_path;
use epic_cli::route::run_route;
use epic_engine::settings::parse_route_args;

const USAGE: &str = "usage: epic-cli route -de <board.dsn> -do <out.ses> \
                     [--result-json <manifest.json>] [--deterministic-budgets=on|off] \
                     [--router.<setting>=<value> ...] [-mp <passes>] [-mt <threads>] \
                     [-scoring-version <v1|v2|legacy|continuous|lower_bound>] \
                     [-oit <threshold>]";

// ---------------------------------------------------------------------------
// the help surface (readiness-fix M1)
// ---------------------------------------------------------------------------

/// The settings reference: the property table the parseable surface is
/// compiled from. The parse itself lives in
/// `epic-engine/src/settings.rs` (`apply_router_setting`, one match —
/// NOT mechanically enumerable as a const, so this static table is the
/// help face's single source, pinned by the `help_surface` integration
/// test's representative-set assert). Value kinds: `on|off` booleans,
/// integers, floats, strings, and the `tuning.pairs` net-pair grammar.
const SETTINGS_REFERENCE: &str = "\
accepted settings (`--router.<property>=<value>` / `--optimizer.<property>=<value>`; \
the same properties layer defaults -> DSN -> CLI):

  router.enabled=<on|off>                     (deprecated flat spelling; \
warns; maps to autorouter.enabled)
  router.autorouter.enabled=<on|off>
  router.autorouter.max_passes=<integer>
  router.autorouter.max_items=<integer>
  router.max_threads=<integer>                (short flag: -mt)
  router.vias_allowed=<on|off>
  router.via_costs=<integer>
  router.plane_via_costs=<integer>
  router.plane_island_clamp=<on|off>
  router.congestion_global=<on|off>
  router.congestion_global.pattern=<on|off>
  router.congestion_global.pathfinder=<on|off>
  router.push_shove=<on|off>
  router.tuning=<on|off>
  router.tuning.meander=<on|off>
  router.tuning.pairs=<NET_A:NET_B[,NET_C:NET_D...]>
  router.assign.pins=<REF[,REF...]>
  router.current.nets=<NET:AMPS[,NET:AMPS...]>
  router.current.copper_oz=<float>
  router.current.temp_rise_c=<float>
  router.gloss.bus=<on|off>
  router.gloss.flow=<on|off>
  router.gloss.via_place=<on|off>
  router.gloss.teardrops=<on|off>
  router.start_ripup_costs=<integer>
  router.automatic_neckdown=<on|off>
  router.trace_pull_tight_accuracy=<integer>
  router.strict_drc=<on|off>
  router.fanout.enabled=<on|off>
  router.fanout.max_passes=<integer>
  router.fanout.max_items=<integer>
  router.fanout.max_milliseconds_per_pin=<integer>
  router.fanout.ripup_allowed=<on|off>
  router.fanout.min_escape_length_mm=<float>
  router.fanout.max_escape_length_mm=<float>
  router.fanout.start_via_diameter_mm=<float>
  router.fanout.end_via_diameter_mm=<float>
  router.fanout.pin_sorting_order=<string>
  router.fanout.fallback_to_board_vias=<on|off>
  router.fanout.timeout=<string>
  router.optimizer.enabled=<on|off>
  router.optimizer.algorithm=<string>
  router.optimizer.max_passes=<integer>
  router.optimizer.max_items=<integer>
  router.optimizer.max_threads=<integer>
  router.optimizer.threads=<integer>
  router.optimizer.enable_preflight_guards=<on|off>
  router.optimizer.max_consecutive_failures=<integer>
  router.optimizer.max_consecutive_failures_pass1=<integer>
  router.optimizer.additional_ripup_cost_factor_at_start=<integer>
  router.optimizer.trace_ripup_cost_factor=<float>
  router.optimizer.max_autoroute_passes=<integer>
  router.optimizer.timeout=<string>
  router.optimizer.improvement_threshold=<float>   (short flag: -oit)
  router.scoring.version=<v1|v2|legacy|continuous|lower_bound>  (short flag: -scoring-version)
  optimizer.* mirrors of every property above (same value kinds; the
    optimizer-box scoring versions are v1|legacy|lower_bound|continuous)

  camelCase aliases of the optimizer.* names (maxPasses, maxItems,
  maxThreads, enablePreflightGuards, maxConsecutiveFailures[Pass1],
  additionalRipupCostFactorAtStart, traceRipupCostFactor,
  maxAutoroutePasses, timeoutString) parse under the bare
  `--optimizer.` prefix ONLY — the `--router.` prefix lowercases the
  path first (`canonical_cli_path`), so `--router.optimizer.maxPasses`
  would silently match nothing.
  Other deprecated flat spellings that still parse (each warns and
  maps to its autorouter.* home): router.max_passes, router.algorithm,
  router.max_items, router.save_intermediate_stages,
  router.ignore_net_classes.

other flags:
  --result-json <manifest.json>   write the run manifest (JSON) to the path
  --dump-aesthetics <file.json>   write the aesthetics sidecar (never the manifest)
  --deterministic-budgets=on|off  engine budget faces (deterministic; no wall clock)
  -mp <passes>                    router.autorouter.max_passes
  -mt <threads>                   router.max_threads
  -oit <threshold>                router.optimizer.improvement_threshold
  -scoring-version <v>            both scoring boxes
  -router-scoring-version <v>     the router scoring box only
  -optimizer-scoring-version <v>  the optimizer scoring box only
  --version                       print the version and exit 0
  --help, help                    this text on stdout, exit 0";

/// The full help text: the usage line + the settings reference.
#[must_use]
pub fn help_text() -> String {
    format!("{USAGE}\n\n{SETTINGS_REFERENCE}\n")
}

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
        // The readiness-fix M1 face: `--help`/`help` on stdout, exit 0
        // (a discovery surface, not an error).
        Some((flag, _)) if flag == "--help" || flag == "help" => {
            print!("{}", help_text());
            0
        }
        Some((command, rest)) if command == "route" => match parse_route_args(rest) {
            Ok(args) if args.batch_mode() => match preflight_output_path(&args) {
                // The readiness-fix M2 face: the output-path pre-flight
                // is the ARG-validation stage — a failure is the usage
                // class (exit 2), before any design bytes are read.
                Err(message) => {
                    eprintln!("Error: {message}");
                    2
                }
                Ok(()) => match run_route(&args) {
                    Ok(code) => code,
                    Err(message) => {
                        eprintln!("Error: {message}");
                        1
                    }
                },
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
        // No arguments at all: the same help text on stderr, exit 2 —
        // the usage class is UNCHANGED (the charter's face: a bare
        // invocation is still an error, just an informed one).
        None => {
            eprintln!("{}", help_text());
            2
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(exit);
}

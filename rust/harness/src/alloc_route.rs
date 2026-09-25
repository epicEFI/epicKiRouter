//! The `alloc-route` harness command (feature `alloc-profile` only) — the
//! M5-T1 allocation-cost runner.
//!
//! Routes ONE fixture IN-PROCESS through the SAME construction the
//! battery's `epic-cli route` subprocess runs (the documented
//! detail-pass construction: parse → board → search tree → normalize →
//! DRC seed → the settings resolution chain → `BatchDriver`), under
//! the counting allocator armed by [`alloc_profile::install`]. The
//! attribution window is **driver construction + run + drop** (the
//! snapshot pair brackets `CliDriverSink::default()` + `BatchDriver::new`,
//! the `BatchDriver::run` call, and the driver drop): parse/normalize/
//! settings allocations fall BEFORE the window and the post-route face
//! walks fall after it.
//!
//! NO session file is written (the scratch `-do` path exists only so
//! `parse_route_args` sees a complete argv face, exactly like the detail
//! pass). The route's completion face (incomplete/violation totals) is
//! computed AFTER [`alloc_profile::freeze`] — its allocations are outside
//! the window, so the stats file's totals are the freeze-time face
//! (setup + window; the post-route walks are excluded). The RESULT
//! stderr line is the WINDOW-ONLY face (the deltas between the two
//! snapshots).
//!
//! External timeout discipline (the standing rule): the CALLER bounds the
//! run (`timeout <tiers timeout>`) — the instrument adds no deadline of
//! its own, mirroring the battery's harness-side timeout face.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use crate::alloc_profile;

/// Routes `dsn` in-process with the counting allocator armed.
///
/// * `stats` — the allocator stats file path (1 s watcher + final flush).
/// * `sample` — capture a backtrace sample every Nth allocation (0 = off).
pub fn run(dsn: &Path, stats: &Path, sample: u64) -> Result<()> {
    alloc_profile::install(stats, sample);
    eprintln!(
        "alloc-route: instrument armed (stats -> {}, sample every {} alloc(s))",
        stats.display(),
        sample
    );

    use epic_board::board::Board;
    use epic_board::tree_manager::SearchTreeManager;
    use epic_cli::route::{CliDriverSink, build_batch_settings};
    use epic_cli::settings::{
        DsnLayer, MergedSettings, ResolvedRouteSettings, apply_board_specific_optimizations, merge,
        parse_route_args, validate,
    };
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;
    use epic_router::pipeline::batch::{BatchDriver, StopFace};
    use epic_router::pipeline::board_statistics::BoardStatistics;

    // The full-profile argv face: NO comparability flags (the assembled
    // fanout → router → optimizer pipeline, the battery face).
    let scratch_ses = std::env::temp_dir().join(format!(
        "epic-m5t1-alloc-{}-{}.ses",
        std::process::id(),
        dsn.file_name().and_then(|n| n.to_str()).unwrap_or("board")
    ));
    let argv = vec![
        "-de".to_string(),
        dsn.to_string_lossy().into_owned(),
        "-do".to_string(),
        scratch_ses.to_string_lossy().into_owned(),
    ];
    let args = parse_route_args(&argv).map_err(anyhow::Error::msg)?;

    let bytes = std::fs::read(dsn).with_context(|| format!("reading {}", dsn.display()))?;
    let mut ses = SesBoard::new();
    match read_board(&bytes, &mut ses) {
        DsnReadResult::Success { .. } | DsnReadResult::OutlineMissing { .. } => {}
        DsnReadResult::ParseError { location, detail } => {
            bail!("parse error at {location}: {detail}");
        }
        DsnReadResult::IoError => bail!("I/O error reading {}", dsn.display()),
    }

    // The settings resolution chain (run_route step 1b — hoisted above
    // the board build so the override consumes the merged settings at
    // its parse-time call point).
    let dsn_layer = DsnLayer::from_metadata(
        ses.metadata.autoroute_settings.as_ref(),
        usize::try_from(ses.metadata.layer_count).unwrap_or(0),
    );
    let mut merged = merge(&MergedSettings::default(), &dsn_layer, &args.layer);
    for warning in validate(&mut merged) {
        eprintln!("Warning: {warning}");
    }

    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    // The manager's parse-time copper-to-edge override (Java
    // `createBoard` :346, BEFORE `Wiring.java:347` normalizeAllTraces)
    // — the profile must measure the production load face (the
    // buglog-189 fix).
    epic_cli::route::apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);

    // The load-time violation seed (the flow's step 2b — POST-override:
    // Java's deferred DRC reads the promoted board).
    let (pre_total, _) =
        epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
    board.pre_existing_clearance_violations_count = i32::try_from(pre_total).unwrap_or(i32::MAX);

    apply_board_specific_optimizations(&mut merged, &board);
    let resolved = ResolvedRouteSettings::resolve(&merged, args.deterministic_budgets);
    let batch = build_batch_settings(&resolved);

    // THE ATTRIBUTION WINDOW: driver construction + run + drop (the
    // snapshot pair brackets CliDriverSink + BatchDriver::new, the
    // BatchDriver::run call, and the driver drop).
    let before = alloc_profile::snapshot();
    let route_started = Instant::now();
    let mut sink = CliDriverSink::default();
    let mut driver = BatchDriver::new(&mut manager, &mut board, batch, StopFace::default());
    let run_result = driver.run(&mut sink);
    drop(driver);
    let route_wall = route_started.elapsed();
    let after = alloc_profile::snapshot();
    // Freeze BEFORE the face walks: the stats file's totals are the
    // freeze-time face (setup + window; the post-route walks excluded).
    alloc_profile::freeze();

    eprintln!(
        "alloc-route RESULT window=driver construction + run + drop wall={:.3}s allocs={} alloc_bytes={} live_at_exit={} peak_live={}",
        route_wall.as_secs_f64(),
        after.allocs - before.allocs,
        after.alloc_bytes - before.alloc_bytes,
        after.live,
        after.peak,
    );

    run_result.map_err(|e| anyhow::anyhow!("batch driver error: {e:?}"))?;

    // The completion face (post-freeze: uncounted, battery-comparable
    // totals + violation counts — the same walks the detail pass runs;
    // the incomplete total is the SUM over the per-net rows, the detail
    // pass's formula — all_incompletes' first tuple member is the
    // max-connections endpoint sum, a different quantity).
    let statistics = BoardStatistics::new(&mut manager, &mut board);
    let (violation_total, _) =
        epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
    let (_max_connections, net_rows) = epic_drc::incompletes::all_incompletes(&manager, &mut board);
    let incomplete_total: i64 = net_rows.iter().map(|row| row.incomplete_count as i64).sum();
    eprintln!(
        "alloc-route FACE: incomplete {incomplete_total} violations {violation_total} \
         traces {} vias {}",
        statistics.traces.total_count, statistics.vias.total_count,
    );
    Ok(())
}

//! The M6-T6 pour-island detector face (advisory, java-free): parse the
//! DSN, run `epic_board::islands::detect_pour_islands`, and print the
//! per-pour rows. The fixture-evidence instrument for the detector
//! population — never a gate; the advisory face changes no route
//! decision and no SES byte.

use anyhow::Result;
use std::path::Path;

use anyhow::bail;
use epic_board::board::Board;
use epic_dsn::reader::{DsnReadResult, read_board};

/// Invocation: `epic-harness islands --dsn <path>`. Prints one
/// `POUR ...` row per filled pour and a trailing `exit=0` line (the
/// evidence convention); exits nonzero on a parse failure.
pub fn run(dsn: &Path) -> Result<()> {
    let bytes = std::fs::read(dsn)
        .map_err(|error| anyhow::anyhow!("cannot read design {}: {error}", dsn.display()))?;
    let mut ses = epic_dsn::ses_board::SesBoard::new();
    match read_board(&bytes, &mut ses) {
        DsnReadResult::Success { warnings } | DsnReadResult::OutlineMissing { warnings } => {
            for warning in warnings {
                eprintln!("Warning: {warning}");
            }
        }
        DsnReadResult::ParseError { location, detail } => {
            bail!("parse error at {location}: {detail}");
        }
        DsnReadResult::IoError => {
            bail!("I/O error reading {}", dsn.display());
        }
    }
    let board = Board::from_ses_board(&ses);
    let faces = epic_board::islands::detect_pour_islands(&board);
    if faces.is_empty() {
        println!("NO_POURS");
    }
    for face in &faces {
        println!(
            "POUR net={} layer={} item={} regions={} islands={} digest={}",
            face.net,
            face.layer,
            face.pour_item_id,
            face.region_count,
            face.island_count,
            face.digest
        );
        for island in &face.islands {
            println!(
                "  ISLAND bbox=({},{})-({},{}) cells={}",
                island.x0, island.y0, island.x1, island.y1, island.cells
            );
        }
    }
    println!(
        "pours={} islands={}",
        faces.len(),
        faces.iter().map(|face| face.island_count).sum::<usize>()
    );
    Ok(())
}

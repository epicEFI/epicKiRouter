//! The board -> SES projection + the SES design-name face, relocated
//! from `epic-cli/src/route.rs` at M9-T2 so the SESSION path and the
//! CLI path call the SAME code (the M9 law: the session path IS the
//! CLI path — two implementations of the projection would let the
//! parity pin drift). Pure relocation: every body and doc comment is
//! byte-verbatim from the route.rs state at `4a0d7b3f5`; epic-cli
//! re-imports [`project_routed_items`] and [`filename_without_extension`].

use epic_board::board::Board;
use epic_board::items::{FixedState, ItemData};
use epic_dsn::ses_board::{ItemIr, SesBoard};
use epic_dsn::sink::{FixedStateIr, TraceIr, ViaIr};
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::rounding::java_round;

/// The rounded corner view of a board polyline corner (`corner_to_int`,
/// `io.specctra.scope.Wiring`): `Point::Int` passes through; a rational
/// corner rounds each axis of its float approximation with `Math.round`
/// semantics.
fn corner_to_int(point: Point) -> IntPoint {
    match point {
        Point::Int(p) => p,
        Point::Rational(r) => {
            let float = r.to_float();
            IntPoint::new(java_round(float.x) as i32, java_round(float.y) as i32)
        }
    }
}

/// The fixed-state transfer (`Item.fixedState` — the ladders are
/// 1:1 across the crates).
fn fixed_to_ir(fixed: FixedState) -> FixedStateIr {
    match fixed {
        FixedState::Unfixed => FixedStateIr::Unfixed,
        FixedState::ShoveFixed => FixedStateIr::ShoveFixed,
        FixedState::UserFixed => FixedStateIr::UserFixed,
        FixedState::SystemFixed => FixedStateIr::SystemFixed,
    }
}

/// Projects the LIVE board's routed items (traces + vias, ascending id)
/// into the parse SES board. Deliberately NOT the
/// `BoardSink::insert_trace` face: those apply parse drop guards and
/// fresh ids; the projection preserves the LIVE item's id verbatim
/// (`SesBoard::push_routed_item`).
///
/// Buglog 210 (M7-T5, Java-exactness): the emitted session's wiring is
/// the LIVE board's wiring — Java's `SesWriter.writeNet` walks
/// `board.getConnectableItems(netNumber)` and emits ONE `(wire` per
/// on-board `PolylineTrace` (instrumented: the jar's session on a
/// pre-routed input carries one wire per net). The parse board retains
/// the input `(wiring` items alongside the projection, so a pre-routed
/// input would emit the pre-route copy NEXT TO the live copy (the
/// duplication the T4 capture showed). The live board's trace/via
/// population is the complete truth — survivors keep their parse ids,
/// ripped-up items are gone — so the parse routed items are drained
/// before the projection. Pins/keepouts/conduction areas are parse
/// items the projection never supplies and stay.
pub fn project_routed_items(board: &Board, ses: &mut SesBoard) {
    ses.items
        .retain(|item| !matches!(item, ItemIr::Trace { .. } | ItemIr::Via { .. }));
    for entry in board.iter_ascending() {
        if !entry.on_the_board {
            // Java's item walks see live items only.
            continue;
        }
        let id = i32::try_from(entry.id.get()).unwrap_or(0);
        match &entry.data {
            ItemData::Trace {
                layer,
                half_width,
                lines,
            } => {
                let corners: Vec<IntPoint> = (0..i32::try_from(lines.corner_count()).unwrap_or(0))
                    .filter_map(|index| lines.corner(index))
                    .map(corner_to_int)
                    .collect();
                ses.push_routed_item(ItemIr::Trace {
                    id,
                    trace: TraceIr {
                        layer_no: *layer,
                        half_width: *half_width,
                        corners,
                        polyline: lines.clone(),
                        nets: entry.nets.clone(),
                        clearance_class: entry.clearance_class,
                        fixed: fixed_to_ir(entry.fixed),
                    },
                });
            }
            ItemData::Via {
                center,
                padstack_no,
                attach_smd_allowed,
            } => {
                ses.push_routed_item(ItemIr::Via {
                    id,
                    via: ViaIr {
                        padstack_no: *padstack_no,
                        location: *center,
                        nets: entry.nets.clone(),
                        clearance_class: entry.clearance_class,
                        fixed: fixed_to_ir(entry.fixed),
                        attach_smd_allowed: *attach_smd_allowed,
                    },
                });
            }
            // Pins, keepouts, conduction areas and outlines are parse
            // items — already in `ses.items` from the read.
            _ => {}
        }
    }
}

/// Java `BoardFileDetails.getFilenameWithoutExtension`
/// (`BoardFileDetails.java:205-210`): the name up to the LAST dot when
/// it contains one, else unchanged. This — not the raw file name — is
/// the job name Java hands `SesWriter.write` (`RoutingJob.java:519`), so
/// the session scope's design face carries NO `.dsn` (the writer's
/// `.replace(".dsn", ".ses")` is a no-op on it).
pub fn filename_without_extension(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(dot) => &filename[..dot],
        None => filename,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The three pins RELOCATED with their functions from epic-cli
    // route.rs's test module (M9-T2; census-neutral: they left that
    // crate's count when they landed here).

    /// The rounded corner view: Int passes through; a rational corner
    /// rounds with Java Math.round per axis — the 1.5 / -2.5 pair
    /// discriminates floor(x+0.5) from truncation and from
    /// half-away-from-zero (-2.5 -> -2, not -3).
    #[test]
    fn corner_to_int_java_round_view() {
        use num_bigint::BigInt;
        assert_eq!(
            corner_to_int(Point::Int(IntPoint::new(7, -9))),
            IntPoint::new(7, -9)
        );
        assert_eq!(
            corner_to_int(Point::get_instance_big(
                BigInt::from(3),
                BigInt::from(-5),
                BigInt::from(2)
            )),
            IntPoint::new(2, -2)
        );
        // The negated-denominator normalization (z < 0 flips) lands on
        // the same rational point.
        assert_eq!(
            corner_to_int(Point::get_instance_big(
                BigInt::from(-3),
                BigInt::from(5),
                BigInt::from(-2)
            )),
            IntPoint::new(2, -2)
        );
    }

    /// The fixed-state ladder is permutation-sensitive — each
    /// `FixedState` maps to the SAME-named IR variant (the
    /// UserFixed/SystemFixed arm swap passed the whole suite and was
    /// invisible to every compare before this pin).
    #[test]
    fn fixed_to_ir_ladder_is_permutation_sensitive() {
        assert!(matches!(
            fixed_to_ir(FixedState::Unfixed),
            FixedStateIr::Unfixed
        ));
        assert!(matches!(
            fixed_to_ir(FixedState::ShoveFixed),
            FixedStateIr::ShoveFixed
        ));
        assert!(matches!(
            fixed_to_ir(FixedState::UserFixed),
            FixedStateIr::UserFixed
        ));
        assert!(matches!(
            fixed_to_ir(FixedState::SystemFixed),
            FixedStateIr::SystemFixed
        ));
    }

    /// The LAST-dot cut (BoardFileDetails.java:205-210): a two-dot
    /// input name must keep the middle segment (the first-dot mutant
    /// emitted `(session my` for `my.bad.dsn`).
    #[test]
    fn filename_without_extension_cuts_at_last_dot() {
        assert_eq!(filename_without_extension("my.bad.dsn"), "my.bad");
        assert_eq!(filename_without_extension("plain.dsn"), "plain");
        assert_eq!(filename_without_extension("noext"), "noext");
    }
}

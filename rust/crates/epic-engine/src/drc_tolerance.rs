//! The #925a DRC clearance-tolerance apply face (upstream
//! `14b28b6ff`, the tolerance half): the resolved
//! `router.drc.clearance_tolerance_um` override written into the
//! board's rules before the pipeline's first violation read.
//!
//! Java anchor: `HeadlessBoardManager.applyClearanceToleranceOverride`
//! (settings load, `:598-623`): a non-finite or negative value is
//! WARNED + IGNORED — the board keeps its default (1.0, seeded in
//! `BoardRules` at both construction faces) — and the run continues;
//! the measure loop itself additionally clamps such values to 0.0
//! (defense in depth, `epic_drc::clearance` module docs item 3). The
//! settings faces are otherwise identical in shape to F1/F2/F3: the
//! flag rides the 6-site plumbing, the write lands at the SAME route
//! head placement in both `Session::route` and the CLI's `run_route`,
//! and a bad value never fails the run.

use epic_board::board::Board;

/// Write the clearance-tolerance override into
/// [`BoardRules::clearance_tolerance_um`](epic_board::rules_surf::BoardRules::clearance_tolerance_um).
/// `Ok(())` when written; `Err(value)` when the value is non-finite
/// or negative — the caller warns and keeps the board default (Java
/// `applyClearanceToleranceOverride` parity).
pub fn apply_clearance_tolerance(board: &mut Board, value: f64) -> Result<(), f64> {
    if value.is_finite() && value >= 0.0 {
        board.rules_mut().clearance_tolerance_um = value;
        Ok(())
    } else {
        Err(value)
    }
}

#[cfg(test)]
mod tests {
    use super::apply_clearance_tolerance;
    use epic_board::board::Board;

    /// The apply face: a valid value (0.0 AND a positive real — both
    /// extremes are legal; 0.0 reproduces the pre-#925a behavior
    /// exactly) writes through; a non-finite or negative value is
    /// REJECTED with the value itself (the caller's warn carries it)
    /// and the board's seeded default (1.0) survives untouched.
    #[test]
    fn apply_writes_valid_and_rejects_invalid_values() {
        // The empty board's rules carry the Default seed (1.0) — the
        // same face a parsed board gets through from_ir.
        let mut board = Board::new();
        assert_eq!(board.rules().clearance_tolerance_um, 1.0, "the seed");

        assert!(apply_clearance_tolerance(&mut board, 0.0).is_ok());
        assert_eq!(board.rules().clearance_tolerance_um, 0.0, "0.0 is legal");

        assert!(apply_clearance_tolerance(&mut board, 2.5).is_ok());
        assert_eq!(board.rules().clearance_tolerance_um, 2.5);

        for bad in [-0.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let Err(rejected) = apply_clearance_tolerance(&mut board, bad) else {
                panic!("{bad} must be rejected");
            };
            // Bitwise compare: `assert_eq!` is NaN-blind (NaN != NaN).
            assert_eq!(
                rejected.to_bits(),
                bad.to_bits(),
                "the caller's warn carries the value"
            );
            assert_eq!(
                board.rules().clearance_tolerance_um,
                2.5,
                "a rejected write leaves the board untouched"
            );
        }
    }
}

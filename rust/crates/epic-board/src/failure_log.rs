//! Java `autoroute/RoutingFailureLog.java` — the per-item routing
//! failure logbook, the `RoutingBoard.failureLog` field
//! (`RoutingBoard.java:64`, constructed at `:91`).
//!
//! The log lives ON the board (a non-transient `Serializable` field in
//! Java), so a board snapshot carries it and a restore ROLLS IT BACK
//! to the snapshot state; mirroring that here is why the log is a
//! plain [`Board`] field ([`crate::board::Board::failure_log`]) rather
//! than driver-side state.
//!
//! The Java `ItemFailureInfo` holds the `Item` OBJECT; the map is
//! keyed by the item id here (the id is all the consumer reads —
//! `getFailureCount`). The state/reason faces are stored as strings:
//! Java serializes the enum by name, and `epic-board` cannot depend on
//! the router crate's `AutorouteAttemptState`.

use std::collections::BTreeMap;

/// Java `RoutingFailureLog.FAILURE_THRESHOLD` — give up after this
/// many failures for the same item.
pub const FAILURE_THRESHOLD: i64 = 50;

/// Java `RoutingFailureLog.ItemFailureInfo` minus the held `Item`
/// reference (the id is the map key).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemFailureInfo {
    /// Java `netNumber` — the item's FIRST net captured at
    /// info-creation time (`item.netCount() > 0 ?
    /// item.getNetNumber(0) : -1`).
    pub net_number: i32,
    /// Java `failureCount`.
    pub failure_count: i64,
    /// Java `lastFailureState` — the enum NAME (serialization form).
    pub last_failure_state: Option<String>,
    /// Java `lastFailureReason` (null-normalized to empty).
    pub last_failure_reason: String,
    /// Java `lastAttemptPass`.
    pub last_attempt_pass: i64,
}

impl ItemFailureInfo {
    /// Java `shouldGiveUp` (`RoutingFailureLog.java`, the inner-class
    /// tail): `failureCount >= FAILURE_THRESHOLD`.
    #[must_use]
    pub fn should_give_up(&self) -> bool {
        self.failure_count >= FAILURE_THRESHOLD
    }
}

/// Java `RoutingFailureLog` — the serializable per-item failure map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoutingFailureLog {
    failures: BTreeMap<u64, ItemFailureInfo>,
}

impl RoutingFailureLog {
    /// Java `recordFailure(item, passNo, state, reason)`
    /// (`:32-53`): create-or-update the id's info. `first_net_number`
    /// is the caller-side evaluation of Java's
    /// `item.netCount() > 0 ? item.getNetNumber(0) : -1` — read ONLY
    /// when the record is created (Java's `compute` creates the info
    /// once); `state` is the enum name (Java serializes it by name);
    /// a null `reason` is stored as the empty string.
    pub fn record_failure(
        &mut self,
        item_id: u64,
        first_net_number: i32,
        pass_no: i64,
        state: &str,
        reason: Option<&str>,
    ) {
        let info = self.failures.entry(item_id).or_default();
        if info.failure_count == 0 && info.last_failure_state.is_none() {
            // Java's `new ItemFailureInfo(item)` runs only on first
            // creation; its only lasting effect is the net capture.
            info.net_number = first_net_number;
        }
        info.failure_count += 1;
        info.last_attempt_pass = pass_no;
        info.last_failure_state = Some(state.to_string());
        info.last_failure_reason = reason.unwrap_or("").to_string();
    }

    /// Java `getFailureCount(item)` (`:89-99`): the item's failure
    /// count, 0 when none recorded.
    #[must_use]
    pub fn failure_count(&self, item_id: u64) -> i64 {
        self.failures
            .get(&item_id)
            .map_or(0, |info| info.failure_count)
    }

    /// Java `shouldSkip(item)`/`ItemFailureInfo.shouldGiveUp` — the
    /// threshold face (unused by the single-thread pass runner, whose
    /// gate is the `>= 3` DEBUG-log one).
    #[must_use]
    pub fn should_give_up(&self, item_id: u64) -> bool {
        self.failures
            .get(&item_id)
            .is_some_and(ItemFailureInfo::should_give_up)
    }

    /// Java `clear()` — drop every record.
    pub fn clear(&mut self) {
        self.failures.clear();
    }

    /// The recorded ids (ascending) — the unroutable-report face of
    /// Java's `getUnroutableItems` keyed view.
    #[must_use]
    pub fn recorded_ids(&self) -> Vec<u64> {
        self.failures
            .iter()
            .filter(|(_, info)| info.should_give_up())
            .map(|(&id, _)| id)
            .collect()
    }
}

#[cfg(test)]
mod pins {
    use super::{FAILURE_THRESHOLD, RoutingFailureLog};

    /// The record/count faces: first-record captures the net, later
    /// records keep it; the count accumulates; the threshold flips
    /// exactly at FAILURE_THRESHOLD (the `>=` gate — Java's
    /// `shouldGiveUp` uses `>=`); `clear` empties.
    #[test]
    fn record_count_and_threshold_faces() {
        let mut log = RoutingFailureLog::default();
        assert_eq!(log.failure_count(7), 0, "no record yet");
        assert!(!log.should_give_up(7));
        log.record_failure(7, 94, 1, "FAILED", Some("because no connection"));
        assert_eq!(log.failure_count(7), 1);
        log.record_failure(7, 1, 2, "FAILED", None);
        assert_eq!(log.failure_count(7), 1 + 1);
        let info = &log.failures[&7];
        assert_eq!(info.net_number, 94, "the FIRST record captured the net");
        assert_eq!(info.last_attempt_pass, 2);
        assert_eq!(
            info.last_failure_state.as_deref(),
            Some("FAILED"),
            "the enum name is stored"
        );
        assert_eq!(info.last_failure_reason, "", "null reason -> empty");
        // Threshold boundary: >= , not >.
        let mut full = RoutingFailureLog::default();
        for pass in 0..FAILURE_THRESHOLD - 1 {
            full.record_failure(3, 1, pass, "FAILED", None);
        }
        assert!(!full.should_give_up(3), "below the threshold");
        full.record_failure(3, 1, 99, "FAILED", None);
        assert!(full.should_give_up(3), "exactly at the threshold");
        full.clear();
        assert_eq!(full.failure_count(3), 0);
    }
}

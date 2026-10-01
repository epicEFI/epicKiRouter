//! Item ids: the `ItemId` newtype and the id allocator.
//!
//! Java anchors: `board/actions/ItemIdGenerator.java` (the whole class —
//! the fork keeps the generator in `board.actions`, not `datastructures`)
//! and `board/model/items/Item.java:86-90` (allocation happens in the
//! `Item` CONSTRUCTOR when the passed id is <= 0 — M2 trap T61:
//! construct-then-discard burns an id, deletion never frees one).
//!
//! Wrap semantics (T61, `ItemIdGenerator.java:37-55`): ids run 1..
//! [`MAX_ID`] inclusive; when `lastGeneratedId` has reached [`MAX_ID`]
//! the next `newId` resets the counter to 0 and returns 1 (never
//! negative). Java emits an `FRLogger.warn` per wrap event (its text
//! carries the wrap counter); this port keeps the `wrap_count` mirror
//! but gates the warning on a ONE-SHOT flag per the M2 plan — the
//! observable surfaces never see it (`FRLogger.warn` is log-only; the
//! D12 parity-warnings list is fed exclusively by `Wiring.java`'s
//! `warnings.add` sites).

/// Java `ItemIdGenerator.MAX_ID = Integer.MAX_VALUE / 2` (1073741823).
pub const MAX_ID: u32 = (i32::MAX as u32) / 2;

/// Java `Item.getId()` — a positive board-unique item id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(u32);

impl ItemId {
    /// Wraps a raw id. Callers guarantee the value came from an id
    /// source (the generator or a parse IR id — always >= 1).
    #[must_use]
    pub fn new(raw: u32) -> Self {
        debug_assert!(raw >= 1, "item ids are positive (Java Item.getId())");
        Self(raw)
    }

    /// The raw id (Java `int`, always >= 1 here).
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// Java `ItemIdGenerator`: monotone counter from 1 with wrap-to-1
/// overflow protection (module docs).
#[derive(Clone, Copy, Debug)]
pub struct ItemIdGenerator {
    /// Java `lastGeneratedId`: the id the last `new_id` handed out
    /// (0 before the first call).
    last_generated_id: u32,
    /// Java `wrapAroundCount` — diagnostics only.
    wrap_count: u64,
    /// Whether the wrap warning has fired (one-shot per the M2 plan;
    /// see the module docs for the Java divergence).
    warned_on_wrap: bool,
}

impl Default for ItemIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemIdGenerator {
    /// A fresh generator; the next id is 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_generated_id: 0,
            wrap_count: 0,
            warned_on_wrap: false,
        }
    }

    /// Java `newId()` (`ItemIdGenerator.java:37-55`): wrap-to-1 at
    /// [`MAX_ID`], then increment. Ids [`MAX_ID`] itself IS handed out;
    /// the wrap fires on the call AFTER it.
    pub fn new_id(&mut self) -> ItemId {
        if self.last_generated_id >= MAX_ID {
            self.wrap_count += 1;
            // Java: FRLogger.warn("IdGenerator: ID counter reached ...").
            // Log-only on the Java side; the one-shot flag stands in for
            // the log call (module docs).
            self.warned_on_wrap = true;
            self.last_generated_id = 0;
        }
        self.last_generated_id += 1;
        ItemId(self.last_generated_id)
    }

    /// Java `maxGeneratedId()`: the last handed-out id (0 when none).
    #[must_use]
    pub fn max_generated_id(&self) -> u32 {
        self.last_generated_id
    }

    /// Java `wrapAroundCount`.
    #[must_use]
    pub fn wrap_count(&self) -> u64 {
        self.wrap_count
    }

    /// Whether the one-shot wrap warning has fired.
    #[must_use]
    pub fn warned_on_wrap(&self) -> bool {
        self.warned_on_wrap
    }

    /// Positions the generator as if `last_generated` had just been
    /// handed out: the next [`ItemIdGenerator::new_id`] returns
    /// `last_generated + 1`, unless `last_generated >= MAX_ID` in which
    /// case it wraps to 1. This is the `from_ses_board` generator
    /// restore (`SesBoard::last_assigned_item_id`) and the test seam for
    /// forcing a wrap without allocating ~2^30 ids.
    pub fn set_next(&mut self, last_generated: u32) {
        self.last_generated_id = last_generated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ItemIdGenerator.java:37-55`: a fresh generator hands out 1, 2, 3
    /// in order. An off-by-one (start at 0 or skip) fails.
    #[test]
    fn new_generator_hands_out_one_two_three() {
        let mut generator = ItemIdGenerator::new();
        assert_eq!(generator.new_id(), ItemId::new(1));
        assert_eq!(generator.new_id(), ItemId::new(2));
        assert_eq!(generator.new_id(), ItemId::new(3));
        assert_eq!(generator.max_generated_id(), 3);
        assert_eq!(generator.wrap_count(), 0);
        assert!(!generator.warned_on_wrap());
    }

    /// The boundary BELOW the wrap: `MAX_ID - 1` is still handed out
    /// normally, the next call returns `MAX_ID` itself, and neither call
    /// wraps (`ItemIdGenerator.java:38` guards `>= MAX_ID`, so the id
    /// equal to `MAX_ID` is issued). A `>` guard would wrap one call
    /// early.
    #[test]
    fn max_id_itself_is_handed_out_before_the_wrap() {
        let mut generator = ItemIdGenerator::new();
        generator.set_next(MAX_ID - 1);
        assert_eq!(generator.new_id(), ItemId::new(MAX_ID));
        assert_eq!(generator.wrap_count(), 0);
        assert!(!generator.warned_on_wrap());
    }

    /// The wrap pin (T61): `set_next(MAX_ID)` then `new_id` must return
    /// 1 — the call executes the wrap branch, not merely approaches it —
    /// and the one-shot warning flag plus the wrap counter fire.
    #[test]
    fn forced_wrap_returns_one_and_fires_the_warning_once() {
        let mut generator = ItemIdGenerator::new();
        generator.set_next(MAX_ID);
        assert_eq!(generator.new_id(), ItemId::new(1), "wrap to 1");
        assert_eq!(generator.wrap_count(), 1);
        assert!(generator.warned_on_wrap());
        // The counter continues monotonically from the wrap point.
        assert_eq!(generator.new_id(), ItemId::new(2));
        // A SECOND wrap fires the counter again but not the warning.
        generator.set_next(MAX_ID);
        assert_eq!(generator.new_id(), ItemId::new(1));
        assert_eq!(generator.wrap_count(), 2);
        assert!(generator.warned_on_wrap(), "warning stays one-shot");
    }
}

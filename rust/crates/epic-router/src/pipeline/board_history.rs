//! Java `autoroute/BoardHistory.java` (`:23-221`) — the bounded,
//! score-ranked board snapshot history the batch loop saves its
//! pre-pass boards into and restores from when a pass regresses
//! (T12 build items 5 and 6), plus the restore-by-copy model.
//!
//! ## Entries: bytes → clones
//!
//! Java's entry stores the FULL board as serialized bytes
//! (`serialize(false)`, `BoardSnapshotManager.java:34-36`) and
//! deserializes on restore. The port stores a [`Board`] clone — the
//! clone is the deserialize's value-semantics twin: an independent
//! copy of every persistent field. What deserialize does BEYOND
//! copying is the TRANSIENT reset ([`Board::reset_transient_after_restore`]:
//! `normalizeSuppressedNetNos` back to empty (`BasicBoard.readObject`
//! `:1393`), `changedArea`/`shoveFailingObstacle`/`shoveFailingLayer`
//! field-initialized (`RoutingBoard.java:69-75`)) — applied by
//! [`restore_from_snapshot`], not by the clone.
//!
//! ## The id-watermark rollback (the heaviest T12 parity item)
//!
//! `Communication.idGenerator` is NON-transient
//! (`Communication.java:30`) → the snapshot's watermark rides every
//! entry; a restore returns `max_generated_id()` to its SNAPSHOT-TIME
//! value, so post-restore allocations RE-BURN the ids the discarded
//! path consumed (ids may repeat across the discarded segment and the
//! post-restore segment). The clone gives this for free; pinned in
//! the tests.
//!
//! ## Gates the anchors call out
//!
//! * `getMaxScore` starts at 0, not −inf — an all-negative history
//!   reads 0 (`:134-147`).
//! * `add` at cap: the new board's score is computed FIRST; the
//!   entry is skipped when `newScore <= worstScore` (strict-better
//!   evictions only, `:86-88`).
//! * `restoreBoard(maxAllowedRestoreCount <= 0)` → `i32::MAX`
//!   (unlimited — `restoreBestBoard` is `restore_board(0)`).
//! * `restoreBoard` SORTS the list in place, score descending,
//!   stable (`Float.compare(o2, o1)`) — `getRank` semantics change
//!   from insertion order to score order across the call (`:162`).
//! * The hash identity (`contains`/`remove`/`getRank`) is the T12
//!   board hash ([`crate::pipeline::board_hash::board_hash`]); the
//!   string is banked-divergent from Java's MD5, the equality
//!   semantics are not.

use epic_board::board::Board;
use epic_board::tree_manager::SearchTreeManager;

use crate::pipeline::board_hash::board_hash;
use crate::pipeline::board_statistics::{BoardStatistics, RouterSettingsScoring};

/// Java `BoardHistory.BoardHistoryEntry` (`:207-220`). The score is
/// computed AT ADD TIME (`:217`) with the FULL
/// [`BoardStatistics::new`] walk (violations + connections).
#[derive(Clone, Debug)]
pub struct BoardHistoryEntry {
    /// Java `byte[] board` (`serialize(false)`) — the port's
    /// value-semantics twin, an independent full-board clone.
    pub board: Board,
    /// Java `String hash` — [`board_hash`] at add time.
    pub hash: String,
    /// Java `float score` — the router score at add time.
    pub score: f32,
    /// Java `int restoreCount` — how often this entry was restored.
    pub restore_count: i32,
}

impl BoardHistoryEntry {
    /// Java `new BoardHistoryEntry(board, routerSettings)`
    /// (`:214-219`): serialize + hash + score + `restoreCount = 0`.
    /// The score is the Java `BoardStatistics.getInstance(board)` walk
    /// on the LIVE board — the caller's tree manager, whose spatial
    /// trees the clearance-violation pass queries (a fresh empty
    /// manager undercounts to zero; the trees are the live board's own
    /// state). The stored copy is taken after the walk; the stats fill
    /// only pure memo caches (value-identical).
    fn new(
        board: &mut Board,
        manager: &mut SearchTreeManager,
        router_settings: &RouterSettingsScoring,
    ) -> Self {
        let hash = board_hash(board);
        let score = BoardStatistics::new(manager, board).get_router_score(Some(router_settings));
        Self {
            board: board.clone(),
            hash,
            score,
            restore_count: 0,
        }
    }
}

/// Java `BoardHistory` (`:23-221`) — bounded, score-ranked.
#[derive(Clone, Debug)]
pub struct BoardHistory {
    /// Java `List<BoardHistoryEntry> boards` (`:33`) — insertion
    /// order until the first `restoreBoard` sort.
    entries: Vec<BoardHistoryEntry>,
    /// Java `RouterSettings routerSettings` (`:34`) — the score face.
    router_settings: RouterSettingsScoring,
    /// Java `int maxHistorySize` (`:32`; default
    /// [`Self::MAX_HISTORY_SIZE`]).
    max_history_size: usize,
}

impl BoardHistory {
    /// Java `MAX_HISTORY_SIZE` (`:30`).
    pub const MAX_HISTORY_SIZE: usize = 30;

    /// Java `new BoardHistory(routerSettings)` (`:38-40`).
    #[must_use]
    pub fn new(router_settings: RouterSettingsScoring) -> Self {
        Self::with_cap(router_settings, Self::MAX_HISTORY_SIZE)
    }

    /// Java's package-private capped ctor (`:43-46`).
    #[must_use]
    pub fn with_cap(router_settings: RouterSettingsScoring, max_history_size: usize) -> Self {
        Self {
            entries: Vec::new(),
            router_settings,
            max_history_size,
        }
    }

    /// Java `add` (`:61-93`): dedup by hash; at the cap the new
    /// board's score is computed first (Java `:80-82`, on the LIVE
    /// board through its own trees — hence the manager parameter) and
    /// the add is SKIPPED unless it strictly beats the worst entry
    /// (which is then evicted).
    pub fn add(&mut self, manager: &mut SearchTreeManager, board: &mut Board) {
        if self.contains(board) {
            return;
        }
        if self.entries.len() >= self.max_history_size {
            // The score gate runs BEFORE the (expensive) serialization
            // — Java computes newScore first for the same reason.
            let new_score =
                BoardStatistics::new(manager, board).get_router_score(Some(&self.router_settings));
            // Linear scan, strict <: the FIRST worst entry wins ties.
            let mut worst_index = 0;
            let mut worst_score = self.entries[0].score;
            for (index, entry) in self.entries.iter().enumerate().skip(1) {
                if entry.score < worst_score {
                    worst_score = entry.score;
                    worst_index = index;
                }
            }
            if new_score <= worst_score {
                return;
            }
            self.entries.remove(worst_index);
        }
        self.entries.push(BoardHistoryEntry::new(
            board,
            manager,
            &self.router_settings,
        ));
    }

    /// Java `clear` (`:102-104`).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Java `contains` (`:107-120`) — hash equality over the entries.
    #[must_use]
    pub fn contains(&self, board: &Board) -> bool {
        let hash = board_hash(board);
        self.entries.iter().any(|entry| entry.hash == hash)
    }

    /// Java `remove` (`:123-131`) — the FIRST hash match.
    pub fn remove(&mut self, board: &Board) {
        let hash = board_hash(board);
        if let Some(index) = self.entries.iter().position(|entry| entry.hash == hash) {
            self.entries.remove(index);
        }
    }

    /// Java `getMaxScore` (`:134-147`) — starts at 0, NOT −inf.
    #[must_use]
    pub fn get_max_score(&self) -> f32 {
        let mut max_score = 0.0f32;
        for entry in &self.entries {
            if entry.score > max_score {
                max_score = entry.score;
            }
        }
        max_score
    }

    /// Java `restoreBoard` (`:153-174`): `maxAllowedRestoreCount <= 0`
    /// → unlimited; sorts the list IN PLACE (score descending, stable
    /// — `Float.compare(o2, o1)`); the first entry whose
    /// `restoreCount <= max` is incremented and returned as a fresh
    /// board (Java's deserialize product). `None` = Java's null.
    pub fn restore_board(&mut self, max_allowed_restore_count: i32) -> Option<Board> {
        let max_allowed_restore_count = if max_allowed_restore_count <= 0 {
            i32::MAX
        } else {
            max_allowed_restore_count
        };
        // Stable sort — ties keep insertion order. `total_cmp` is the
        // `Float.compare` total order (−0 < +0, NaN last); the scores
        // are clamped and never NaN.
        self.entries.sort_by(|a, b| b.score.total_cmp(&a.score));
        for entry in &mut self.entries {
            if entry.restore_count <= max_allowed_restore_count {
                entry.restore_count += 1;
                return Some(entry.board.clone());
            }
        }
        None
    }

    /// Java `restoreBestBoard` (`:177-179`) — unlimited gate.
    pub fn restore_best_board(&mut self) -> Option<Board> {
        self.restore_board(0)
    }

    /// Java `size` (`:182-189`).
    #[must_use]
    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// Java `getRank` (`:192-205`) — 1-indexed position in the
    /// CURRENT list order (insertion until the first restore sort,
    /// score order after), −1 when absent.
    #[must_use]
    pub fn get_rank(&self, board: &Board) -> i32 {
        let hash = board_hash(board);
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.hash == hash {
                return index as i32 + 1;
            }
        }
        -1
    }
}

/// The T12 restore model (Java `BasicBoard.deserialize` +
/// `readObject` `:1386-1398`, the anchors' "restore-by-copy"
/// adjudication): the snapshot becomes the live board BY VALUE, the
/// transients reset ([`Board::reset_transient_after_restore`] — the
/// suppression set, the changed-area session, the shove-failure
/// report; the id watermark and the failure log are NON-transient and
/// ROLL BACK with the copy), and the search-tree manager is REBUILT
/// (`new SearchTreeManager(this)` + the items re-inserted in
/// `getItems()` order — the descending-id `startReadObject` walk,
/// which is exactly `insert_all_board_items`).
pub fn restore_from_snapshot(manager: &mut SearchTreeManager, board: &mut Board, snapshot: &Board) {
    *board = snapshot.clone();
    board.reset_transient_after_restore();
    *manager = SearchTreeManager::new();
    manager.insert_all_board_items(board);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::board_statistics::{RouterScoreSettings, RouterScoringVersion};
    use crate::test_util::parse;
    use epic_board::board::ItemEntry;
    use epic_board::items::{FixedState, ItemData};
    use epic_board::trace_ops::{insert_trace_without_cleaning, remove_item_through_repository};
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polyline::Polyline;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`).
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn pt(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    /// A hand-built board carrying ONE trace whose geometry is the
    /// given corner pair — the cheap distinct-hash board factory.
    fn single_trace_board(x0: i32, y0: i32, x1: i32, y1: i32) -> Board {
        let mut board = Board::new();
        let id = board.alloc_id();
        board.insert_item(ItemEntry {
            id,
            data: ItemData::Trace {
                layer: 0,
                half_width: 500,
                lines: Polyline::from_two_corners(&pt(x0, y0), &pt(x1, y1)),
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        board
    }

    /// V2 box with every weight zeroed → ANY board scores exactly
    /// 1000.0 (the crafted-gate worlds need a predictable new-score).
    fn constant_score_settings() -> RouterSettingsScoring {
        RouterSettingsScoring {
            scoring: None,
            router_scoring: Some(RouterScoreSettings {
                version: RouterScoringVersion::V2Continuous,
                unrouted_free_fraction: Some(0.5),
                unrouted_first_half_weight: Some(0.0),
                unrouted_second_half_weight: Some(0.0),
                clearance_violation_count_weight: Some(0.0),
                clearance_violation_depth_weight: Some(0.0),
                clearance_violation_depth_scale: Some(1000.0),
            }),
            optimizer_scoring: None,
        }
    }

    /// V2 box with count weight −25, everything else 0: the score
    /// GROWS with the live violation count (1000 + 25·count/difficulty)
    /// — the real-flow eviction world needs boards whose score strictly
    /// increases with routed damage. The box is the test's own
    /// construction; Java's weights are user settings, sign-unchecked.
    fn inverted_count_settings() -> RouterSettingsScoring {
        RouterSettingsScoring {
            scoring: None,
            router_scoring: Some(RouterScoreSettings {
                version: RouterScoringVersion::V2Continuous,
                unrouted_free_fraction: Some(0.5),
                unrouted_first_half_weight: Some(0.0),
                unrouted_second_half_weight: Some(0.0),
                clearance_violation_count_weight: Some(-25.0),
                clearance_violation_depth_weight: Some(0.0),
                clearance_violation_depth_scale: Some(1000.0),
            }),
            optimizer_scoring: None,
        }
    }

    /// Directly crafted entry (white-box: the test module shares the
    /// struct's fields) with a chosen score and restore count.
    fn crafted(board: Board, score: f32, restore_count: i32) -> BoardHistoryEntry {
        BoardHistoryEntry {
            hash: board_hash(&board),
            board,
            score,
            restore_count,
        }
    }

    /// Two crossing traces (net 49 horizontal × net 94 vertical) in
    /// otherwise empty corridors — the t12_violation_world geometry,
    /// shifted by `y`: each call adds exactly ONE self-crossing
    /// violation (plus any fixture interaction, which only ever
    /// helps the strict-monotone world). Pairs 200 000 apart never
    /// touch each other (half_width 1500).
    fn add_crossing(manager: &mut SearchTreeManager, board: &mut Board, y: i32) {
        insert_trace_without_cleaning(
            manager,
            board,
            Polyline::from_two_corners(&pt(530_000, 210_000 + y), &pt(600_000, 210_000 + y)),
            0,
            1500,
            &[49],
            1,
            FixedState::Unfixed,
        )
        .expect("horizontal insert succeeds");
        insert_trace_without_cleaning(
            manager,
            board,
            Polyline::from_two_corners(&pt(560_000, 190_000 + y), &pt(560_000, 230_000 + y)),
            0,
            1500,
            &[94],
            1,
            FixedState::Unfixed,
        )
        .expect("vertical insert succeeds");
    }

    /// A parsed fixture mutated with one crossing pair at `y` — a
    /// REAL-rules board (with its populated tree manager) whose hash
    /// is distinct per `y`. The `add` score walk queries the manager's
    /// trees (a hand-built `Board::new()` hits the documented
    /// empty-matrix fatal in `max_value`); crafted entries are never
    /// scored, so only the boards handed to `add` need this.
    fn fixture_variant(y: i32) -> (SearchTreeManager, Board) {
        let (mut manager, mut board) = parse_fixture();
        add_crossing(&mut manager, &mut board, y);
        (manager, board)
    }

    /// The score of a board through its OWN populated tree manager
    /// (the manager cloned with the board shares item ids, so a fresh
    /// manager over the clone reproduces the live walk).
    fn score_with_own_trees(settings: &RouterSettingsScoring, board: &Board) -> f32 {
        let mut clone = board.clone();
        let mut mgr = SearchTreeManager::new();
        mgr.reinsert_tree_items(&mut clone);
        BoardStatistics::new(&mut mgr, &mut clone).get_router_score(Some(settings))
    }

    /// The entry captures the hash and the score AT ADD TIME on an
    /// INDEPENDENT board copy (Java serializes at `:217`; the clone is
    /// the port's twin): the entry's hash equals a direct `board_hash`
    /// of the live board, its score equals an independent
    /// `BoardStatistics` walk, and later mutations of the ORIGINAL do
    /// not leak into the entry (an aliasing bug fails the last arm).
    #[test]
    fn bhe_entry_captures_hash_score_and_independent_copy() {
        let settings = constant_score_settings();
        let (mut manager, mut board) = parse_fixture();
        let mut history = BoardHistory::new(settings);
        history.add(&mut manager, &mut board);
        assert_eq!(history.size(), 1);
        let entry = &history.entries[0];
        assert_eq!(entry.hash, board_hash(&board), "hash at add time");
        assert_eq!(entry.restore_count, 0, "fresh entries never restored");
        // independent recomputation of the score face (own trees).
        let expected = score_with_own_trees(&constant_score_settings(), &board);
        assert_eq!(entry.score, expected, "score computed at add time");
        assert_eq!(entry.score, 1000.0, "the zeroed weights saturate at 1000");

        // the entry's board is a COPY: mutating the original now must
        // not change it (Java's byte[] snapshot has this semantics).
        let entry_board_count = entry.board.item_count();
        let entry_hash = entry.hash.clone();
        let new_id = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&pt(-20_000, 0), &pt(20_000, 0)),
            0,
            1500,
            &[1],
            1,
            FixedState::Unfixed,
        )
        .expect("insert succeeds");
        assert_eq!(history.entries[0].board.item_count(), entry_board_count);
        assert_eq!(history.entries[0].hash, entry_hash);
        let _ = new_id;
    }

    /// `contains` dedups by hash: adding the same board twice is one
    /// entry (Java `:65-67`), a mutated board is a new one.
    #[test]
    fn bhe_contains_dedups_by_hash() {
        let (mut manager, mut board) = parse_fixture();
        let mut history = BoardHistory::new(constant_score_settings());
        history.add(&mut manager, &mut board);
        history.add(&mut manager, &mut board);
        assert_eq!(history.size(), 1, "identical board dedups");
        assert!(history.contains(&board));
        add_crossing(&mut manager, &mut board, 0);
        assert!(!history.contains(&board), "mutated board is a new state");
        history.add(&mut manager, &mut board);
        assert_eq!(history.size(), 2);
    }

    /// The cap gate (Java `:79-92`) with CRAFTED entries and the
    /// constant-score settings (any real add scores exactly 1000):
    /// the strict-better eviction removes the FIRST of equal-worst
    /// entries, an add scoring equal to the worst is SKIPPED (the
    /// `newScore <= worstScore` bound), and getMaxScore floors at 0.
    #[test]
    fn bhe_gate_eviction_first_worst_and_skip() {
        let board_x = single_trace_board(0, 0, 1000, 0);
        let board_y = single_trace_board(0, 0, 2000, 0);
        let board_z = single_trace_board(0, 0, 3000, 0);
        let mut history = BoardHistory::with_cap(constant_score_settings(), 3);
        history.entries = vec![
            crafted(board_x.clone(), 700.0, 0),
            crafted(board_y.clone(), 600.0, 0),
            crafted(board_z.clone(), 600.0, 0),
        ];
        assert_eq!(history.get_max_score(), 700.0);

        // add scores 1000 > worst 600 → evict the FIRST 600 (Y).
        // The `<=`-last-worst mutant evicts Z instead → contains(Y).
        // The newcomer is a REAL board (the score walk inside `add`
        // needs the parsed rules; crafted entries are never scored).
        let (mut new_mgr, mut newcomer) = fixture_variant(0);
        assert_ne!(board_hash(&newcomer), board_hash(&board_x));
        history.add(&mut new_mgr, &mut newcomer);
        assert_eq!(history.size(), 3);
        assert!(!history.contains(&board_y), "the FIRST worst is evicted");
        assert!(history.contains(&board_z), "the tied later entry stays");

        // the skip arm: cap 1, existing score == the constant new score
        // 1000 → `1000 <= 1000` skips (no evict, no growth). The
        // strict-`<` mutant evicts X and pushes the newcomer → the
        // entry identity check below fails.
        let mut capped = BoardHistory::with_cap(constant_score_settings(), 1);
        capped.entries = vec![crafted(board_x.clone(), 1000.0, 0)];
        capped.add(&mut new_mgr, &mut newcomer);
        assert_eq!(capped.size(), 1, "equal-score add skips");
        assert_eq!(capped.entries[0].score, 1000.0);
        assert_eq!(capped.entries[0].hash, board_hash(&board_x), "X stayed");
    }

    /// getMaxScore floors at 0 (Java initializes `maxScore = 0`, NOT
    /// −inf): an all-negative history reads 0.0; the −inf mutant
    /// returns −10. A positive maximum is reported verbatim.
    #[test]
    fn bhe_get_max_score_floors_at_zero() {
        let mut history = BoardHistory::new(constant_score_settings());
        history.entries = vec![
            crafted(single_trace_board(0, 0, 1000, 0), -50.0, 0),
            crafted(single_trace_board(0, 0, 2000, 0), -10.0, 0),
        ];
        assert_eq!(history.get_max_score(), 0.0, "the floor is 0, not −inf");
        history
            .entries
            .push(crafted(single_trace_board(0, 0, 3000, 0), 30.0, 0));
        assert_eq!(history.get_max_score(), 30.0);
    }

    /// The REAL-flow eviction: inverted count weight makes the score
    /// strictly grow with violation damage; at the cap the third board
    /// (worst damage) beats the worst entry (the clean base) and
    /// evicts it. Kills an eviction-dropped mutant and a
    /// gate-recomputes-skip mutant. World validation asserts the
    /// monotone scores first — if the fixture carried no violations
    /// the world would be vacuous.
    #[test]
    fn bhe_real_flow_eviction_monotone_scores() {
        let settings = inverted_count_settings();
        let (mut manager, mut base) = parse_fixture();
        let score_of = |board: &Board| score_with_own_trees(&settings, board);
        let s_base = score_of(&base);

        let mut cross1 = base.clone();
        add_crossing(&mut manager, &mut cross1, 0);
        let s_cross1 = score_of(&cross1);

        let mut cross2 = cross1.clone();
        add_crossing(&mut manager, &mut cross2, 4000);
        let s_cross2 = score_of(&cross2);

        assert!(
            s_base < s_cross1 && s_cross1 < s_cross2,
            "world check: scores strictly grow with damage ({s_base} < {s_cross1} < {s_cross2})"
        );

        let mut history = BoardHistory::with_cap(settings, 2);
        history.add(&mut manager, &mut base);
        history.add(&mut manager, &mut cross1);
        assert_eq!(history.size(), 2, "below the cap everything lands");
        history.add(&mut manager, &mut cross2);
        assert_eq!(history.size(), 2, "the cap holds");
        assert!(!history.contains(&base), "the worst entry was evicted");
        assert!(history.contains(&cross1));
        assert!(history.contains(&cross2));
        assert_eq!(
            history.get_max_score(),
            s_cross2,
            "the max tracks the worst-damage board"
        );
    }

    /// `restoreBoard` (Java `:153-174`): the IN-PLACE stable sort by
    /// score desc (getRank's order changes across the call), the
    /// restoreCount gate (`<= max`, first eligible wins, incremented),
    /// the exhausted → null arm, and the `<= 0 → unlimited` normalization
    /// (negative included — the `== 0`-only mutant fails the −5 arm).
    #[test]
    fn bhe_restore_board_gates_sort_and_rank() {
        let board_a = single_trace_board(0, 0, 1000, 0);
        let board_b = single_trace_board(0, 0, 2000, 0);
        let board_c = single_trace_board(0, 0, 3000, 0);
        let mut history = BoardHistory::new(constant_score_settings());
        // insertion order C(850, rc5), A(900, rc1), B(800, rc0).
        history.entries = vec![
            crafted(board_c.clone(), 850.0, 5),
            crafted(board_a.clone(), 900.0, 1),
            crafted(board_b.clone(), 800.0, 0),
        ];
        // getRank is 1-indexed in the CURRENT (insertion) order; a
        // foreign board reads −1.
        assert_eq!(history.get_rank(&board_c), 1);
        assert_eq!(history.get_rank(&board_a), 2);
        assert_eq!(history.get_rank(&board_b), 3);
        assert_eq!(history.get_rank(&single_trace_board(9, 9, 10, 9)), -1);

        // restore_board(1): sort → A(900/rc1), C(850/rc5), B(800/rc0);
        // A is the first eligible (rc 1 <= 1) → returned, rc → 2.
        let restored = history.restore_board(1).expect("A is eligible");
        assert_eq!(board_hash(&restored), board_hash(&board_a));
        // the sort MUTATED the rank order (stable: C before B on the
        // unchanged relative order).
        assert_eq!(history.get_rank(&board_a), 1);
        assert_eq!(history.get_rank(&board_c), 2);
        assert_eq!(history.get_rank(&board_b), 3);
        assert_eq!(history.entries[0].restore_count, 2, "A's gate count grew");

        // second restore at max 1: A exhausted, C exhausted → B.
        let restored = history.restore_board(1).expect("B is eligible");
        assert_eq!(board_hash(&restored), board_hash(&board_b));
        assert_eq!(history.entries[2].restore_count, 1);

        // restore_board(0) → UNLIMITED: A returns despite rc 2 > 0.
        let restored = history.restore_board(0).expect("0 means unlimited");
        assert_eq!(board_hash(&restored), board_hash(&board_a));
        // a NEGATIVE gate is unlimited too (Java `<= 0`).
        let restored = history.restore_board(-5).expect("negative is unlimited");
        assert_eq!(board_hash(&restored), board_hash(&board_a));
        assert_eq!(history.entries[0].restore_count, 4);

        // at max 1, B (rc 1) is served exactly once more, and THEN the
        // history is exhausted: A rc 4, B rc 2, C rc 5 — nothing
        // eligible → None (Java null).
        let restored = history.restore_board(1).expect("B once more");
        assert_eq!(board_hash(&restored), board_hash(&board_b));
        assert_eq!(history.entries[2].restore_count, 2);
        assert!(history.restore_board(1).is_none(), "exhausted");
    }

    /// The sort is STABLE on score ties (Java `Float.compare(o2, o1)`
    /// via a stable sort): equal scores keep insertion order and the
    /// FIRST one is restored. An unstable/unstable-by-id sort fails
    /// the returned-board check whenever Y precedes X by id.
    #[test]
    fn bhe_restore_ties_keep_insertion_order() {
        let board_x = single_trace_board(0, 0, 1000, 0);
        let board_y = single_trace_board(0, 0, 2000, 0);
        let mut history = BoardHistory::new(constant_score_settings());
        // X inserted first, Y second, EQUAL scores. ids: y > x.
        history.entries = vec![
            crafted(board_x.clone(), 700.0, 0),
            crafted(board_y.clone(), 700.0, 0),
        ];
        let restored = history.restore_board(5).expect("tie restore");
        assert_eq!(
            board_hash(&restored),
            board_hash(&board_x),
            "the tie keeps insertion order (stable sort)"
        );
    }

    /// `remove` takes the FIRST hash match only; `clear` empties.
    /// The duplicate-hash entries are crafted directly (the public
    /// `add` would dedup them).
    #[test]
    fn bhe_remove_first_match_and_clear() {
        let board_a = single_trace_board(0, 0, 1000, 0);
        let mut history = BoardHistory::new(constant_score_settings());
        let hash = board_hash(&board_a);
        history.entries = vec![
            BoardHistoryEntry {
                hash: hash.clone(),
                board: board_a.clone(),
                score: 500.0,
                restore_count: 0,
            },
            BoardHistoryEntry {
                hash: hash.clone(),
                board: single_trace_board(0, 0, 2000, 0),
                score: 600.0,
                restore_count: 0,
            },
        ];
        history.remove(&board_a);
        assert_eq!(history.size(), 1, "only the first match went");
        assert_eq!(history.entries[0].score, 600.0, "the SECOND entry stayed");
        history.clear();
        assert_eq!(history.size(), 0);
        assert_eq!(history.get_max_score(), 0.0);
    }

    /// THE restore model (T12 item 6): `*board = snapshot.clone()` +
    /// the transient reset + a REBUILT tree manager. Pinned faces:
    /// content rollback (the post-snapshot trace U is gone, the
    /// pre-snapshot trace T is back), the ID-WATERMARK rollback
    /// (allocations after the restore RE-BURN the discarded path's
    /// ids — the clone carries the snapshot-time generator), the
    /// shove transients reset, the failure LOG rolls back (Java's
    /// failureLog is non-transient — the snapshot carries it), and
    /// the search tree answers spatial queries again with EXACTLY the
    /// snapshot items (kills the forgot-insert mutant → empty pick,
    /// and the forgot-manager-reset mutant → double entries).
    ///
    /// COVERAGE NOTE (quality MINOR-5): the batch-side wiring that
    /// CALLS this function — the mid-loop restore arm (rank check +
    /// recompute + restore row) and the final best-restore body — is
    /// unreachable-on-fixture in EITHER engine and stays covered by
    /// this pin ONLY until a natural regressing-pass world exists;
    /// see SEAM's "Mid-loop restore gate reachability" row.
    #[test]
    fn bhe_restore_from_snapshot_model() {
        let (mut manager, mut board) = parse_fixture();

        // T: a pre-snapshot trace at a known point.
        let t_id = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&pt(-10_000, -10_000), &pt(-9_000, -9_000)),
            0,
            1500,
            &[1],
            1,
            FixedState::Unfixed,
        )
        .expect("T insert");
        let t_mid = pt(-9_500, -9_500);
        let snapshot = board.clone();
        let snapshot_count = board.item_count();
        let watermark_at_snapshot = board.max_generated_id();
        let failure_count_at_snapshot = board.failure_log.failure_count(u64::from(t_id.get()));

        // evolve the live world past the snapshot.
        remove_item_through_repository(&mut manager, &mut board, t_id);
        assert!(
            !manager.pick_items(&mut board, &t_mid, 0).contains(&t_id),
            "pre-restore: T is out of the tree"
        );
        let burn1 = board.alloc_id();
        let burn2 = board.alloc_id();
        assert_eq!(board.max_generated_id(), watermark_at_snapshot + 2);
        let u_id = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&pt(30_000, 30_000), &pt(31_000, 31_000)),
            0,
            1500,
            &[1],
            1,
            FixedState::Unfixed,
        )
        .expect("U insert");
        board.set_shove_failing_layer(1);
        board.set_shove_failing_obstacle(Some(u_id));
        board
            .failure_log
            .record_failure(u64::from(u_id.get()), 1, 7, "FAILED", None);
        assert_eq!(board.failure_log.failure_count(u64::from(u_id.get())), 1);
        assert_eq!(
            failure_count_at_snapshot, 0,
            "world check: T had no failures"
        );

        // THE restore.
        restore_from_snapshot(&mut manager, &mut board, &snapshot);

        // content rollback.
        assert_eq!(board.item_count(), snapshot_count, "U is gone");
        assert!(board.get(u_id).is_none());
        assert!(board.get(t_id).is_some(), "T is back");
        // id watermark rollback + the re-burn: the next allocation
        // hands out watermark+1, the id the DISCARDED path burned as
        // burn1 (ids may duplicate across the discarded and the
        // post-restore segments — Java's non-transient generator).
        assert_eq!(board.max_generated_id(), watermark_at_snapshot);
        let reburn = board.alloc_id();
        assert_eq!(reburn, burn1, "the discarded path's id is re-burned");
        assert_ne!(reburn, burn2);
        // the transients reset...
        assert_eq!(board.shove_failing_layer(), -1);
        assert_eq!(board.shove_failing_obstacle(), None);
        // ...the failure log ROLLED BACK (non-transient in Java)...
        assert_eq!(
            board.failure_log.failure_count(u64::from(u_id.get())),
            0,
            "the post-snapshot failure rode the discarded copy away"
        );
        // ...and the tree answers exactly the snapshot's content.
        let picked = manager.pick_items(&mut board, &t_mid, 0);
        assert_eq!(
            picked.len(),
            1,
            "exactly one item at T's midpoint (a stale manager double-inserts)"
        );
        assert_eq!(picked[0], t_id, "and it is T");
        let u_mid = pt(30_500, 30_500);
        assert!(
            manager.pick_items(&mut board, &u_mid, 0).is_empty(),
            "U is out of the rebuilt tree"
        );
    }
}

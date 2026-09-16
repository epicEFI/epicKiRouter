---
description: learned preferences, project conventions, and Do-Not-Repeat rules
budget_tokens: 2000
---
# Cerebrum

> OpenWolf's learning memory. Updated automatically as the AI learns from interactions.
> Do not edit manually unless correcting an error.
> Last updated: 2026-09-12

## User Preferences

<!-- How the user likes things done. Code style, tools, patterns, communication. -->

## Key Learnings

- **Project:** EpicRouter
- **Description:** <p align="center">

## Do-Not-Repeat

<!-- Mistakes made and corrected. Each entry prevents the same mistake recurring. -->
<!-- Format: [YYYY-MM-DD] Description of what went wrong and what to do instead. -->

- [2026-09-12] Oracle pins have three failure modes, all seen in M1a reviews: (1) memory-reconstructed expectations instead of jar-captured values (bug-045); (2) degenerate inputs where the terms under test cancel — e.g. fast-path pins at x=0 can't catch sign swaps (T5 gap 1, T6 finding 1); (3) pins that never execute the branch they claim to cover — e.g. a T17 bug-compat pin whose geometry takes the other branch (T6 finding 2). For every pin: capture from the oracle, choose inputs where the mutated terms differ, and confirm the branch actually runs.
- [2026-09-12] Two more pin failure modes found in M1b Tasks 1-2: (4) anchor-blind agreement — pinning only the input form where a wrong implementation coincidentally matches the oracle (`(string_quote ')` matched while `(string_quote 'a)` diverged; IGNORE_QUOTE bug, Task 1 review). Pins must include a form where wrong and right differ. (5) capture-tool rounding — Java `FloatPoint.toString()` rounds to 4 fraction digits, so pinning through it records a rounded lie (`7` vs exact `7.000000000000002`); capture doubles via `Double.toString`/raw bits, never through a formatting toString.
- [2026-09-14] Wedge false-positive: a subagent transcript frozen for ~3 min is NOT proof of a wedge — a long model-generation turn or a minutes-long cargo/gates run looks identical in its first minutes (Task 9 spec review: 3-min freeze, then 108KB→370KB growth on re-sample). Before TaskStop + re-dispatch, take a SECOND size sample after a ~4 min grace interval; growth = healthy. Also: pins never land in the digest warnings surface from FRLogger sites — the D12 parity warnings list is fed ONLY by `Wiring.java`'s `warnings.add` sites via `ReadScopeParameter.warnings` (Task 6 jar correction); every other FRLogger.warn is log-only and wiring `push_warning` there breaks WARN_COUNT parity.
- [2026-09-14] Pin failure mode (6), found in the M2 T3 fix round: a MEMOIZED oracle accessor makes RE-QUERYING the same object pin the CACHE, not the code path. `Pin.getShape` memoizes the whole shape array on first call (`precalculatedShapes`, Pin.java:170-236), so re-querying one pin after flipping `flipStyleRotateFirst` returned byte-identical cached shapes (verified empirically) — a "both styles" pin built on re-query passes even with the mirror-order port wrong. Fix: insert a FRESH object under the changed flag/table (SPIKE3 vs SPIKE2) and check for cache-memoizing fields (`precalculated*`, set-once caches like `Pin.getCenter`'s `setCenter`) before designing a re-query pin.
- [2026-09-14] Pin failure mode (7), found in the M2 T8 quality review: an EQUALITY pin (`dispatch == plain branch`) is blind when the two compared paths COINCIDE on the pinned input — an inverted flag, a wrong flag source (manager vs tree), and an always-plain port all pass. A pin asserting two paths agree needs a CONTRAST witness that disagrees on the same input (`dispatch == plain != core`), and a flag-source pin needs an input where the two candidate flags DIFFER (autoroute class-1 tree with compensation off: tree flag true, manager flag false). Sharpening of mode (4) for dispatch/equality-shaped pins.
- [2026-09-15] Pin failure mode (8), found in the M2 T10 contacts round: a pin literal RECONSTRUCTED from a capture's narrative (assumed trace 1372 had 2 corners because M_PAIR described an endpoint-on-mid-corner pair) failed against a faithful port — the capture row pinned only first/last (`M_Y 1372 0 first=... last=...`), the DSN wire is 3-corner, and `(type protect)` = USER_FIXED makes normalizeAllTraces a no-op there. Rule: every expected value must be a LITERAL capture row; when a capture shows only part of a structure, pin only that part. Related: pins asserting ABSOLUTE ids of post-parse insertions must replay the oracle's FULL insertion sequence (alloc is monotone — skipping earlier inserts shifts every id).
- [2026-09-15] Pin failure mode (9), found in the M2 T15 quality review: a SELECTION pin (`first-qualifying-drill-wins`) is blind when the pinned input's candidate behaviors COINCIDE — with ONE qualifier + a fall-through, the last-qualifier mutant (M8: walk all drills, remember the last, quirk intact) returns the same verdict and survives every gate. A selection/order pin needs an input where the alternative selection rules actually DIFFER (TWO qualifiers with different centers: first=(2000,0) vs last=(1500,0)). Extension of mode (7) from equality pins to winner-selection pins; the doc comment must not assert "both qualify" unless the vector makes it true.

## Decision Log

<!-- Significant technical decisions with rationale. Why X was chosen over Y. -->

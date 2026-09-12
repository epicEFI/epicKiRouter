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

## Decision Log

<!-- Significant technical decisions with rationale. Why X was chosen over Y. -->

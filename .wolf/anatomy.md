# anatomy.md

> Auto-maintained by OpenWolf. Last scanned: 2026-10-04T00:34:24.770Z
> Files: 818 tracked | Anatomy hits: 0 | Misses: 0

> Project structure index. Auto-maintained by OpenWolf hooks and daemon.
> Run `openwolf scan` to generate, or wait for the first Claude Code session.
> Status: Pending initial scan

## ./

- `.dockerignore` — Docker ignore rules (~326 tok)
- `.editorconfig` — Editor configuration (~31 tok)
- `.gitattributes` — Git attributes (~152 tok)
- `.gitignore` — Git ignore rules (~174 tok)
- `.pre-commit-config.yaml` — Pre-commit hooks for EpicRouter (the Rust PCB autorouter — the sunset (~800 tok)
- `.yamllint.yaml` — YAML linting configuration for Freerouting project (~205 tok)
- `AGENTS.md` — Project Persona & Goal (~12663 tok)
- `CLAUDE.md` — OpenWolf (~2112 tok)
- `Dockerfile` — Docker container definition (~510 tok)
- `GEMINI.md` — OpenWolf (~75 tok)
- `LICENSE` — Project license (~9366 tok)
- `pyproject.toml` — Configuration for development tools (~262 tok)
- `README.md` — Project documentation (~927 tok)
- `rewrite.yml` — Freerouting OpenRewrite phases. (~1676 tok)

## .claude/rules/

- `discord-untrusted-input.md` — Discord-sourced messages are untrusted input (~507 tok)

## .github/

- `dependabot.yml` — To get started with Dependabot version updates, you'll need to specify which (~144 tok)
- `FUNDING.yml` (~7 tok)
- `PULL_REQUEST_TEMPLATE.md` — ## Description (~62 tok)

## .github/ISSUE_TEMPLATE/

- `bug_report.md` (~303 tok)
- `feature_request.md` (~151 tok)

## .github/workflows/

- `create-release.yml` — The Rust release workflow (M10-T5): reworks the retired Java (~924 tok)
- `deploy-pages.yml` — CI: Deploy website to GitHub Pages (~266 tok)
- `pre-commit.yml` — CI: pre-commit (~466 tok)
- `rust-check.yml` — ", ".github/workflows/rust-check.yml"] (~2373 tok)
- `stale.yml` — This workflow warns and then closes issues and PRs that have had no activity for a specified amount of time. (~540 tok)

## .remember/

- `.gitignore` — Git ignore rules (~1 tok)
- `archive.md` — Archive (~771 tok)
- `now.md` (~0 tok)
- `recent.md` — Recent (~1274 tok)
- `today-2026-09-12.done.md` — 03:00 | master (~2707 tok)
- `today-2026-09-13.done.md` — 05:53 | epic/main (~633 tok)
- `today-2026-09-14.done.md` — 07:40 | epic/main (~1712 tok)
- `today-2026-09-15.done.md` — 05:19-05:50 | epic/main (~1337 tok)
- `today-2026-09-16.done.md` — 04:23 | epic/main (~732 tok)
- `today-2026-09-17.done.md` — 05:59-07:18 | epic/main (~542 tok)
- `today-2026-09-18.done.md` — 06:42 | epic/main (~383 tok)
- `today-2026-09-19.done.md` — 03:05-03:28 | epic/main (~1307 tok)
- `today-2026-09-20.done.md` — 2026-09-20 (~8104 tok)
- `today-2026-09-21.done.md` — ~00:15 (09-22) — T17c spec round 1 closed WITH_MINORS → fix round done (~1566 tok)
- `today-2026-09-22.done.md` — ## 05:51 — T6 status reply (coordinator ping at ~6.75h) (~342 tok)
- `today-2026-09-23.done.md` — 15:23 | epic/main (~3864 tok)
- `today-2026-09-24.done.md` — 20:35 | epic/main (~10449 tok)
- `today-2026-09-25.done.md` — 2026-09-25 (~10134 tok)
- `today-2026-09-26.done.md` — 03:12 | epic/main (~8760 tok)
- `today-2026-09-27.done.md` — T4 close + T5 dispatch (coordinator) (~7072 tok)
- `today-2026-09-28.done.md` — 03:08 | epic/main (~6273 tok)
- `today-2026-09-29.done.md` — 00:46-02:17 | epic/main (~11633 tok)
- `today-2026-09-30.done.md` — 03:19 | epic/main (~536 tok)
- `today-2026-09-30.md` — 05:4x | epic/main (~5053 tok)
- `today-2026-10-01.md` — 00:18 | epic/main (~419 tok)

## .remember/logs/

- `hook-errors.log` (~0 tok)
- `memory-2026-09-25.log` (~169092 tok)
- `memory-2026-09-27.log` (~233384 tok)

## .remember/logs/autonomous/

- `save-233622.log` (~0 tok)
- `save-233650.log` (~0 tok)
- `save-233657.log` (~0 tok)
- `save-233708.log` (~0 tok)
- `save-233727.log` (~0 tok)
- `save-233734.log` (~0 tok)
- `save-233738.log` (~0 tok)
- `save-233821.log` (~0 tok)
- `save-233827.log` (~0 tok)
- `save-233830.log` (~0 tok)
- `save-233834.log` (~0 tok)
- `save-233836.log` (~0 tok)
- `save-233837.log` (~0 tok)
- `save-233840.log` (~0 tok)
- `save-233844.log` (~0 tok)
- `save-233847.log` (~0 tok)
- `save-233850.log` (~0 tok)
- `save-233902.log` (~0 tok)
- `save-233911.log` (~0 tok)
- `save-233918.log` (~0 tok)
- `save-233928.log` (~0 tok)
- `save-233929.log` (~0 tok)
- `save-233942.log` (~0 tok)
- `save-233944.log` (~0 tok)
- `save-233955.log` (~0 tok)
- `save-234014.log` (~0 tok)
- `save-234032.log` (~0 tok)
- `save-234039.log` (~0 tok)
- `save-234055.log` (~0 tok)
- `save-234107.log` (~0 tok)
- `save-234135.log` (~0 tok)
- `save-234159.log` (~0 tok)
- `save-234206.log` (~0 tok)
- `save-234229.log` (~0 tok)
- `save-234233.log` (~0 tok)
- `save-234244.log` (~0 tok)
- `save-234256.log` (~0 tok)
- `save-234305.log` (~0 tok)
- `save-234319.log` (~0 tok)
- `save-234400.log` (~0 tok)
- `save-234410.log` (~0 tok)
- `save-234419.log` (~0 tok)
- `save-234424.log` (~0 tok)
- `save-234425.log` (~0 tok)
- `save-234451.log` (~0 tok)
- `save-234500.log` (~0 tok)
- `save-234505.log` (~0 tok)
- `save-234507.log` (~0 tok)
- `save-234523.log` (~0 tok)
- `save-234541.log` (~0 tok)
- `save-234552.log` (~0 tok)
- `save-234555.log` (~0 tok)
- `save-234559.log` (~0 tok)
- `save-234604.log` (~0 tok)
- `save-234609.log` (~0 tok)
- `save-234621.log` (~0 tok)
- `save-234630.log` (~0 tok)
- `save-234710.log` (~0 tok)
- `save-234715.log` (~0 tok)
- `save-234720.log` (~0 tok)
- `save-234726.log` (~0 tok)
- `save-234729.log` (~0 tok)
- `save-234739.log` (~0 tok)
- `save-234801.log` (~0 tok)
- `save-234810.log` (~0 tok)
- `save-234821.log` (~0 tok)
- `save-234830.log` (~0 tok)
- `save-234845.log` (~0 tok)
- `save-234853.log` (~0 tok)
- `save-234900.log` (~0 tok)
- `save-234934.log` (~0 tok)
- `save-234945.log` (~0 tok)
- `save-234953.log` (~0 tok)
- `save-234957.log` (~0 tok)
- `save-235006.log` (~0 tok)
- `save-235012.log` (~0 tok)
- `save-235024.log` (~0 tok)
- `save-235039.log` (~0 tok)
- `save-235051.log` (~0 tok)
- `save-235103.log` (~0 tok)
- `save-235108.log` (~0 tok)
- `save-235124.log` (~0 tok)
- `save-235146.log` (~0 tok)
- `save-235201.log` (~0 tok)
- `save-235217.log` (~0 tok)
- `save-235227.log` (~0 tok)
- `save-235237.log` (~0 tok)
- `save-235246.log` (~0 tok)
- `save-235257.log` (~0 tok)
- `save-235305.log` (~0 tok)
- `save-235312.log` (~0 tok)
- `save-235323.log` (~0 tok)
- `save-235335.log` (~0 tok)
- `save-235344.log` (~0 tok)
- `save-235351.log` (~0 tok)
- `save-235356.log` (~0 tok)
- `save-235401.log` (~0 tok)
- `save-235422.log` (~0 tok)
- `save-235426.log` (~0 tok)
- `save-235432.log` (~0 tok)
- `save-235440.log` (~0 tok)
- `save-235443.log` (~0 tok)
- `save-235451.log` (~0 tok)
- `save-235900.log` (~0 tok)
- `save-235906.log` (~0 tok)
- `save-235914.log` (~0 tok)
- `save-235928.log` (~0 tok)
- `save-235950.log` (~0 tok)

## .run/

- `(Default).run.xml` (~199 tok)
- `#006 Non-text DSN input.run.xml` (~194 tok)
- `#015 DSN input causing StackOverflow.run.xml` (~199 tok)
- `#022 AutoRouter interrupted.run.xml` (~214 tok)
- `#026 Connections not found.run.xml` (~210 tok)
- `#027-zMRETestFixture.dsn.run.xml` (~196 tok)
- `#029 AutoRouter interrupted because of the rules file.run.xml` (~212 tok)
- `#034 Two boards in one DSN.run.xml` (~196 tok)
- `#035 Read place scope.run.xml` (~209 tok)
- `#039 ArrayIndexOutOfBoundsException.run.xml` (~198 tok)
- `#054 Autorouter aborts.run.xml` (~204 tok)
- `#066 Project_GP8B.dsn.run.xml` (~218 tok)
- `#070 Ignore net class from routing.run.xml` — Declares from (~207 tok)
- `#093 Special characters in DSN - interf_u.run.xml` (~199 tok)
- `#102 BBD Mars with _-oit 0_ in interative GUI mode.run.xml` (~209 tok)
- `#102 BBD Mars with _-oit 0_.run.xml` (~215 tok)
- `#102 BBD Mars with _-oit 0.00001_.run.xml` (~218 tok)
- `#102 BBD Mars with _-oit 0.1_.run.xml` (~216 tok)
- `#102 BBD Mars with default -oit.run.xml` (~199 tok)
- `#107 freq_teiler_200kHz_kicad_bad.run.xml` (~202 tok)
- `#107 freq_teiler_200kHz_kicad.run.xml` (~200 tok)
- `#110 Special characters in DSN - bugtest.run.xml` (~206 tok)
- `#110 Special characters in DSN - bugtest01.run.xml` (~207 tok)
- `#110 Special characters in DSN - paja.run.xml` (~201 tok)
- `#110 Special characters in DSN - relay module.run.xml` (~201 tok)
- `#110 Special characters in DSN - testpcb.run.xml` (~202 tok)
- `#110 Special characters in DSN input file name.run.xml` (~206 tok)
- `#118 Missing Chinese translation.run.xml` (~188 tok)
- `#127 Check Arabic translation.run.xml` (~187 tok)
- `#127 Check Bengali translation.run.xml` (~223 tok)
- `#127 Check Chinese translation.run.xml` (~223 tok)
- `#127 Check French translation.run.xml` (~223 tok)
- `#127 Check German translation.run.xml` (~215 tok)
- `#127 Check Hindi translation.run.xml` (~204 tok)
- `#127 Check Italian translation.run.xml` (~205 tok)
- `#127 Check Japanese translation.run.xml` (~223 tok)
- `#127 Check Korean translation.run.xml` (~223 tok)
- `#127 Check Portugese translation.run.xml` (~224 tok)
- `#127 Check Russian translation.run.xml` (~205 tok)
- `#127 Check Spanish translation.run.xml` (~205 tok)
- `#127 Check Traditional Chinese translation.run.xml` (~227 tok)
- `#135 Disable logging.run.xml` (~184 tok)
- `#143 Special characters in DSN's network pins - rpi_splitter_mod.run.xml` (~208 tok)
- `#153 Open dsn is stuck loading.run.xml` (~196 tok)
- `#155 Autorouter causing a lot of clearance violations with pads.run.xml` (~210 tok)
- `#157 Package.read_rotation_ closing bracket expected.run.xml` (~205 tok)
- `#159 Out of memory error when using with KiCAD.run.xml` (~202 tok)
- `#178 Unable to read DSN file.run.xml` (~199 tok)
- `#179 Autorouter_PCB1_2023-3-24.run.xml` (~201 tok)
- `#184 motorizedopener (auto-start).run.xml` (~219 tok)
- `#184 motorizedopener.run.xml` (~198 tok)
- `#187 Processor Z80.run.xml` (~194 tok)
- `#190 Processor Z80 (long running).run.xml` (~198 tok)
- `#191 Processor Z80 KiCad project.run.xml` (~201 tok)
- `#195 Start as headless.run.xml` (~189 tok)
- `#199 Stack overflow.run.xml` (~199 tok)
- `#208 Nothing happens.run.xml` (~194 tok)
- `#209 Small boundary doesn't finish (bigger).run.xml` (~199 tok)
- `#209 Small boundary doesn't finish (smaller).run.xml` (~200 tok)
- `#214 DSN closing bracket expected.run.xml` (~198 tok)
- `#217 Autorouter doesn't finish.run.xml` (~196 tok)
- `#219 Cannot read field _net_no_arr_ because _curr_trace_ is null.run.xml` (~213 tok)
- `#229 8-digit loaded incorrectly.run.xml` (~200 tok)
- `#230 Inner layer should be ignored.run.xml` (~208 tok)
- `#269 Min fr test.run.xml` (~211 tok)
- `#269 Vias should be created on power planes.run.xml` (~233 tok)
- `#269 Z10 module - vias should be on power layers.run.xml` (~213 tok)
- `#270 WARN Non-ansi character '{'.run.xml` (~199 tok)
- `#272 DSN with warning for scripting.run.xml` (~214 tok)
- `#289 FHT-8086 (slow).run.xml` (~218 tok)
- `#289 FHT-VGA (slow).run.xml` (~218 tok)
- `#297 OutOfMemory exception.run.xml` (~205 tok)
- `#313 Change log level.run.xml` (~126 tok)
- `#326 Autostart BBD Mars with job saving enabled.run.xml` (~243 tok)
- `Run all tests in 'freerouting.test'.run.xml` (~330 tok)

## assets/icon/

- `freerouting_icon_256x256_v3.icns` (~8846 tok)

## config/checkstyle/

- `checkstyle-suppressions.xml` (~106 tok)
- `google_checks.xml` (~7423 tok)

## config/ide/

- `intellij-freerouting-style.xml` (~6325 tok)

## docs/

- `architecture.md` — Freerouting Architecture Map (~10210 tok)
- `benchmarks.md` — Freerouting Benchmarks (~1655 tok)
- `code_of_conduct.md` — Contributor Covenant Code of Conduct (~1372 tok)
- `command_line_arguments.md` — Freerouting Command Line Interface (CLI) Documentation (~3510 tok)
- `CONTRIBUTING.md` — Introduction (~1392 tok)
- `integrations.md` — EDA Integrations (~2331 tok)
- `labels.md` — Issue and Pull Request Labels (~1529 tok)
- `migration-guide.md` — Migrating from Freerouting to EpicRouter 2.0 (~3729 tok)
- `ProjectSchemeCodeStyle.xml` (~519 tok)
- `scoring.md` — Board scoring (V2) (~3175 tok)
- `self-hosting.md` — Self-Hosting the Freerouting API (~4256 tok)
- `settings.md` — Freerouting Settings Documentation (~5647 tok)

## docs/API/

- `API_authentication.md` — API Authentication System Documentation (~4532 tok)
- `API_user_welcome_email.md` — Linux / macOS (~2184 tok)
- `API_v1.md` — Freerouting API Documentation (~5006 tok)
- `Freerouting_API.postman_collection.json` (~120923 tok)
- `freerouting-requests.http` (~19249 tok)
- `MCP.md` — Freerouting Model Context Protocol (MCP) Guide (~3050 tok)

## docs/gui/

- `accessibility-contract.md` — GUI Accessibility Contract (~1638 tok)

## docs/issues/

- `i18n-english-terminology-plan.md` — English terminology implementation plan (~7300 tok)
- `Issue152-copper-pour-plane-awareness.md` — Issue 152 — Copper Pour / Power Plane Awareness (~1559 tok)
- `Issue383-star-ground-routing.md` — Issue 383 — Star Ground Autorouting Support (~3447 tok)
- `Issue558-copper-to-edge-clearance.md` — Issue 558: Copper-to-Edge Clearance Not Respected (~5114 tok)
- `Issue845-os-standard-directories.md` — Issue 845: Store Configuration, Data, Logs, and Cache in OS-Standard Directories (~706 tok)
- `security-audit-inventory.md` — Security Audit Inventory (~3325 tok)
- `security-audit-pass-a.md` — Security Audit Pass A — REST Authentication and Authorization (~3379 tok)
- `security-audit-pass-b.md` — Security Audit Pass B — MCP Server and Tool Bridge (~4996 tok)
- `security-audit-pass-c.md` — Security Audit Pass C — Design I/O and Job Files (~3513 tok)
- `security-audit-pass-d.md` — Security Audit Pass D — Analytics and Cloud Credentials (~3569 tok)
- `security-audit-pass-e.md` — Security Audit Pass E — Settings, Docker, and Installers (~3728 tok)
- `security-audit-pass-f.md` — Security Audit Pass F — CI and Supply Chain (~2872 tok)
- `security-audit-pass-g.md` — Security Audit Pass G — Java Deserialization and Board Snapshots (~2121 tok)
- `security-audit-pass-h.md` — Security Audit Pass H — Resource Exhaustion and Rate Limits (~4494 tok)
- `security-audit-plan.md` — Complete Codebase Security Audit Plan (~4516 tok)
- `security-audit-remediation-log.md` — Security Audit Remediation Log (Phase 4) (~1189 tok)
- `security-audit-risk-register.md` — Security Audit Risk Register (~11790 tok)
- `security-audit-scanner-summary.md` — Security Audit Scanner Summary (~1406 tok)
- `security-audit-threat-model.md` — Security Audit Threat Model (~4988 tok)

## docs/reference/

- `Cadence_SPECCTRA_Design_Language_Reference_v10_(2000-05).md` — SPECCTRA (~72304 tok)

## docs/research/

- `clearance_violations_reduction_plan.md` — Clearance Violations Reduction Plan (Tier B Multi-Layer Focus) (~5900 tok)
- `escape_congestion_research.md` — Escape Congestion Zones & Fanout Redesign — Research & Plan (~5341 tok)
- `kicad_ipc_api_research.md` — Purpose (~4527 tok)
- `optimizer_preflight_and_threshold_plan.md` — Freerouting Optimizer Pre-Flight Guards & Threshold Optimization Plan (~3683 tok)
- `optimizer_threshold_sweep_raw.json` (~7692 tok)
- `optimizer_threshold_sweep_results.csv` (~2054 tok)
- `optimizer_unification_clean_fixtures.txt` (~441 tok)
- `optimizer_unification_clean.csv` (~916 tok)
- `optimizer_unification_clean.json` (~237809 tok)
- `optimizer_unification_plan.md` — Freerouting Optimizer Unification Plan (~3223 tok)
- `optimizer_v2_weights_WL1000_WV2000_WB500.csv` (~13576 tok)
- `optimizer_v2_weights_WL3500_WV4000_WB1000.csv` (~13606 tok)
- `optimizer_v2_weights_WL500_WV8000_WB2000.csv` (~13489 tok)
- `planned_experiments.md` — Freerouting Planned Experiments (~3527 tok)
- `router_v2_weights_W1333_W2667_WC25_WD300.csv` (~16690 tok)
- `router_v2_weights_WU1000_WC25_WD300_F50.csv` (~15730 tok)
- `router_v2_weights_WU1000_WC25_WD300.csv` (~13918 tok)
- `schema_v5_current_v19_pair.md` — Schema v5 current vs v1.9 pair walkthrough (~298 tok)
- `scoring_revision_plan.md` — Freerouting Dedicated Scoring Architecture & Versioning Plan (~10165 tok)
- `self_improving_routing_loop_plan.md` — Self-Improving Routing Optimization Loop (~8486 tok)
- `v19_v2_replay.csv` (~4702 tok)

## docs/superpowers/plans/

- `2026-09-11-epicrouter-m0-rust-scaffold-and-baselines.md` — EpicRouter M0: Rust Workspace Scaffold + Java Golden Baselines — Implementation Plan (~18426 tok)
- `2026-09-12-epicrouter-m1a-geometry-kernel.md` — EpicRouter M1a: Geometry Kernel Port + Differential Corpus — Implementation Plan (~11001 tok)
- `2026-09-13-epicrouter-m1b-dsn-parser.md` — EpicRouter M1b: DSN Parser + SES Writer — Implementation Plan (~10990 tok)
- `2026-09-14-epicrouter-m2-board-and-index.md` — EpicRouter M2: Board Model + Spatial Index — Implementation Plan (~12402 tok)
- `2026-09-15-epicrouter-m3-detail-routing-core.md` — EpicRouter M3 — Detail Routing Core + epic-cli Implementation Plan (~6342 tok)
- `2026-09-22-epicrouter-m4-full-pipeline.md` — EpicRouter M4 — Full Pipeline (Fanout + Detail Optimizer + Batch Optimizer) Implementation Plan (~8064 tok)
- `2026-09-25-epicrouter-m5-performance.md` — EpicRouter M5 — Performance: Arena/SoA + Deterministic Parallelism Implementation Plan (~6333 tok)
- `2026-09-26-epicrouter-m6-routing-intelligence.md` — EpicRouter M6 — Routing Intelligence: Plane-Aware Routing + Congestion-Aware Global Stage + Push-and-Shove Implementation Plan (~10997 tok)
- `2026-09-28-epicrouter-m7-tuning.md` — EpicRouter M7 — Tuning: Length Constraints, Meander Insertion, Diff Pairs Implementation Plan (~13657 tok)
- `2026-09-28-epicrouter-m8-gloss.md` — EpicRouter M8 — Gloss: Aesthetics Measurement, Bus Hugging, 45° Flow, Via Placement, Teardrops Implementation Plan (~18379 tok)
- `2026-09-30-epicrouter-m10-sunset-and-release.md` — EpicRouter M10 — Sunset the Java Oracle, Release 2.0 — Implementation Plan (~17056 tok)
- `2026-09-30-epicrouter-m9-gui.md` — EpicRouter M9 — GUI: Session/Event Core, Render Lists, Desktop Shell Implementation Plan (~19183 tok)

## docs/superpowers/research/

- `2026-09-11-autorouter-state-of-the-art.md` — Autorouter State of the Art (September 2026) (~3691 tok)
- `2026-09-11-freerouting-gaps-and-user-pain-points.md` — Freerouting: Gaps, Complaints, and Feature Demand (September 2026) (~4225 tok)
- `2026-09-11-language-and-architecture-evidence.md` — Language & Architecture Evidence for the Rust Rewrite (~2360 tok)

## docs/superpowers/specs/

- `2026-09-11-epicrouter-rust-rewrite-design.md` — EpicRouter: Rust Rewrite Design (~28306 tok)

## examples/tutorial_board/

- `tutorial_board.dsn` — Declares signal (~190521 tok)
- `tutorial_board.kicad_prl` (~345 tok)
- `tutorial_board.kicad_pro` (~2165 tok)
- `tutorial_board.kicad_sch` (~36 tok)

## experiments/

- `experiments.jsonl` (~0 tok)
- `REPORT.md` — Autopilot experiment report (~44 tok)

## fixtures/

- `empty_board.dsn` — Declares signal (~118 tok)
- `Issue006-LPC18XX_43XX_SCH.dsn` (~71210 tok)
- `Issue015-StackOverflow.dsn` — Declares signal (~39912 tok)
- `Issue022-AutoRouter_interrupted.dsn` — Declares signal (~47592 tok)
- `Issue026-J2_reference.dsn` — Declares signal (~2114 tok)
- `Issue026-J2_reference.ses` (~3846 tok)
- `Issue027-zMRETestFixture.dsn` — Declares signal (~19016 tok)
- `Issue029-hw48na_invalid.rules` — Declares smd_to_turn_gap (~6075 tok)
- `Issue029-hw48na_valid.rules` — Declares default (~6093 tok)
- `Issue029-hw48na.dsn` — Declares signal (~39227 tok)
- `Issue029-hw48na.rules` — Declares smd_to_turn_gap (~6082 tok)
- `Issue034-Green14SegLED.dsn` — Declares signal (~8989 tok)
- `Issue035-ReadPlaceScope.dsn` — Declares signal (~15603 tok)
- `Issue039-bug-design.dsn` — Declares signal (~11821 tok)
- `Issue054-tairakb.dsn` — Declares signal (~11559 tok)
- `Issue066-Project_GP8B.dsn` — Declares signal (~29421 tok)
- `Issue070-Autorouter_FQ101_PCB_2022-05-13.dsn` — Declares default_smd (~32506 tok)
- `Issue093-interf_u.dsn` — Declares signal (~39667 tok)
- `Issue102-Mars-64-revE-rot00.dsn` — Declares signal (~13277 tok)
- `Issue103-Board-Routed.dsn` — Declares smd_to_turn_gap (~272987 tok)
- `Issue103-Board-Unrouted.dsn` — Declares signal (~11641 tok)
- `Issue107-freq_teiler_200kHz_kicad_bad.dsn` — Declares signal (~18704 tok)
- `Issue107-freq_teiler_200kHz_kicad_bad.rules` — Declares smd_to_turn_gap (~1002 tok)
- `Issue107-freq_teiler_200kHz_kicad.dsn` — Declares signal (~18482 tok)
- `Issue107-freq_teiler_200kHz_kicad.rules` — Declares smd_to_turn_gap (~1002 tok)
- `Issue110-Pajalnaja_stancija.dsn` — Declares signal (~7148 tok)
- `Issue110-RelayModule.dsn` — Declares signal (~26421 tok)
- `Issue110-testPCBSpecctraFile.dsn` — Declares signal (~16761 tok)
- `Issue110-testProjectFromFreeroutingBugTest.dsn` — Declares signal (~2101 tok)
- `Issue110-testProjectFromFreeroutingBugTest01.dsn` — Declares signal (~2199 tok)
- `Issue110-Паяльная станция.dsn` — Declares signal (~7148 tok)
- `Issue113-Protein.dsn` — Declares signal (~36524 tok)
- `Issue143-rpi_splitter_mod.dsn` — Declares signal (~1042 tok)
- `Issue143-rpi_splitter.dsn` — Declares signal (~1038 tok)
- `Issue145-smoothieboard.dsn` — Declares signal (~35209 tok)
- `Issue153-wavefolder.dsn` — Declares signal (~32949 tok)
- `Issue155-CH376_MCP795_Module.dsn` — Declares signal (~13428 tok)
- `Issue155-CH376_MCP795_Module.log` (~2835 tok)
- `Issue157-TeamAdapt-LinePCB.dsn` — Declares signal (~53617 tok)
- `Issue159-setonix_2hp-pcb.dsn` — Declares signal (~6464 tok)
- `Issue163-pic_programmer.dsn` — Declares signal (~23023 tok)
- `Issue178-KeebMaker_Sofle_Choc.dsn` — Declares signal (~140303 tok)
- `Issue179-Autorouter_PCB1_2023-3-24.dsn` — Declares default_smd (~3458 tok)
- `Issue190-processor.Z80.dsn` — Declares signal (~247367 tok)
- `Issue208-freerouting.dsn` — Declares signal (~11169 tok)
- `Issue209-split05.dsn` — Declares signal (~9776 tok)
- `Issue209-split10.dsn` — Declares signal (~9776 tok)
- `Issue214-freerouting.dsn` — Declares signal (~52090 tok)
- `Issue217-8088sbc.dsn` — Declares signal (~22616 tok)
- `Issue219-LogicBoard_smt.dsn` — Declares signal (~28808 tok)
- `Issue229-display-8-digit-hc595.dsn` — Declares signal (~9584 tok)
- `Issue230-CNH_Functional_Tester_1.dsn` — Declares signal (~43647 tok)
- `Issue269-caniot-tiny-arm.dsn` — Declares signal (~29160 tok)
- `Issue269-z10_module.dsn` — Declares signal (~45262 tok)
- `Issue270-non-ansi_bracket.dsn` — Declares signal (~1038 tok)
- `Issue289-Autorouter_PCB_FHT-8086_2024-03-08.dsn` — Declares default_smd (~42636 tok)
- `Issue289-Autorouter_PCB_FHT-VGA_2024-03-25.dsn` — Declares default_smd (~54514 tok)
- `Issue297-myboard.dsn` — Declares signal (~23992 tok)
- `Issue313-FastTest.dsn` — Declares default_smd (~11263 tok)
- `Issue313-FastTest.ses` — Declares protect (~7089 tok)
- `Issue326-Mars-64-revE.base64` (~18082 tok)
- `Issue326-Mars-64-revE.dsn` — Declares signal (~13277 tok)
- `Issue367-Charger.dsn` — Declares signal (~19898 tok)
- `Issue368-CorneyIslandWireless_input_design.dsn` — Declares signal (~840 tok)
- `Issue368-CorneyIslandWireless_input_design.json` (~2374 tok)
- `Issue368-CorneyIslandWireless_output_session.json` (~379 tok)
- `Issue368-CorneyIslandWireless_output_session.ses` (~520 tok)
- `Issue413-test.dsn` — Declares signal (~2314 tok)
- `Issue420-contribution-board.dsn` — Declares signal (~46718 tok)
- `Issue433-my-board.dsn` — Declares signal (~2182 tok)
- `Issue442-clearance_type_tests.rules` — Declares smd_to_turn_gap (~3873 tok)
- `Issue508-DAC2020_bm01.dsn` — Declares signal (~8118 tok)
- `Issue508-DAC2020_bm02.dsn` — Declares signal (~21410 tok)
- `Issue508-DAC2020_bm04.dsn` — Declares signal (~7178 tok)
- `Issue508-DAC2020_bm05.dsn` — Declares signal (~4460 tok)
- `Issue508-DAC2020_bm05.ses` (~8780 tok)
- `Issue508-DAC2020_bm06.dsn` — Declares signal (~6102 tok)
- `Issue508-DAC2020_bm07-current.ses` (~8942 tok)
- `Issue508-DAC2020_bm07-v190.ses` (~8288 tok)
- `Issue508-DAC2020_bm07.dsn` — Declares signal (~3949 tok)
- `Issue508-DAC2020_bm08-routed.ses` (~2519 tok)
- `Issue508-DAC2020_bm08.dsn` — Declares signal (~1467 tok)
- `Issue508-DAC2020_bm09.dsn` — Declares signal (~6654 tok)
- `Issue508-DAC2020_bm10.dsn` — Declares signal (~8356 tok)
- `Issue508-DAC2020_bm11.dsn` — Declares signal (~7000 tok)
- `Issue508-SMD-routing-issue-demo.dsn` — Declares signal (~623 tok)
- `Issue555-BBD_Mars-64-current.ses` (~14095 tok)
- `Issue555-BBD_Mars-64-v190.ses` (~7875 tok)
- `Issue555-BBD_Mars-64.dsn` — Declares signal (~14552 tok)
- `Issue555-CNH_Functional_Tester_1.dsn` — Declares signal (~43641 tok)
- `Issue558-dev-board.dsn` — Declares signal (~7251 tok)
- `Issue575-drc_BBD_Mars-64_6_track_1_hole_clearance_violations-freerouting_drc.json` (~14769 tok)
- `Issue575-drc_BBD_Mars-64_6_track_1_hole_clearance_violations-kicad_drc.json` (~28272 tok)
- `Issue575-drc_BBD_Mars-64_6_track_1_hole_clearance_violations.dsn` — Declares signal (~24739 tok)
- `Issue575-drc_dev-board_4_hole_clearance_violations-freerouting_drc.json` (~5807 tok)
- `Issue575-drc_dev-board_4_hole_clearance_violations-kicad_drc.json` (~2871 tok)
- `Issue575-drc_dev-board_4_hole_clearance_violations.dsn` — Declares signal (~16674 tok)
- `Issue575-drc_Natural_Tone_Preamp_7_unconnected_items-freerouting_drc.json` (~4285 tok)
- `Issue575-drc_Natural_Tone_Preamp_7_unconnected_items-kicad_drc.json` (~22458 tok)
- `Issue575-drc_Natural_Tone_Preamp_7_unconnected_items.dsn` — Declares signal (~65565 tok)
- `Issue593-BBD_Mars-64.dsn` — Declares signal (~14547 tok)
- `Issue593-BBD_Mars-64.rules` — Declares smd_to_turn_gap (~883 tok)
- `Issue593-BBD_Mars-64.ses` (~12824 tok)
- `Issue649-kicad_ecc83-pp_input_board_v1.dsn` — Declares signal (~9311 tok)
- `Issue649-kicad_ecc83-pp_input_board_v1.json` (~4006 tok)
- `Issue649-kicad_ecc83-pp_input_board_v2.dsn` — Declares signal (~10719 tok)
- `Issue649-kicad_ecc83-pp_input_board_v2.json` (~9052 tok)
- `Issue676-ch32v-tx118s.dsn` — Declares signal (~3847 tok)
- `Issue684-Autorouter_PCB1_2026-5-8.dsn` — Declares default_smd (~6093 tok)
- `Issue689-BBD_Mars-64.dsn` — Declares signal (~14547 tok)
- `Issue690-ecc83.dsn` — Declares signal (~10711 tok)
- `Issue690-ecc83.ses` — Declares protect (~3088 tok)
- `Issue690-kit-dev-coldfire-xilinx_5213.dsn` — Declares signal (~109915 tok)
- `Issue690-sonde_xilinx.dsn` — Declares signal (~13267 tok)
- `Issue721-Autorouter_CE2632_HarryMu_2026-6-15.dsn` — Declares default_smd (~12437 tok)
- `Issue723-CombineStackOverflow.dsn` — Declares signal (~74841 tok)
- `Issue730-DAC2020_bm11.dsn` — Declares signal (~7000 tok)
- `Issue732-CM5_MINIMA_3.dsn` — Declares signal (~39317 tok)
- `Issue732-DAC2020_bm10.dsn` — Declares signal (~8356 tok)
- `Issue732-RoyalBlue54L-Feather.dsn` — Declares signal (~27266 tok)
- `Issue733-kicad_complex_hierarchy_input_design.dsn` — Declares power (~14241 tok)
- `Issue733-kicad_complex_hierarchy_input_design.json` (~25665 tok)
- `Issue733-kicad_complex_hierarchy_output_session.json` (~19552 tok)
- `Issue733-kicad_complex_hierarchy_output_session.ses` (~9150 tok)
- `Issue733-kicad_interf_u_input_design.dsn` — Declares signal (~36034 tok)
- `Issue733-kicad_interf_u_input_design.json` (~109708 tok)
- `Issue742-tastexx-pcb.dsn` — Declares signal (~9396 tok)
- `Issue742-tastexx-pcb.ses` (~2039 tok)
- `Issue753-CPU-85_r104.dsn` — Declares signal (~40379 tok)
- `Issue754-avionics_hub.dsn` — Declares signal (~44332 tok)
- `Issue756-minimal-hang.dsn` — Declares signal (~231 tok)
- `Issue756-minimal-ok.dsn` — Declares signal (~209 tok)
- `Issue756-tomu-fpga.dsn` — Declares signal (~33728 tok)
- `Issue756-tomu-fpga11.dsn` — Declares signal (~34878 tok)
- `Issue756-tomu-fpga7.dsn` — Declares signal (~92446 tok)
- `Issue756-tomu-fpga8.dsn` — Declares signal (~92204 tok)
- `Issue756-tomu-fpga9.dsn` — Declares signal (~93791 tok)
- `Issue757-minimal-soe-ok.dsn` — Declares signal (~221 tok)
- `Issue757-minimal-soe.dsn` — Declares signal (~245 tok)
- `scoring-empty-net.dsn` — Declares signal (~279 tok)
- `scoring-mixed-layer.dsn` — Declares signal (~324 tok)
- `scoring-perfect-two-pin.dsn` — Declares signal (~197 tok)
- `scoring-zero-length.dsn` — Declares signal (~286 tok)

## fixtures/Issue069-TestSensel/

- `fp-lib-table` — Declares KiCad (~26 tok)
- `gui_defaults.par` (~886 tok)
- `TestSensel-cache.lib` (~227 tok)
- `TestSensel-KiCad6.dsn` — Declares signal (~1383 tok)
- `TestSensel.dsn` — Declares signal (~1727 tok)
- `TestSensel.kicad_pcb` — Declares Default (~3781 tok)
- `TestSensel.kicad_pcb-bak` — Declares Default (~3781 tok)
- `TestSensel.kicad_prl` (~321 tok)
- `TestSensel.kicad_pro` (~1990 tok)
- `TestSensel.pro` (~957 tok)
- `TestSensel.sch` (~648 tok)
- `TestSensel.sch-bak` (~648 tok)

## fixtures/Issue069-TestSensel/MyLibs/

- `PCB_Fork_10mils_5mmx5mm.kicad_mod` (~412 tok)
- `PCB_Fork_10mils_5mmx5mm.lib` (~353 tok)

## fixtures/Issue180-Test/

- `fp-lib-table` (~30 tok)
- `report.txt` (~267 tok)
- `Test.kicad_pcb` — Declares solid (~193572 tok)
- `Test.kicad_prl` (~318 tok)
- `Test.kicad_pro` (~3308 tok)
- `Test.kicad_sch` — Declares default (~36521 tok)

## fixtures/Issue180-Test/Gerbers/

- `Test-B_Cu.gbr` (~22021 tok)
- `Test-B_Mask.gbr` (~13831 tok)
- `Test-drl.rpt` (~222 tok)
- `Test-Edge_Cuts.gbr` (~11568 tok)
- `Test-F_Courtyard.gbr` (~12771 tok)
- `Test-F_Cu.gbr` (~22472 tok)
- `Test-F_Mask.gbr` (~13831 tok)
- `Test-F_Silkscreen.gbr` (~18477 tok)
- `Test-In1_Cu.gbr` (~56440 tok)
- `Test-In2_Cu.gbr` (~56440 tok)
- `Test-job.gbrjob` (~906 tok)
- `Test-NPTH.drl` (~104 tok)
- `Test-PTH.drl` (~3166 tok)

## fixtures/Issue180-Test/Library.pretty/

- `D_DO-41_SOD81_P10.16mm_Horizontal.kicad_mod` (~1169 tok)
- `R_Axial_DIN0309_L9.0mm_D3.2mm_P12.70mm_Horizontal.kicad_mod` (~847 tok)

## fixtures/Issue184-motorizedopener/

- `fp-info-cache` (~1 tok)
- `motorizedopener.kicad_pcb` (~60039 tok)
- `motorizedopener.kicad_prl` (~307 tok)
- `motorizedopener.kicad_pro` (~2484 tok)
- `motorizedopener.kicad_sch` — Declares default (~50365 tok)
- `run_freerouting.bat` (~19 tok)

## fixtures/Issue191-processor.Z80/

- `buffers.kicad_sch` — Declares default (~28866 tok)
- `bus.kicad_sch` — Declares default (~29376 tok)
- `CPU-DATA-DIR-GAL.csv` (~3992 tok)
- `CPU-DATA-DIR-GAL.dig` (~3138 tok)
- `CPU-DATA-DIR-GAL.jed` (~361 tok)
- `CPU-DATA-DIR-GAL.tru` (~811 tok)
- `CPU-MAPPER.csv` (~9594 tok)
- `CPU-MAPPER.dig` (~3856 tok)
- `CPU-MAPPER.jed` (~480 tok)
- `CPU-MAPPER.tru` (~1170 tok)
- `CPU-WS-GAL.csv` (~1456 tok)
- `CPU-WS-GAL.dig` (~2559 tok)
- `CPU-WS-GAL.jed` (~137 tok)
- `CPU-WS-GAL.tru` (~287 tok)
- `DMA-CS-GAL.csv` (~10699 tok)
- `DMA-CS-GAL.dig` (~5302 tok)
- `DMA-CS-GAL.jed` (~650 tok)
- `DMA-CS-GAL.tru` (~1516 tok)
- `DMA-RDY-GAL.csv` (~9601 tok)
- `DMA-RDY-GAL.dig` (~2779 tok)
- `DMA-RDY-GAL.jed` (~342 tok)
- `DMA-RDY-GAL.tru` (~1107 tok)
- `DMA.kicad_sch` — Declares default (~53399 tok)
- `fp-lib-table` — Declares KiCad (~35 tok)
- `fpanel.kicad_sch` — Declares default (~19146 tok)
- `GAL22V10.dcm` (~13 tok)
- `gal22v10.kicad_sym` — Declares default (~1356 tok)
- `GALS.kicad_sch` — Declares default (~150436 tok)
- `IM2.kicad_sch` — Declares default (~12572 tok)
- `mapper.kicad_sch` — Declares default (~86267 tok)
- `power.kicad_sch` — Declares default (~28217 tok)
- `processor.rules` — Declares smd_to_turn_gap (~2943 tok)
- `processor.ses` (~104319 tok)
- `processor.Z80.csv` (~2411 tok)
- `processor.Z80.kicad_prl` (~309 tok)
- `processor.Z80.kicad_pro` (~3030 tok)
- `processor.Z80.kicad_sch` — Declares default (~27954 tok)
- `processor.Z80.xml` (~73921 tok)
- `readme.txt` (~50 tok)
- `sym-lib-table` (~121 tok)
- `TL16C550CFN.kicad_sym` — Declares default (~2312 tok)
- `waitstate.kicad_sch` — Declares default (~12388 tok)
- `Z80CPU.kicad_sch` — Declares default (~24355 tok)
- `Zilog_Z80_Peripherals.kicad_sym` — Declares default (~92266 tok)
- `Zilog_z80.kicad_sym` — Declares default (~24744 tok)

## fixtures/Issue191-processor.Z80/CUPL_CPU-DATA-DIR-GAL/

- `CUPL.PLD` (~202 tok)

## fixtures/Issue191-processor.Z80/CUPL_CPU-MAPPER/

- `CUPL.PLD` (~219 tok)

## fixtures/Issue191-processor.Z80/CUPL_CPU-WS-GAL/

- `CPU-WS-GAL.dig.jed` (~224 tok)
- `CUPL.abs` (~294 tok)
- `CUPL.PLD` (~161 tok)
- `CUPL.sim` (~122 tok)
- `CUPL.wo` — WAVEFORM (~3 tok)
- `tmpcsim.im` (~238 tok)

## fixtures/Issue191-processor.Z80/CUPL_DMA-CS-GAL/

- `CUPL.PLD` (~282 tok)

## fixtures/Issue191-processor.Z80/CUPL_DMA-RDY-GAL/

- `CUPL.PLD` (~204 tok)

## fixtures/Issue191-processor.Z80/data-gear_gal/

- `DATAGEAR.EQN` (~118 tok)
- `DATAGEAR.JED` (~737 tok)
- `DATAGEAR.LOG` (~692 tok)
- `Velesoft DMA circuit.url` (~34 tok)
- `velesoft DMA.txt` — Declares for (~206 tok)

## fixtures/Issue199-StackOverflow/

- `Signale_Vor+Block.dsn` — Declares signal (~5345 tok)

## fixtures/Issue230-CNH_Functional_Tester/

- `CNH_Functional_Tester_1.dsn` — Declares signal (~43647 tok)
- `CNH_Functional_Tester_1.kicad_dru` (~0 tok)
- `CNH_Functional_Tester_1.kicad_prl` (~350 tok)
- `CNH_Functional_Tester_1.kicad_pro` (~5181 tok)
- `CNH_Functional_Tester_1.kicad_sch` — Declares default (~89910 tok)
- `fp-info-cache` (~1 tok)
- `fp-lib-table` (~34 tok)
- `Perf_Breakout.kicad_sch` — Declares default (~42955 tok)

## fixtures/Issue230-CNH_Functional_Tester/Gerbers/

- `CNH_Functional_Tester_1-B_Cu.gbr` (~25405 tok)
- `CNH_Functional_Tester_1-B_Mask.gbr` (~7304 tok)
- `CNH_Functional_Tester_1-B_Silkscreen.gbr` (~37687 tok)
- `CNH_Functional_Tester_1-Edge_Cuts.gbr` (~185 tok)
- `CNH_Functional_Tester_1-F_Cu.gbr` (~24917 tok)
- `CNH_Functional_Tester_1-F_Mask.gbr` (~7606 tok)
- `CNH_Functional_Tester_1-F_Silkscreen.gbr` (~75128 tok)
- `CNH_Functional_Tester_1-In1_Cu.gbr` (~201107 tok)
- `CNH_Functional_Tester_1-In2_Cu.gbr` (~200486 tok)
- `CNH_Functional_Tester_1-job.gbrjob` (~975 tok)
- `CNH_Functional_Tester_1.drl` (~3516 tok)
- `report.txt` (~417 tok)

## fixtures/Issue230-CNH_Functional_Tester/Library.pretty/

- `R78_9V_1A_SIP_TH.kicad_mod` — Declares solid (~896 tok)

## fixtures/Issue269-NoViasOnPowerPlanes/

- `Issue269-NoViasOnPowerPlanes.dsn` — Declares signal (~5168 tok)
- `Issue269-NoViasOnPowerPlanes.kicad_pcb` — Declares solid (~11292 tok)
- `Issue269-NoViasOnPowerPlanes.kicad_prl` (~352 tok)
- `Issue269-NoViasOnPowerPlanes.kicad_pro` (~4227 tok)

## fixtures/Issue269-min_fr_test/

- `min_fr_test_no_quotes.dsn` — Declares signal (~676 tok)
- `min_fr_test.dsn` — Declares signal (~677 tok)
- `min_fr_test.kicad_pcb` — Declares solid (~4700 tok)
- `min_fr_test.kicad_prl` (~339 tok)
- `min_fr_test.kicad_pro` (~3659 tok)
- `min_fr_test.kicad_sch` — Declares default (~2814 tok)

## logs/M10-T4/

- `quality-review-prompt-t4.md` — M10-T4 QUALITY REVIEW — charter (the Java sunset; five commits, config + one Rust repair) (~1200 tok)
- `quality-review-t4-1.md` — M10-T4 QUALITY REVIEW (quality-review-t4-1) — the Java sunset (~4302 tok)
- `report-t4.md` — M10-T4 IMPLEMENTER REPORT — the Java sunset (~8315 tok)
- `spec-review-prompt-t4.md` — M10-T4 SPEC REVIEW — charter (the Java sunset; FIVE commits landed) (~1578 tok)
- `spec-review-t4-1.md` — M10-T4 SPEC REVIEW (spec-review-t4-1) — the Java sunset (~4288 tok)

## logs/M10-T5/

- `dispatch-prompt-t5.md` — M10-T5 DISPATCH — version 2.0.0 + the Linux release artifact (+ the AM4 intake hygiene; two commits  (~2263 tok)
- `quality-review-prompt-t5.md` — M10-T5 CODE-QUALITY REVIEW — DISPATCH PROMPT (fresh reviewer, zero prior context) (~2178 tok)
- `report-t5.md` — M10-T5 IMPLEMENTER REPORT — ROUND 1: BLOCKED (probe-stage stop; zero edits, zero commits) (~3817 tok)
- `spec-review-prompt-t5.md` — M10-T5 SPEC-COMPLIANCE REVIEW — DISPATCH PROMPT (fresh reviewer, zero prior context) (~3173 tok)

## logs/M10-T6/

- `dispatch-prompt-t6.md` — M10-T6 DISPATCH — the migration guide + release docs (fresh implementer) (~2212 tok)

## logs/M10-T7/

- `quality-review-prompt-t7.md` — M10-T7 QUALITY REVIEW (fresh eyes; the coordinator's charter) (~1414 tok)
- `quality-review-t7-1.md` — M10-T7 QUALITY REVIEW — FINAL REPORT (materialized verbatim by the coordinator) (~3154 tok)
- `report-t7.md` — M10-T7 — the exit note + the close-out (the M9-T8 pattern) (~4932 tok)
- `spec-review-prompt-t7.md` — M10-T7 SPEC REVIEW (fresh eyes; the coordinator's charter) (~1765 tok)
- `spec-review-t7-1.md` — M10-T7 SPEC COMPLIANCE REVIEW — FINAL REPORT (materialized verbatim by the coordinator) (~2564 tok)

## logs/M10-T7/evidence/

- `30-spec-census.log` — 30-spec-census.log — M10-T7 SPEC-COMPLIANCE REVIEWER (fresh eyes), primary re-derivation (~211 tok)
- `40-quality-mutant-probe.log` (~561 tok)

## logs/M10-T8/

- `dispatch-prompt-t8.md` — M10-T8 DISPATCH — the terminal milestone adjudication (fresh adjudicator) (~2002 tok)
- `report-t8.md` — M10-T8 — the terminal milestone adjudication (materialized verbatim by the coordinator) (~3558 tok)

## logs/java-oracle-m11/

- `run-adjudication.sh` — M11 adjudication: upstream Java pre-#931 (aa909a345^) vs post-#931 (aa909a345) (~357 tok)
- `run-interfu-default.sh` — Probe B: Java pre-#931 vs post-#931, DEFAULT routing on interf_u. (~318 tok)

## logs/readiness-2026-10-01/

- `152a-commit-msg.txt` (~1131 tok)
- `933-survivors-intake.md` — #933 survivors — upstream intake + port map (2026-10-03) (~1747 tok)
- `933s1-commit-msg.txt` (~667 tok)
- `933s2-commit-msg.txt` (~898 tok)
- `933s3-commit-msg.txt` (~893 tok)
- `audit-code-quality.md` — EpicRouter 2.0.0 — code-quality / product-readiness audit (2026-10-01) (~4008 tok)
- `charter.md` — Post-2.0 readiness campaign — 2026-10-01 (~465 tok)
- `dispatch-prompt-fixround.md` — Readiness fix round — implementer dispatch (2026-10-01) (~2379 tok)
- `e2e-REVIEW.md` — E2E review — regular 600s sweep (2026-10-01, final release bin, post-454334bdc tree) (~1340 tok)
- `export-msg-405709959.txt` — Declares placements (~649 tok)
- `export-msg-918cb6648.txt` (~179 tok)
- `export-msg-96492284c.txt` — Declares power (~309 tok)
- `export-msg-96688fa3f.txt` (~296 tok)
- `export-msg-ca0e785cc.txt` (~429 tok)
- `export-msg-d93cb388a.txt` — Declares synthesis (~229 tok)
- `export-msg-f4.txt` — Declares width (~494 tok)
- `export-msg-m11t2.txt` (~330 tok)
- `export-msg-m11t2rot.txt` (~160 tok)
- `export-msg-m11t3.txt` (~452 tok)
- `export-msg-m11t4.txt` (~573 tok)
- `export-msg-m11t6.txt` (~308 tok)
- `export-msg-m11t6fix.txt` (~217 tok)
- `export-msg-m11t7.txt` (~325 tok)
- `export-msg-m11t9d.txt` (~373 tok)
- `export-msg-m11t9i.txt` (~317 tok)
- `export-msg-p3.txt` — Declares promotion (~488 tok)
- `export-msg-p4.txt` (~243 tok)
- `export-msg-p5.txt` (~115 tok)
- `export-msg-p6.txt` (~191 tok)
- `export-msg-seamfix.txt` (~286 tok)
- `f1b-commit-msg.txt` — Declares placements (~747 tok)
- `f2a-commit-msg.txt` — Declares synthesis (~450 tok)
- `f2b-commit-msg.txt` — Declares cloned (~771 tok)
- `f3a-commit-msg.txt` — Declares 0 (~569 tok)
- `f3b-commit-msg.txt` (~599 tok)
- `f4a-commit-msg.txt` (~556 tok)
- `f4b-commit-msg.txt` — Declares width (~530 tok)
- `f4c-commit-msg.txt` — Declares width (~739 tok)
- `fix-round-charter.md` — Readiness fix round — 2.0.1 hardening charter (2026-10-01) (~787 tok)
- `fixa-commit-msg.txt` (~344 tok)
- `M11-931.md` — M11 — the #931 Tier-B completion campaign (upstream `aa909a345`) (~5292 tok)
- `m11cd-commit-msg.txt` (~1009 tok)
- `m11t1-commit-msg.txt` (~615 tok)
- `m11t2-commit-msg.txt` (~616 tok)
- `m11t3-commit-msg.txt` (~1498 tok)
- `m11t4-commit-msg.txt` (~1391 tok)
- `m11t4fix-commit-msg.txt` (~1204 tok)
- `m11t6-commit-msg.txt` (~756 tok)
- `m11t6-fix-commit-msg.txt` (~431 tok)
- `m11t7-commit-msg.txt` (~472 tok)
- `m11t9d-commit-msg.txt` (~1782 tok)
- `m11t9i-commit-msg.txt` (~2236 tok)
- `p1-commit-msg.txt` — Declares power (~595 tok)
- `p2-commit-msg.txt` (~988 tok)
- `p3-commit-msg.txt` — Declares cell (~1737 tok)
- `p3-recon.md` — P3 recon — DRC 1µm shortfall tolerance (#925a, upstream 14b28b6ff) (~1744 tok)
- `p3extract.py` — grab (~152 tok)
- `p4-commit-msg.txt` — Declares as (~846 tok)
- `p5-commit-msg.txt` (~618 tok)
- `p6-commit-msg.txt` (~1176 tok)
- `probe_sigint.new.sh` — Live SIGINT probe (readiness-fix M5 coordinator evidence): route a slow (~1037 tok)
- `probe_sigint.sh` — Live SIGINT probe (readiness-fix M5 coordinator evidence): route a slow (~576 tok)
- `report-fixround.md` — Readiness fix-round report (2026-10-01) (~2421 tok)
- `report-fixround2.md` — Readiness fix-round-2 report (2026-10-01) (~1694 tok)
- `review-fixround.md` — Fresh-eyes review — 2.0.0 hardening diff (2026-10-01) (~2186 tok)
- `rotate_gv_m11t4fix.sh` — M11-T4 FIX-ROUND gv-iu golden rotation (the one drifted face). (~491 tok)
- `rotate_gv_m11t4t3.sh` — M11-T4+T3 gv golden rotations (the five drifted faces; gv-g1 is (~581 tok)
- `rotate_gv_t3only.sh` — M11-T3-ONLY gv golden rotations (the two drifted faces; the other (~609 tok)
- `run_batch_ext.sh` — Extended-cap E2E runner: same as run_batch.sh but 2400s wall and e2e-ext/ output. (~204 tok)
- `run_batch.sh` — Readiness E2E runner: routes unseen real boards through the product CLI face. (~246 tok)
- `run_gates_152a.sh` — #152 COMMIT A landing battery (2026-10-03) — heuristic relaxation (~1459 tok)
- `run_gates_933s1.sh` — #933 SURVIVOR-1 landing battery (2026-10-03) — the fanout (~1189 tok)
- `run_gates_933s2.sh` — #933 SURVIVOR-2 landing battery (2026-10-03) — the last-mile (~1212 tok)
- `run_gates_m11cd.sh` — M11-T1+T5 (upstream #931 clusters C+D: the stagnant-net ripup AND (~783 tok)
- `run_gates_m11t1.sh` — M11-T1 (upstream #931 cluster C: failure-triggered stagnant-net ripup) (~600 tok)
- `run_gates_m11t2t6.sh` — M11-T2+T6 battery — the outline edge-pin-net exemption (T6, 023fb1c8b) (~1218 tok)
- `run_gates_m11t4fix_final.sh` — M11-T4 fix landing battery — FINAL LEG: the corpus-family compares, (~371 tok)
- `run_gates_m11t4fix_r2.sh` — M11-T4 fix landing battery — ROUND 2 (the 30-min background limit (~469 tok)
- `run_gates_m11t4fix.sh` — M11-T4 FIX-ROUND landing battery (2026-10-03) — the C1 floor (~985 tok)
- `run_gates_m11t4t3_r2.sh` — M11-T4+T3 battery ROUND 2 — after the fix cycle: fmt applied, the (~1110 tok)
- `run_gates_m11t4t3_r3.sh` — M11-T4+T3 battery ROUND 3 — after the r2 fix cycle: (~1109 tok)
- `run_gates_m11t4t3_r4.sh` — M11-T4+T3 battery, round 4 — the post-rotation confirmation battery. (~1007 tok)
- `run_gates_m11t4t3_r5.sh` — M11-T4+T3 battery, round 5 — the LANDING battery at the final tree (~956 tok)
- `run_gates_m11t4t3.sh` — M11-T4+T3 battery — #931 cluster F (corner-touch inserts, locator_45 (~1312 tok)
- `run_gates_m11t9d.sh` — M11-T9d battery — the DrillPage containsNet conjunct (upstream #931, (~1169 tok)
- `run_gates_m11t9i.sh` — M11-T9i (upstream aa909a345 FoundConnectionInserter.java:212 neckdown (~731 tok)
- `run_gates_m11t9i2.sh` — M11-T9i battery, ROUND 2 — on the COMBINED tree (T9i working set + (~714 tok)
- `run_gates_p3.sh` — P3 (#925a clearance-tolerance) gate battery: fmt + clippy both faces + (~582 tok)
- `run_gates_p4.sh` — P4 (#925b same-component Pin-Pin exemptions) gate battery: fmt + clippy (~590 tok)
- `run_gates_p5.sh` — P5 (upstream 8fb76a61b convex-split stop guard) gate battery: fmt + (~586 tok)
- `run_gates_rerun.sh` — Gate re-run after the tripwire-retirement fix: census + fresh release (~238 tok)
- `run_gates_seamfix.sh` — M11 seam-fix commit 1 (the pass-count reading seam: manifest (~1439 tok)
- `run_gates.sh` — R2 gate battery — current re-proof of every standing face. Serial; every command (~371 tok)
- `run_golden_m11t9i2.sh` — M11-T9i golden-verify sweep — the face BOTH T9i batteries missed (the (~719 tok)
- `run_t3only_landing.sh` — M11-T3-ONLY landing battery — 2026-10-03. (~714 tok)
- `run_t3only_v2.sh` — M11-T3-ONLY landing battery v2 — 2026-10-03. (~1021 tok)
- `seamfix-commit-msg.txt` (~938 tok)
- `TASKS.md` — Campaign task list (living) — 2026-10-01 (~3857 tok)
- `upstream-intake.md` — Upstream Freerouting intake — commits since baseline e7f9bdf1a (2026-10-01) (~1734 tok)
- `verdict.md` — EpicRouter 2.0.0 readiness verdict — 2026-10-01 (~1598 tok)

## logs/readiness-2026-10-01/f2-smoke/

- `width-craft.dsn` — Declares signal (~336 tok)

## logs/readiness-2026-10-01/gates-overflow/

- `ANALYSIS.md` — Tier A wall delta: 242.5s → 318.4s — CLOSED (attributed, not a regression) (~1185 tok)

## logs/readiness-2026-10-01/gates/m11t4t3/

- `e1-manifest.jsonl` (~27 tok)
- `e1-oracle-probe.sh` — M11-T4+T3 e1_ripup events adjudication: which Java tree does the Rust (~273 tok)

## rust/

- `Cargo.toml` — Rust package manifest (~286 tok)
- `README.md` — Project documentation (~6421 tok)

## rust/crates/epic-board/src/

- `aesthetics.rs` — The M8 aesthetics measurer (design :86 — the four metrics): ONE (~11338 tok)
- `board.rs` — The live board: the item arena, the id generator, and the (~34990 tok)
- `components.rs` — Components, the board-side library mirror, and PIN PLACEMENT (~24645 tok)
- `lib.rs` — Board model: items, ids, undo, rules, layers, components (M2). The (~753 tok)
- `plane_obstacle.rs` — Upstream #152 (PR #889): the `planeAsObstacle` board face. (~1901 tok)
- `routing_ledger.rs` — Java `NetRoutingLedger` (#933, upstream 339e8bb50 — "Speed up (~3120 tok)
- `rules_surf.rs` — The rules READ surface (M2 Task 3): the clearance matrix, the net (~18076 tok)
- `shape_trace_entries.rs` — The shove substrate: which obstacles sit in a shape, which trace (~29177 tok)
- `trace_shover.rs` — The shove drivers: can a trace segment be forced into a live board, (~26962 tok)
- `undo_facade.rs` — The board-level undo/redo/snapshot FACADE — Java (~6215 tok)

## rust/crates/epic-board/src/items/

- `outline.rs` — Board outline — keepout derivations (M2 Task 4). (~13353 tok)

## rust/crates/epic-board/tests/

- `probe_degenerate.rs` — SCRATCH PROBE (delete after use) — locate corpus fixtures whose pin (~677 tok)
- `probe_interfu.rs` — SCRATCH PROBE (delete after use) — interf_u attribution facts: does (~427 tok)
- `probe_interfu2.rs` — SCRATCH PROBE 2 (delete after use) — WHICH item kinds poke out of (~390 tok)

## rust/crates/epic-cli/

- `Cargo.toml` — Rust package manifest (~219 tok)

## rust/crates/epic-cli/examples/

- `p3rows.rs` — Scratch probe: the drc-main load walk rows at tolerance 1.0 vs 0.0. (~631 tok)

## rust/crates/epic-cli/src/

- `main.rs` — `epic-cli` — the EpicRouter headless command-line binary (M3-T13). (~2346 tok)
- `route.rs` — The `route` flow (M3-T13): DSN read -> board build + trace (~57079 tok)

## rust/crates/epic-cli/tests/

- `cli_surface.rs` — The readiness-fix M1/M2 bin-level pins: the built `epic-cli` debug (~2563 tok)
- `version_pin.rs` — The M10-T5 `--version` pin: the built `epic-cli` bin, spawned with (~446 tok)

## rust/crates/epic-drc/examples/

- `p3probe.rs` — Scratch probe for the #925a boundary worlds — prints communication, (~364 tok)

## rust/crates/epic-drc/src/

- `clearance.rs` — The clearance-violation walk — the port of Java (~16064 tok)
- `lib.rs` — The two counts every M3 quality gate consumes — incomplete (~357 tok)
- `routing_ledger.rs` — The #933 survivor-3 recount driver (upstream 339e8bb50, Java (~4835 tok)
- `test_util.rs` — Crafted DSN boards + parse/lookup helpers for the epic-drc pin (~2802 tok)

## rust/crates/epic-dsn/src/

- `reader.rs` — Port of `io.specctra.DsnReader` (`DsnReader.java`) — the read (~11995 tok)
- `ses_board.rs` — The parse-derivable mini board model (plan decision D9): (~26601 tok)

## rust/crates/epic-dsn/src/scope/

- `structure.rs` — The `(structure ...)` scope reader: the port of (~33376 tok)

## rust/crates/epic-engine/src/

- `current_width.rs` — The F2 current-driven trace-width core: the IPC-2221B closed-form (~7142 tok)
- `drc_tolerance.rs` — The #925a DRC clearance-tolerance apply face (upstream (~848 tok)
- `interview.rs` — F4: the pre-route interview — constraint inference from the board (~4966 tok)
- `lib.rs` — The headless application core: jobs, the layered settings (~759 tok)
- `pin_assign.rs` — The F1 pin auto-assignment core: a self-contained min-cost (~7713 tok)
- `plane_nets.rs` — Upstream #152 (PR #889, d9694ab82): the `planeNets` override. (~1644 tok)
- `pour.rs` — F3 (Rust-only, no Java counterpart): the ground-pour ask. (~5746 tok)
- `session.rs` — The headless application session — the M9-T2 `Session` (the Java (~16231 tok)
- `settings.rs` — The T13 settings subset resolver (Java `SettingsMerger` + `CliSettings` (~65091 tok)

## rust/crates/epic-engine/tests/

- `dump_bm07_kinds.rs` — TEMPORARY re-capture helper (M11-T4 fix round, 2026-10-03) — NOT (~880 tok)
- `events_stream.rs` — M9-T3: the event-stream pins (the dispatch's charter for (~5610 tok)
- `overlays.rs` — M9-T5: the overlay DATA faces' engine pins: (~5156 tok)
- `session_workflow.rs` — M9-T2: the `Session` workflow pins (the dispatch's charter for (~6336 tok)

## rust/crates/epic-engine/tests/goldens/

- `bm07.event-kinds.txt` — bm07 (scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm07.dsn) through (~4123 tok)

## rust/crates/epic-geometry/src/

- `java_random.rs` — Port of the JDK `java.util.Random` 48-bit linear congruential generator. (~2451 tok)

## rust/crates/epic-gui/src/

- `shell.rs` — The M9-T6 desktop-shell PROTOCOL module — deliberately UNGATED (~13443 tok)

## rust/crates/epic-gui/src/bin/

- `epic-gui.rs` — The epic-gui bin: the desktop shell's entry face. DEFAULT-OFF: (~412 tok)

## rust/crates/epic-gui/src/desktop/

- `canvas.rs` — The M9-T6 canvas (desktop-gated): the egui painter over the pure (~1811 tok)
- `mod.rs` — The M9-T6 desktop shell (desktop-gated): the THIN eframe host — (~12196 tok)
- `worker.rs` — The M9-T6 desktop worker thread (desktop-gated; its PROTOCOL types (~2751 tok)

## rust/crates/epic-gui/tests/

- `render_goldens.rs` — M9-T4: the render-golden pins (the dispatch charter for (~16171 tok)
- `version_pin.rs` — The M10-T6 fix-round Q1 pin: the built `epic-gui` bin, spawned with (~704 tok)

## rust/crates/epic-router/

- `SEAM.md` — SEAM — the Java surface beyond `autoroute/` (M3-T3 audit) (~93053 tok)

## rust/crates/epic-router/src/

- `control.rs` — Java `autoroute/maze/AutorouteControl.java` — the per-net cost table (~14896 tok)
- `engine.rs` — Java `autoroute/maze/AutorouteEngine.java` — the per-net routing (~52076 tok)

## rust/crates/epic-router/src/drill/

- `drill_page.rs` — Java `autoroute/drill/DrillPage.java` (194 lines) — one grid cell's (~3622 tok)
- `pins.rs` — M3-T5 drill-subsystem pins — literal replay of the DrillSpike (~28632 tok)
- `tests.rs` — Synthetic-world unit pins for the drill subsystem (no capture rows (~22972 tok)

## rust/crates/epic-router/src/expansion/

- `neighbours_forty_five.rs` — Java `Sorted45DegreeRoomNeighbours` (~14410 tok)

## rust/crates/epic-router/src/global/

- `history.rs` — The PathFinder negotiated-congestion scheduler (M6-T8) — per-resource (~4047 tok)
- `map.rs` — The coarse-grid congestion map (M6-T7) — an occupancy/overflow (~6193 tok)
- `pattern.rs` — The pattern router (M6-T7) — L/Z 1-2-bend routes inside guides for (~2294 tok)
- `tests.rs` — The M6-T7 pin bank (charter: pins in `global/tests.rs`). (~8761 tok)

## rust/crates/epic-router/src/maze/

- `pins.rs` — The T6 jar-capture pins — the maze search CORE replayed against the (~44923 tok)
- `search_engine.rs` — Java `autoroute/maze/MazeSearchEngine.java` — the maze search CORE: (~19522 tok)

## rust/crates/epic-router/src/path/

- `inserter.rs` — Java `autoroute/path/FoundConnectionInserter.java` — inserts the (~23784 tok)
- `locator_45.rs` — Port of Java `app.freerouting.autoroute.path.FoundConnectionLocator45Degree` (~4703 tok)
- `pins.rs` — Literal capture pins for the T9 found-connection locator (module (~16679 tok)

## rust/crates/epic-router/src/pipeline/

- `batch.rs` — Java `autoroute/pipeline/BatchAutorouter.java` + (~28411 tok)
- `board_statistics.rs` — Java `core/scoring/BoardStatistics.java` — the board counting walk (~22856 tok)
- `connection_router.rs` — Java `autoroute/pipeline/AutorouteConnectionRouter.java` — the (~12481 tok)
- `event_sink.rs` — The driver's log-only output seam: Java scatters `FRLogger.info` / (~2456 tok)
- `fanout.rs` — Java `autoroute/pipeline/BatchFanout.java` — the fanout stage: the (~30123 tok)
- `full.rs` — The full-pipeline assembly (M4-T10): the port of Java (~14569 tok)
- `java_random.rs` — `java.util.Random` + `java.util.Collections.shuffle` — the EXACT (~1533 tok)
- `last_mile.rs` — Java `LastMileBlockerRipup` (#933, upstream 339e8bb50 — "Speed up (~3805 tok)
- `mod.rs` — The batch-autoroute pipeline (M3-T12): the multi-pass driver over (~192 tok)
- `optimizer.rs` — Java `autoroute/pipeline/BatchOptimizer.java` — the rip-and-reroute (~46334 tok)
- `pairs.rs` — M7-T6: the differential-PAIR face — the pair DECLARATION resolution, (~13881 tok)
- `pass_runner.rs` — Java `autoroute/pipeline/AutoroutePassRunner.java` — the (~20918 tok)

## rust/harness/

- `run-gate.sh` — M6-T7 (rider: exit-printing by construction) — run a gate/instrument (~250 tok)

## rust/harness/fixtures/global-spike/

- `g5_mixedlayer.dsn` — Declares signal (~377 tok)

## rust/harness/fixtures/p3/

- `p3-pinpair.dsn` — Declares signal (~164 tok)

## rust/harness/fixtures/p4/

- `p4-pins.dsn` — Declares signal (~332 tok)

## rust/harness/fixtures/t6/

- `t6-degenerate-pad.dsn` — Declares signal (~195 tok)
- `t6-edge-pins.dsn` — Declares signal (~206 tok)

## rust/harness/src/

- `baseline.rs` — Distills oracle runs into committed golden baselines and compares runs (~8255 tok)
- `ci_tripwire.rs` — THE workflow tripwire pin (M3-T17c; closes banked mutant S6): the CI (~5283 tok)
- `drc_corpus.rs` — DRC parity corpus (M3 Task 2): the two counts every M3 quality gate (~11685 tok)
- `dsn_corpus.rs` — /*.dsn` lexicographic; dedup by path across the two (~18880 tok)
- `global_golden.rs` — The M6-T7 settings-ON golden face (`epic-harness global-golden`) — (~4460 tok)
- `oracle.rs` — `, the corpus dirs, events-golden, (~8438 tok)
- `route_events.rs` — Route event-stream corpus (M3 Task 16) — the maze-level (~32655 tok)
- `router_compare.rs` — Router quality scoreboard (M3 Task 15): DIRECTIONAL compare gates for (~51671 tok)

## rust/scripts/

- `package-linux.sh` — The Linux release artifact, reproduced locally (M10-T5): the exact (~697 tok)

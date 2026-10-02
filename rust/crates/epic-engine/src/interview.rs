//! F4: the pre-route interview — constraint inference from the board
//! itself (Lysdex's ask, relayed by Tyler: "for people that dont
//! understand what they need to feed it… so really it needs to
//! prompt the user for information").
//!
//! The engine DERIVES the questions; the host decides how to ask.
//! Three detector families, each feeding a setting that already
//! exists (every question is actionable — a question with no
//! settings face would be noise):
//!
//! * [`InterviewQuestion::GroundPour`] — the F3 ask folded in: a
//!   ground-like net with no pour ([`crate::pour::pour_candidates`]),
//!   minus nets the caller already poured. Feeds `router.pour.nets`.
//! * [`InterviewQuestion::DiffPair`] — two nets whose NAMES form a
//!   differential pair (`USB_DP`/`USB_DN`, `CLK_P`/`CLK_N`,
//!   `D+`/`D-`): the KiCad/Altium naming families. Feeds
//!   `router.tuning.pairs` (the M7 pair-tuning declaration).
//! * [`InterviewQuestion::CurrentWidth`] — a power-looking rail
//!   (`VCC`/`VDD`/`VBAT`…, `3V3`/`5V` volt rails) whose width the
//!   user may want current-driven. Feeds `router.current.nets`
//!   (amps is the user's answer — the one thing a board cannot say).
//!
//! Like F3's detection face: PURE (no mutation, no sink events, no
//! manifest rows), deterministic (net-number order within each
//! family, families in a fixed order), and OFF by construction —
//! the host opts in. Every golden face is byte-identical unless a
//! host surface prints.

use epic_board::board::Board;

use crate::pour::{is_ground_like, pour_candidates};
use crate::settings::MergedSettings;

/// One inferred constraint, phrased as a question the host renders
/// (the CLI's `--interview` loop and the GUI's pre-route dialog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterviewQuestion {
    /// A ground-like net with no copper pour — "synthesize one?"
    /// Feeds `router.pour.nets` (layer = the last signal layer
    /// unless the user says otherwise).
    GroundPour {
        /// The board's own net spelling (the canonical-name law).
        net_name: String,
        /// On-board pins carrying the net (the pour's would-be load).
        pin_count: usize,
    },
    /// Two nets whose names pair as differential — "tune as a diff
    /// pair?" Feeds `router.tuning.pairs`.
    DiffPair {
        /// The lower-net-number member, board spelling.
        net_a: String,
        /// Its partner, board spelling.
        net_b: String,
    },
    /// A power-looking rail — "how many amps does it carry?" Feeds
    /// `router.current.nets` (the answer is amps, the one value the
    /// board cannot declare).
    CurrentWidth {
        /// The board's own net spelling.
        net_name: String,
        /// On-board pins carrying the net (a 1-pin rail routes
        /// nothing and is never asked).
        pin_count: usize,
    },
}

impl InterviewQuestion {
    /// The exact `--router.` fragment that answers this question —
    /// the CLI prints it verbatim in show mode; the GUI's dialog
    /// caption mirrors it. `<amps>` marks the user's value.
    #[must_use]
    pub fn setting_hint(&self) -> String {
        match self {
            InterviewQuestion::GroundPour { net_name, .. } => {
                format!("--router.pour.nets={net_name}")
            }
            InterviewQuestion::DiffPair { net_a, net_b } => {
                format!("--router.tuning.pairs={net_a}:{net_b}")
            }
            InterviewQuestion::CurrentWidth { net_name, .. } => {
                format!("--router.current.nets={net_name}:<amps>")
            }
        }
    }
}

/// The volt-rail name shape: `^[+-]?\d+(\.\d+)?V\d*$` uppercased —
/// `5V`, `3V3`, `1V8`, `12V`, `+3V3`, `-12V`. Hand-parsed (no regex
/// dependency): split at the FIRST `V`, the head must be a plain
/// number, the tail (the second rail digit, `3V3`'s `3`) digits or
/// empty. `VCC` (empty head) and `VIN` (non-digit tail) fail both.
fn is_volt_rail(upper: &str) -> bool {
    let s = upper
        .strip_prefix('+')
        .or_else(|| upper.strip_prefix('-'))
        .unwrap_or(upper);
    let Some((head, tail)) = s.split_once('V') else {
        return false;
    };
    let plain_number = !head.is_empty() && head.chars().all(|c| c.is_ascii_digit() || c == '.');
    let rail_digit = tail.is_empty() || tail.chars().all(|c| c.is_ascii_digit());
    plain_number && rail_digit
}

/// The power-name heuristic: the uppercased name starts with a rail
/// family prefix (`VCC`, `VDD`, `VBAT`, …) or is a volt rail
/// (`3V3`, `+5V`). Grounds are excluded by the caller (they are
/// pour questions, not width questions). Deliberately a CLOSED set:
/// a missed prefix costs one unasked question, not a wrong route.
fn is_power_like(name: &str) -> bool {
    const RAIL_PREFIXES: [&str; 11] = [
        "VCC", "VDD", "VPP", "VEE", "VIN", "VOUT", "VBAT", "VBUS", "VSYS", "VAUX", "PWR",
    ];
    let upper = name.to_uppercase();
    RAIL_PREFIXES.iter().any(|p| upper.starts_with(p)) || is_volt_rail(&upper)
}

/// The differential-pair partner a net's NAME implies, as the
/// partner's expected UPPERCASE spelling (`USB_DP` → `USB_DN`,
/// `USB_DM` → `USB_DP`, `D+` → `D-`). The P/N/M terminal requires a
/// SEPARATOR before it (`_`, `.`, `-`) so ordinary trailing letters
/// never pair (`PIN`'s N is not a terminal); the literal `+`/`-`
/// terminals pair bare (KiCad's `D+`/`D-`). `None` = the name
/// implies no partner.
fn pair_partner_upper(name: &str) -> Option<String> {
    let upper = name.to_uppercase();
    for separator in ['_', '.', '-'] {
        // TWO-char terminals first: the USB family, where the marker
        // is the differential letter + the polarity letter
        // (`USB_DP`/`USB_DN`/`USB_DM`) — the longest match wins, or
        // `USB_DP` would miss entirely (it ends in `DP`, not `_P`).
        for (terminal, partner_terminal) in [("DP", "DN"), ("DN", "DP"), ("DM", "DP")] {
            let suffix = format!("{separator}{terminal}");
            if let Some(stem) = upper.strip_suffix(&suffix)
                && !stem.is_empty()
            {
                return Some(format!("{stem}{separator}{partner_terminal}"));
            }
        }
        // Then the single-letter polarity terminals
        // (`CLK_P`/`CLK_N`; `M` = minus, pairing with `P`).
        for (terminal, partner_terminal) in [("P", "N"), ("N", "P"), ("M", "P")] {
            let suffix = format!("{separator}{terminal}");
            if let Some(stem) = upper.strip_suffix(&suffix)
                && !stem.is_empty()
            {
                return Some(format!("{stem}{separator}{partner_terminal}"));
            }
        }
    }
    if let Some(stem) = upper.strip_suffix('+')
        && !stem.is_empty()
    {
        return Some(format!("{stem}-"));
    }
    if let Some(stem) = upper.strip_suffix('-')
        && !stem.is_empty()
    {
        return Some(format!("{stem}+"));
    }
    None
}

/// Case-insensitive membership: is `name` in a list of net-name
/// strings the caller already set (pour.nets / current.nets entries,
/// tuning.pairs members)?
fn listed_ignore_case(name: &str, names: &[String]) -> bool {
    let needle = name.to_uppercase();
    names.iter().any(|n| n.to_uppercase() == needle)
}

/// The DETECTION face (pure): every question the board implies,
/// minus everything the caller's merged settings already answer.
/// Family order is fixed (pours, pairs, currents — the same order
/// every host renders); within a family, net-number order. No
/// events, no mutation — the host decides how to ask.
#[must_use]
pub fn interview_questions(board: &Board, merged: &MergedSettings) -> Vec<InterviewQuestion> {
    let mut questions = Vec::new();

    // Pours: the F3 candidates minus nets the caller already poured.
    let poured = merged.pour_nets.clone().unwrap_or_default();
    for candidate in pour_candidates(board) {
        if listed_ignore_case(&candidate.net_name, &poured) {
            continue;
        }
        questions.push(InterviewQuestion::GroundPour {
            net_name: candidate.net_name,
            pin_count: candidate.pin_count,
        });
    }

    // Diff pairs: for each net in number order, the partner its name
    // implies; emit once at the LOWER number, skipping pairs the
    // caller already declared (either member order).
    let nets: Vec<(i32, String, bool)> = board
        .rules()
        .nets
        .iter()
        .map(|(number, net)| (number, net.name.clone(), net.contains_plane))
        .collect();
    let by_upper: std::collections::HashMap<String, (i32, String)> = nets
        .iter()
        .map(|(number, name, _)| (name.to_uppercase(), (*number, name.clone())))
        .collect();
    let declared_pairs = merged.tuning_pairs.clone().unwrap_or_default();
    for (number, name, contains_plane) in &nets {
        if *contains_plane || is_ground_like(name) {
            continue;
        }
        let Some(partner_upper) = pair_partner_upper(name) else {
            continue;
        };
        let Some(&(partner_number, ref partner_name)) = by_upper.get(&partner_upper) else {
            continue;
        };
        if partner_number < *number {
            continue; // emitted at the partner's visit
        }
        let already_declared = declared_pairs.iter().any(|(a, b)| {
            let (a_up, b_up) = (a.to_uppercase(), b.to_uppercase());
            let name_up = name.to_uppercase();
            let partner_up = partner_upper.clone();
            (a_up == name_up && b_up == partner_up) || (a_up == partner_up && b_up == name_up)
        });
        if already_declared {
            continue;
        }
        // The partner's own contains_plane: a plane member is not a
        // trace-tuning candidate.
        let partner_is_plane = nets
            .iter()
            .any(|(n, _, plane)| *n == partner_number && *plane);
        if partner_is_plane {
            continue;
        }
        questions.push(InterviewQuestion::DiffPair {
            net_a: name.clone(),
            net_b: partner_name.clone(),
        });
    }

    // Current widths: power-looking, non-plane, 2+ on-board pins,
    // not already current-driven.
    let current = merged.current_nets.clone().unwrap_or_default();
    let current_names: Vec<String> = current.iter().map(|r| r.net.clone()).collect();
    for (number, name, contains_plane) in &nets {
        if *contains_plane || is_ground_like(name) || !is_power_like(name) {
            continue;
        }
        if listed_ignore_case(name, &current_names) {
            continue;
        }
        let pins = crate::pour::pin_count(board, *number);
        if pins < 2 {
            continue;
        }
        questions.push(InterviewQuestion::CurrentWidth {
            net_name: name.clone(),
            pin_count: pins,
        });
    }

    questions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volt_rails_match_the_families_and_reject_lookalikes() {
        for name in ["5V", "3V3", "1V8", "12V", "+3V3", "-12V", "3.3V"] {
            assert!(is_volt_rail(name), "{name} is a volt rail");
        }
        for name in ["VCC", "VIN", "V5", "3VA", "3V3A", "V", "5vX"] {
            assert!(!is_volt_rail(name), "{name} is not a volt rail");
        }
    }

    #[test]
    fn power_names_cover_the_rail_families() {
        for name in [
            "VCC", "VDD", "VCC_3V3", "VBAT", "VBUS", "VIN", "VOUT", "PWR", "PWRIN", "3V3", "+5V",
        ] {
            assert!(is_power_like(name), "{name} is power-like");
        }
        for name in ["GND", "DATA0", "USB_DP", "VSS", "RX", "3V3_EN"] {
            assert!(!is_power_like(name), "{name} is not power-like");
        }
    }

    #[test]
    fn pair_partners_follow_the_separator_law() {
        assert_eq!(pair_partner_upper("USB_DP"), Some("USB_DN".to_string()));
        assert_eq!(pair_partner_upper("USB_DN"), Some("USB_DP".to_string()));
        assert_eq!(pair_partner_upper("usb_dm"), Some("USB_DP".to_string()));
        assert_eq!(pair_partner_upper("ETH0_DP"), Some("ETH0_DN".to_string()));
        assert_eq!(pair_partner_upper("CLK.P"), Some("CLK.N".to_string()));
        assert_eq!(pair_partner_upper("D+"), Some("D-".to_string()));
        assert_eq!(pair_partner_upper("USB_D-"), Some("USB_D+".to_string()));
        // No separator before P/N/M, and bare terminals: never pair.
        for name in ["PIN", "CLKP", "TOP", "P", "N", "M", "+", "-"] {
            assert_eq!(
                pair_partner_upper(name),
                None,
                "{name} must not imply a partner"
            );
        }
    }
}

/// The craft board for the integration face: the F3 pour craft's
/// geometry (CONN1 + T-targets) with the interview families as the
/// netlist. Every net has 2+ pins except GND (3) and 3V3 (3);
/// DATA0/DATA1 are the name-heuristic controls (ordinary nets that
/// must produce NO question).
#[cfg(test)]
mod interview_tests {
    use super::*;
    use crate::current_width::CurrentNetRequest;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    const INTERVIEW_CRAFT_DSN: &str = r#"(pcb interview-craft.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal)(property(index 0)))
    (layer B.Cu (type signal)(property(index 1)))
    (boundary (path pcb 0  0 0  128000 0  128000 128000  0 128000  0 0))
    (rule (clearance 250))
  )
  (placement
    (component "CONN" (place "CONN1" 20000 64000 Front 0.000000))
    (component "TGT" (place "T20" 36000 4000 Front 0.000000))
    (component "TGT" (place "T19" 36000 8000 Front 0.000000))
    (component "TGT" (place "T18" 36000 12000 Front 0.000000))
    (component "TGT" (place "T17" 36000 16000 Front 0.000000))
    (component "TGT" (place "T16" 36000 20000 Front 0.000000))
    (component "TGT" (place "T15" 36000 24000 Front 0.000000))
    (component "TGT" (place "T14" 36000 28000 Front 0.000000))
    (component "TGT" (place "T13" 36000 32000 Front 0.000000))
    (component "TGT" (place "T12" 36000 36000 Front 0.000000))
    (component "TGT" (place "T11" 36000 40000 Front 0.000000))
    (component "TGT" (place "T10" 36000 44000 Front 0.000000))
    (component "TGT" (place "T9" 36000 48000 Front 0.000000))
    (component "TGT" (place "T8" 36000 52000 Front 0.000000))
    (component "TGT" (place "T7" 36000 56000 Front 0.000000))
    (component "TGT" (place "T6" 36000 60000 Front 0.000000))
    (component "TGT" (place "T5" 100000 32000 Front 0.000000))
    (component "TGT" (place "T4" 100000 40000 Front 0.000000))
    (component "TGT" (place "T3" 100000 48000 Front 0.000000))
    (component "TGT" (place "T2" 100000 56000 Front 0.000000))
    (component "TGT" (place "T1" 100000 64000 Front 0.000000))
  )
  (library
    (image "CONN"
      (pin "PAD" "CA1" 0 0)
      (pin "PAD" "CA2" 0 -8000)
      (pin "PAD" "CA3" 0 -16000)
      (pin "PAD" "CA4" 0 -24000)
    )
    (image "TGT"
      (pin "PAD" "TA" 0 0)
    )
    (padstack "PAD"
      (shape (circle F.Cu 2000))
      (attach off)
    )
  )
  (network
    (net "GND" (pins "CONN1"-"CA1" "T1"-"TA" "T2"-"TA"))
    (net "AGND" (pins "CONN1"-"CA2" "T3"-"TA"))
    (net "USB_DP" (pins "CONN1"-"CA3" "T4"-"TA"))
    (net "USB_DN" (pins "CONN1"-"CA4" "T5"-"TA"))
    (net "CLK_P" (pins "T6"-"TA" "T7"-"TA"))
    (net "CLK_N" (pins "T8"-"TA" "T9"-"TA"))
    (net "DATA0" (pins "T10"-"TA" "T11"-"TA"))
    (net "DATA1" (pins "T12"-"TA" "T13"-"TA"))
    (net "3V3" (pins "T14"-"TA" "T15"-"TA" "T16"-"TA"))
    (net "VBAT" (pins "T17"-"TA" "T18"-"TA"))
    (net "PWR" (pins "T19"-"TA" "T20"-"TA"))
    (class kicad_default "GND" "AGND" "USB_DP" "USB_DN" "CLK_P" "CLK_N" "DATA0" "DATA1" "3V3" "VBAT" "PWR"
      (rule (clearance 250)(width 200))
    )
  )
)"#;

    fn interview_board() -> Board {
        let mut ses = SesBoard::new();
        match read_board(INTERVIEW_CRAFT_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    #[test]
    fn detects_the_three_families_in_family_then_net_order() {
        let board = interview_board();
        let questions = interview_questions(&board, &MergedSettings::default());
        let render: Vec<String> = questions
            .iter()
            .map(InterviewQuestion::setting_hint)
            .collect();
        // Family order: pours (GND then AGND), pairs (USB before
        // CLK — the lower net number), currents (3V3, VBAT, PWR in
        // net order). DATA0/DATA1 never appear (name controls).
        assert_eq!(
            render,
            [
                "--router.pour.nets=GND",
                "--router.pour.nets=AGND",
                "--router.tuning.pairs=USB_DP:USB_DN",
                "--router.tuning.pairs=CLK_P:CLK_N",
                "--router.current.nets=3V3:<amps>",
                "--router.current.nets=VBAT:<amps>",
                "--router.current.nets=PWR:<amps>",
            ]
        );
        // Pin counts ride along on the pour/current questions.
        assert!(questions.iter().any(|q| matches!(
            q,
            InterviewQuestion::GroundPour { net_name, pin_count: 3 } if net_name == "GND"
        )));
        assert!(questions.iter().any(|q| matches!(
            q,
            InterviewQuestion::CurrentWidth { net_name, pin_count: 3 } if net_name == "3V3"
        )));
    }

    #[test]
    fn already_answered_settings_retire_their_questions() {
        let board = interview_board();
        let merged = MergedSettings {
            pour_nets: Some(vec!["gnd".to_string()]), // case-insensitive
            tuning_pairs: Some(vec![("USB_DN".to_string(), "USB_DP".to_string())]), // reversed
            current_nets: Some(vec![CurrentNetRequest {
                net: "3V3".to_string(),
                amps: 2.0,
            }]),
            ..MergedSettings::default()
        };
        let questions = interview_questions(&board, &merged);
        let render: Vec<String> = questions
            .iter()
            .map(InterviewQuestion::setting_hint)
            .collect();
        assert_eq!(
            render,
            [
                "--router.pour.nets=AGND",
                "--router.tuning.pairs=CLK_P:CLK_N",
                "--router.current.nets=VBAT:<amps>",
                "--router.current.nets=PWR:<amps>",
            ]
        );
    }
}

// Consumed only by tests until the capture/verify CLI commands (M0 Tasks 4-7)
// start calling `TierFile::load`.
#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::PathBuf;

/// The curated fixture matrix (design §5). `fixtures_root` is resolved
/// against the repository root (the harness locates it by walking up from the
/// working directory until it finds `.git` + `build.gradle`).
#[derive(Debug, Deserialize)]
pub struct TierFile {
    pub fixtures_root: PathBuf,
    pub tiers: Vec<Tier>,
}

#[derive(Debug, Deserialize)]
pub struct Tier {
    pub name: String,
    pub fixtures: Vec<FixtureRef>,
}

#[derive(Debug, Deserialize)]
pub struct FixtureRef {
    /// Path relative to `fixtures_root`. May contain spaces (e.g.
    /// `KiCad_10_demos/sonde xilinx.dsn`).
    pub path: String,
    pub timeout_seconds: u64,
}

impl TierFile {
    /// Loads and validates the tier file at `path`.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading tier file {}", path.display()))?;
        let file: TierFile = serde_norway::from_str(&raw)
            .with_context(|| format!("parsing tier file {}", path.display()))?;
        file.validate()?;
        Ok(file)
    }

    /// Structural sanity: non-empty everywhere, positive timeouts.
    pub fn validate(&self) -> Result<()> {
        if self.tiers.is_empty() {
            bail!("tier file contains no tiers");
        }
        for tier in &self.tiers {
            if tier.name.is_empty() {
                bail!("tier with empty name");
            }
            if tier.fixtures.is_empty() {
                bail!("tier {} has no fixtures", tier.name);
            }
            for fixture in &tier.fixtures {
                if fixture.timeout_seconds == 0 {
                    bail!("fixture {} has timeout_seconds 0", fixture.path);
                }
            }
        }
        Ok(())
    }

    /// Case-insensitive tier lookup by name.
    pub fn tier(&self, name: &str) -> Option<&Tier> {
        self.tiers
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
fixtures_root: scripts/benchmark/fixtures
tiers:
  - name: A
    fixtures:
      - path: DAC2020_boards/DAC2020_bm08.dsn
        timeout_seconds: 120
      - path: KiCad_10_demos/sonde xilinx.dsn
        timeout_seconds: 300
  - name: C
    fixtures:
      - path: KiCad_10_demos/video.dsn
        timeout_seconds: 1800
"#;

    #[test]
    fn parses_tiers_and_preserves_spaces_in_paths() {
        let file: TierFile = serde_norway::from_str(SAMPLE).expect("sample must parse");
        assert_eq!(
            file.fixtures_root,
            PathBuf::from("scripts/benchmark/fixtures")
        );
        assert_eq!(file.tiers.len(), 2);
        let a = file.tier("A").expect("tier A");
        assert_eq!(a.fixtures.len(), 2);
        assert_eq!(a.fixtures[1].path, "KiCad_10_demos/sonde xilinx.dsn");
        assert_eq!(a.fixtures[1].timeout_seconds, 300);
        assert!(file.tier("B").is_none());
    }

    #[test]
    fn validation_rejects_zero_timeout_and_empty_tier() {
        let bad: TierFile =
            serde_norway::from_str("fixtures_root: x\ntiers:\n  - name: A\n    fixtures:\n      - path: a.dsn\n        timeout_seconds: 0\n")
                .expect("structurally valid");
        assert!(bad.validate().is_err());

        let empty: TierFile =
            serde_norway::from_str("fixtures_root: x\ntiers:\n  - name: A\n    fixtures: []\n")
                .expect("structurally valid");
        assert!(empty.validate().is_err());
    }
}

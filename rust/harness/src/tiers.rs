use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The curated fixture matrix (design §5). `fixtures_root` is resolved
/// against the repository root (the harness locates it by walking up from the
/// working directory until it finds `.git`; the pre-M10-T4 `build.gradle`
/// co-requirement retired with the gradle build).
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
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading tier file {}", path.display()))?;
        let file: TierFile = serde_norway::from_str(&raw)
            .with_context(|| format!("parsing tier file {}", path.display()))?;
        file.validate()
            .with_context(|| format!("validating tier file {}", path.display()))?;
        Ok(file)
    }

    /// Structural sanity: non-empty everywhere, positive timeouts, no
    /// duplicate tier names or fixture paths, fixture paths relative to
    /// `fixtures_root`.
    pub fn validate(&self) -> Result<()> {
        if self.tiers.is_empty() {
            bail!("tier file contains no tiers");
        }
        let mut tier_names = HashSet::new();
        let mut fixture_paths = HashSet::new();
        for (index, tier) in self.tiers.iter().enumerate() {
            if tier.name.is_empty() {
                bail!("tier {index} has an empty name");
            }
            if !tier_names.insert(tier.name.as_str()) {
                bail!("duplicate tier name {}", tier.name);
            }
            if tier.fixtures.is_empty() {
                bail!("tier {} has no fixtures", tier.name);
            }
            for fixture in &tier.fixtures {
                if !Path::new(&fixture.path).is_relative() {
                    bail!(
                        "fixture path {} must be relative to fixtures_root",
                        fixture.path
                    );
                }
                if !fixture_paths.insert(fixture.path.as_str()) {
                    bail!("duplicate fixture path {}", fixture.path);
                }
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
        // Lookup is case-insensitive.
        assert!(file.tier("a").is_some());
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

    #[test]
    fn validation_rejects_duplicate_fixture_path() {
        let dup: TierFile = serde_norway::from_str(
            "fixtures_root: x\ntiers:\n  - name: A\n    fixtures:\n      - path: a.dsn\n        timeout_seconds: 1\n      - path: a.dsn\n        timeout_seconds: 2\n",
        )
        .expect("structurally valid");
        assert!(dup.validate().is_err());
    }

    #[test]
    fn validation_rejects_absolute_fixture_path() {
        let abs: TierFile = serde_norway::from_str(
            "fixtures_root: x\ntiers:\n  - name: A\n    fixtures:\n      - path: /etc/passwd\n        timeout_seconds: 1\n",
        )
        .expect("structurally valid");
        assert!(abs.validate().is_err());
    }
}

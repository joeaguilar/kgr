use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::rules::RuleViolation;

const BASELINE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct BaselineRuleViolation {
    pub rule: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Baseline {
    pub version: u32,
    /// Each inner Vec is a canonicalized cycle (smallest node first).
    pub cycles: Vec<Vec<String>>,
    pub rule_violations: Vec<BaselineRuleViolation>,
}

impl Baseline {
    pub fn new(cycles: &[Vec<PathBuf>], rule_violations: &[RuleViolation]) -> Self {
        let mut canonical_cycles: Vec<Vec<String>> =
            cycles.iter().map(|c| normalize_cycle(c)).collect();
        canonical_cycles.sort();
        canonical_cycles.dedup();

        let mut brvs: Vec<BaselineRuleViolation> = rule_violations
            .iter()
            .map(|v| BaselineRuleViolation {
                rule: v.rule_name.clone(),
                from: baseline_path_string(&v.from),
                to: baseline_path_string(&v.to),
            })
            .collect();
        brvs.sort_by(|a, b| (&a.rule, &a.from, &a.to).cmp(&(&b.rule, &b.from, &b.to)));
        brvs.dedup();

        Self {
            version: BASELINE_VERSION,
            cycles: canonical_cycles,
            rule_violations: brvs,
        }
    }

    pub fn load(path: &Path) -> std::io::Result<Option<Self>> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let baseline: Self = serde_json::from_str(&content).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("malformed baseline JSON: {e}"),
            )
        })?;
        Ok(Some(baseline.normalized()))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, content)
    }

    /// Returns only the cycles not present in this baseline.
    pub fn new_cycles<'a>(&self, cycles: &'a [Vec<PathBuf>]) -> Vec<&'a Vec<PathBuf>> {
        cycles
            .iter()
            .filter(|c| !self.cycles.contains(&normalize_cycle(c)))
            .collect()
    }

    /// Returns only the rule violations not present in this baseline.
    pub fn new_rule_violations<'a>(
        &self,
        violations: &'a [RuleViolation],
    ) -> Vec<&'a RuleViolation> {
        violations
            .iter()
            .filter(|v| {
                let brv = BaselineRuleViolation {
                    rule: v.rule_name.clone(),
                    from: baseline_path_string(&v.from),
                    to: baseline_path_string(&v.to),
                };
                !self.rule_violations.contains(&brv)
            })
            .collect()
    }

    pub fn total(&self) -> usize {
        self.cycles.len() + self.rule_violations.len()
    }

    fn normalized(mut self) -> Self {
        for cycle in &mut self.cycles {
            for path in cycle.iter_mut() {
                *path = normalize_separators(path);
            }
            let paths: Vec<PathBuf> = cycle.iter().map(PathBuf::from).collect();
            *cycle = normalize_cycle(&paths);
        }
        self.cycles.sort();
        self.cycles.dedup();

        for violation in &mut self.rule_violations {
            violation.from = normalize_separators(&violation.from);
            violation.to = normalize_separators(&violation.to);
        }
        self.rule_violations
            .sort_by(|a, b| (&a.rule, &a.from, &a.to).cmp(&(&b.rule, &b.from, &b.to)));
        self.rule_violations.dedup();

        self
    }
}

fn baseline_path_string(path: &Path) -> String {
    normalize_separators(&path.to_string_lossy())
}

fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}

/// Canonicalize a cycle by rotating so the lexicographically smallest node is first.
fn normalize_cycle(cycle: &[PathBuf]) -> Vec<String> {
    if cycle.is_empty() {
        return Vec::new();
    }
    let strings: Vec<String> = cycle.iter().map(|p| baseline_path_string(p)).collect();
    let min_pos = strings
        .iter()
        .enumerate()
        .min_by_key(|(_, s)| s.as_str())
        .map(|(i, _)| i)
        .unwrap_or(0);
    strings[min_pos..]
        .iter()
        .chain(strings[..min_pos].iter())
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Severity;

    fn rule_violation(rule: &str, from: &str, to: &str) -> RuleViolation {
        RuleViolation {
            rule_name: rule.to_string(),
            from: PathBuf::from(from),
            to: PathBuf::from(to),
            severity: Severity::Error,
        }
    }

    #[test]
    fn new_baseline_normalizes_path_separators() {
        let baseline = Baseline::new(
            &[vec![PathBuf::from(r"src\b.rs"), PathBuf::from(r"src\a.rs")]],
            &[rule_violation(
                "no-legacy",
                r"legacy\repo.ts",
                r"core\db.ts",
            )],
        );

        assert_eq!(baseline.cycles, vec![vec!["src/a.rs", "src/b.rs"]]);
        assert_eq!(baseline.rule_violations[0].from, "legacy/repo.ts");
        assert_eq!(baseline.rule_violations[0].to, "core/db.ts");
    }

    #[test]
    fn loaded_windows_style_baseline_suppresses_unix_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".kgr-baseline.json");
        std::fs::write(
            &path,
            r#"{
  "version": 1,
  "cycles": [["src\\b.rs", "src\\a.rs"]],
  "rule_violations": [
    {"rule": "no-legacy", "from": "legacy\\repo.ts", "to": "core\\db.ts"}
  ]
}"#,
        )
        .unwrap();

        let baseline = Baseline::load(&path).unwrap().unwrap();
        let cycles = vec![vec![PathBuf::from("src/b.rs"), PathBuf::from("src/a.rs")]];
        let violations = vec![rule_violation("no-legacy", "legacy/repo.ts", "core/db.ts")];

        assert!(baseline.new_cycles(&cycles).is_empty());
        assert!(baseline.new_rule_violations(&violations).is_empty());
    }
}

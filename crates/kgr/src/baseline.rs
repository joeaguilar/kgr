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
    pub fn new(
        cycles: &[Vec<PathBuf>],
        rule_violations: &[RuleViolation],
    ) -> Self {
        let mut canonical_cycles: Vec<Vec<String>> = cycles
            .iter()
            .map(|c| normalize_cycle(c))
            .collect();
        canonical_cycles.sort();
        canonical_cycles.dedup();

        let mut brvs: Vec<BaselineRuleViolation> = rule_violations
            .iter()
            .map(|v| BaselineRuleViolation {
                rule: v.rule_name.clone(),
                from: v.from.to_string_lossy().into_owned(),
                to: v.to.to_string_lossy().into_owned(),
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

    pub fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
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
                    from: v.from.to_string_lossy().into_owned(),
                    to: v.to.to_string_lossy().into_owned(),
                };
                !self.rule_violations.contains(&brv)
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.cycles.is_empty() && self.rule_violations.is_empty()
    }

    pub fn total(&self) -> usize {
        self.cycles.len() + self.rule_violations.len()
    }
}

/// Canonicalize a cycle by rotating so the lexicographically smallest node is first.
fn normalize_cycle(cycle: &[PathBuf]) -> Vec<String> {
    if cycle.is_empty() {
        return Vec::new();
    }
    let strings: Vec<String> = cycle
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
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

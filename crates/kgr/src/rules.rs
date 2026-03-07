use std::path::PathBuf;

use globset::{Glob, GlobSetBuilder};
use kgr_core::types::{DepEdge, DepGraph, ImportKind};

use crate::config::{Rule, Severity};

pub struct RuleViolation {
    pub rule_name: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub severity: Severity,
}

pub fn check_rules(graph: &DepGraph, rules: &[Rule]) -> Vec<RuleViolation> {
    if rules.is_empty() {
        return Vec::new();
    }

    // Pre-compile all glob sets
    let compiled: Vec<(String, globset::GlobSet, globset::GlobSet, &Severity)> = rules
        .iter()
        .filter_map(|rule| {
            let from_set = GlobSetBuilder::new()
                .add(Glob::new(&rule.from).ok()?)
                .build()
                .ok()?;
            let to_set = GlobSetBuilder::new()
                .add(Glob::new(&rule.to).ok()?)
                .build()
                .ok()?;
            Some((rule.name.clone(), from_set, to_set, &rule.severity))
        })
        .collect();

    let local_edges: Vec<&DepEdge> = graph
        .edges
        .iter()
        .filter(|e| e.kind == ImportKind::Local)
        .collect();

    let mut violations = Vec::new();

    for (name, from_set, to_set, severity) in &compiled {
        for edge in &local_edges {
            let from_str = edge.from.to_string_lossy();
            let to_str = edge.to.to_string_lossy();
            if from_set.is_match(from_str.as_ref()) && to_set.is_match(to_str.as_ref()) {
                violations.push(RuleViolation {
                    rule_name: name.clone(),
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    severity: (*severity).clone(),
                });
            }
        }
    }

    violations
}

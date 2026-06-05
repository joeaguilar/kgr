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

#[derive(Debug, Clone)]
pub struct RuleCompileError {
    pub rule_name: String,
    pub field: &'static str,
    pub pattern: String,
    pub message: String,
}

pub fn check_rules(
    graph: &DepGraph,
    rules: &[Rule],
) -> Result<Vec<RuleViolation>, Vec<RuleCompileError>> {
    if rules.is_empty() {
        return Ok(Vec::new());
    }

    let mut compile_errors = Vec::new();
    let mut compiled: Vec<(String, globset::GlobSet, globset::GlobSet, &Severity)> = Vec::new();

    for rule in rules {
        let from_set = compile_rule_glob(rule, "from", &rule.from);
        let to_set = compile_rule_glob(rule, "to", &rule.to);

        match (from_set, to_set) {
            (Ok(from_set), Ok(to_set)) => {
                compiled.push((rule.name.clone(), from_set, to_set, &rule.severity));
            }
            (from_result, to_result) => {
                if let Err(error) = from_result {
                    compile_errors.push(error);
                }
                if let Err(error) = to_result {
                    compile_errors.push(error);
                }
            }
        }
    }

    if !compile_errors.is_empty() {
        return Err(compile_errors);
    }

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

    Ok(violations)
}

fn compile_rule_glob(
    rule: &Rule,
    field: &'static str,
    pattern: &str,
) -> Result<globset::GlobSet, RuleCompileError> {
    let glob = Glob::new(pattern).map_err(|error| RuleCompileError {
        rule_name: rule.name.clone(),
        field,
        pattern: pattern.to_string(),
        message: error.to_string(),
    })?;

    GlobSetBuilder::new()
        .add(glob)
        .build()
        .map_err(|error| RuleCompileError {
            rule_name: rule.name.clone(),
            field,
            pattern: pattern.to_string(),
            message: error.to_string(),
        })
}

use std::path::PathBuf;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use kgr_core::types::{DepEdge, DepGraph, ImportKind};

use crate::config::{Rule, Severity};

/// Rule glob semantics are intentionally path-like, not substring-like:
///
/// - Patterns are matched against root-relative paths and are anchored at the
///   scan root. `legacy/**` matches `legacy/foo.rs`, not `src/legacy/foo.rs`.
///   Use `**/legacy/**` for an any-depth directory match.
/// - `*` does not match path separators. Use `**` when a rule is meant to
///   cross directory boundaries.
const RULE_GLOB_SEMANTICS: &str = "rule globs are root-relative and anchored at the scan root; use '**/dir/**' for any-depth directory matches. '*' does not match path separators; use '**' to cross directories";

#[derive(Debug)]
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
    let check = evaluate_rules(graph, rules)?;
    for diagnostic in &check.diagnostics {
        diagnostic.emit();
    }
    Ok(check.violations)
}

#[derive(Debug, Default)]
struct RuleCheck {
    violations: Vec<RuleViolation>,
    diagnostics: Vec<RuleDeadDiagnostic>,
}

struct CompiledRule<'a> {
    name: String,
    from_pattern: String,
    to_pattern: String,
    from_set: GlobSet,
    to_set: GlobSet,
    severity: &'a Severity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuleDeadDiagnostic {
    rule_name: String,
    from_pattern: String,
    to_pattern: String,
    from_matches: usize,
    to_matches: usize,
}

impl RuleDeadDiagnostic {
    fn warning(&self) -> String {
        let reason = match (self.from_matches == 0, self.to_matches == 0) {
            (true, true) => "from and to globs matched no local edge endpoints",
            (true, false) => "from glob matched no local edge sources",
            (false, true) => "to glob matched no local edge targets",
            (false, false) => "rule has matching local edge endpoints",
        };

        format!(
            "warning[kgr::rule-config]: rule '{}' is likely dead: matched zero local import edges because {}; from='{}' matched {} source edge(s), to='{}' matched {} target edge(s). {}",
            self.rule_name,
            reason,
            self.from_pattern,
            self.from_matches,
            self.to_pattern,
            self.to_matches,
            RULE_GLOB_SEMANTICS
        )
    }

    fn emit(&self) {
        eprintln!("{}", self.warning());
    }
}

#[derive(Debug, Default)]
struct RuleMatchStats {
    from_matches: usize,
    to_matches: usize,
}

fn evaluate_rules(graph: &DepGraph, rules: &[Rule]) -> Result<RuleCheck, Vec<RuleCompileError>> {
    if rules.is_empty() {
        return Ok(RuleCheck::default());
    }

    let mut compile_errors = Vec::new();
    let mut compiled: Vec<CompiledRule<'_>> = Vec::new();

    for rule in rules {
        let from_set = compile_rule_glob(rule, "from", &rule.from);
        let to_set = compile_rule_glob(rule, "to", &rule.to);

        match (from_set, to_set) {
            (Ok(from_set), Ok(to_set)) => {
                compiled.push(CompiledRule {
                    name: rule.name.clone(),
                    from_pattern: rule.from.clone(),
                    to_pattern: rule.to.clone(),
                    from_set,
                    to_set,
                    severity: &rule.severity,
                });
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
    let mut diagnostics = Vec::new();

    for rule in &compiled {
        let mut stats = RuleMatchStats::default();
        for edge in &local_edges {
            let from_matches = rule.from_set.is_match(&edge.from);
            let to_matches = rule.to_set.is_match(&edge.to);

            if from_matches {
                stats.from_matches += 1;
            }
            if to_matches {
                stats.to_matches += 1;
            }
            if from_matches && to_matches {
                violations.push(RuleViolation {
                    rule_name: rule.name.clone(),
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    severity: (*rule.severity).clone(),
                });
            }
        }

        if stats.from_matches == 0 || stats.to_matches == 0 {
            diagnostics.push(RuleDeadDiagnostic {
                rule_name: rule.name.clone(),
                from_pattern: rule.from_pattern.clone(),
                to_pattern: rule.to_pattern.clone(),
                from_matches: stats.from_matches,
                to_matches: stats.to_matches,
            });
        }
    }

    Ok(RuleCheck {
        violations,
        diagnostics,
    })
}

fn compile_rule_glob(
    rule: &Rule,
    field: &'static str,
    pattern: &str,
) -> Result<globset::GlobSet, RuleCompileError> {
    let glob = GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|error| RuleCompileError {
            rule_name: rule.name.clone(),
            field,
            pattern: pattern.to_string(),
            message: error.to_string(),
        })?;

    let mut builder = GlobSetBuilder::new();
    builder.add(glob);
    builder.build().map_err(|error| RuleCompileError {
        rule_name: rule.name.clone(),
        field,
        pattern: pattern.to_string(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Severity;
    use kgr_core::types::DepGraph;

    fn rule(name: &str, from: &str, to: &str) -> Rule {
        Rule {
            name: name.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            severity: Severity::Error,
        }
    }

    fn graph(edges: &[(&str, &str)]) -> DepGraph {
        DepGraph {
            root: PathBuf::from("/repo"),
            files: Vec::new(),
            edges: edges
                .iter()
                .map(|(from, to)| DepEdge {
                    from: PathBuf::from(from),
                    to: PathBuf::from(to),
                    kind: ImportKind::Local,
                })
                .collect(),
            cycles: Vec::new(),
            roots: Vec::new(),
            orphans: Vec::new(),
            test_entries: Vec::new(),
        }
    }

    #[test]
    fn root_relative_directory_rule_matches_root_directory() {
        let graph = graph(&[("legacy/repo.ts", "core/db.ts")]);
        let rules = vec![rule("no-legacy-to-core", "legacy/**", "core/**")];

        let check = evaluate_rules(&graph, &rules).unwrap();

        assert_eq!(check.violations.len(), 1);
        assert!(check.diagnostics.is_empty());
    }

    #[test]
    fn root_relative_directory_rule_does_not_match_nested_directory() {
        let graph = graph(&[("src/legacy/repo.ts", "src/core/db.ts")]);
        let rules = vec![rule("no-nested-legacy", "legacy/**", "src/core/**")];

        let check = evaluate_rules(&graph, &rules).unwrap();

        assert!(check.violations.is_empty());
        assert_eq!(check.diagnostics.len(), 1);
        let warning = check.diagnostics[0].warning();
        assert!(warning.contains("from glob matched no local edge sources"));
        assert!(warning.contains("root-relative and anchored at the scan root"));
    }

    #[test]
    fn any_depth_directory_rule_matches_nested_directory() {
        let graph = graph(&[("src/legacy/repo.ts", "src/core/db.ts")]);
        let rules = vec![rule("no-any-legacy", "**/legacy/**", "**/core/**")];

        let check = evaluate_rules(&graph, &rules).unwrap();

        assert_eq!(check.violations.len(), 1);
        assert!(check.diagnostics.is_empty());
    }

    #[test]
    fn single_star_does_not_cross_directory_separator() {
        let graph = graph(&[("src/legacy/repo.ts", "src/core/db.ts")]);
        let rules = vec![rule("no-src-star", "src/*", "src/core/**")];

        let check = evaluate_rules(&graph, &rules).unwrap();

        assert!(check.violations.is_empty());
        assert_eq!(check.diagnostics.len(), 1);
        let warning = check.diagnostics[0].warning();
        assert!(warning.contains("from glob matched no local edge sources"));
        assert!(warning.contains("'*' does not match path separators"));
    }

    #[test]
    fn double_star_crosses_directory_boundaries() {
        let graph = graph(&[("src/legacy/repo.ts", "src/core/db.ts")]);
        let rules = vec![rule("no-src-double-star", "src/**", "src/core/**")];

        let check = evaluate_rules(&graph, &rules).unwrap();

        assert_eq!(check.violations.len(), 1);
        assert!(check.diagnostics.is_empty());
    }

    #[test]
    fn invalid_glob_reports_compile_error() {
        let graph = graph(&[("legacy/repo.ts", "core/db.ts")]);
        let rules = vec![rule("bad", "legacy/[", "core/**")];

        let errors = evaluate_rules(&graph, &rules).unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].rule_name, "bad");
        assert_eq!(errors[0].field, "from");
        assert_eq!(errors[0].pattern, "legacy/[");
    }
}

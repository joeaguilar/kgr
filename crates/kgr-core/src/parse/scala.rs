use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const SCALA_QUERY_SRC: &str = r#"
;; import scala.collection.mutable
(import_declaration) @import.decl
"#;

static SCALA_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_scala::LANGUAGE.into();
    Query::new(&language, SCALA_QUERY_SRC).expect("Failed to compile Scala import query")
});

static SCALA_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_scala::LANGUAGE.into();
    Query::new(&language, SCALA_SYMBOL_QUERY_SRC).expect("Failed to compile Scala symbol query")
});

const SCALA_SYMBOL_QUERY_SRC: &str = r#"
;; Class definition
(class_definition
  name: (identifier) @class.name)

;; Object definition (singleton)
(object_definition
  name: (identifier) @class.name)

;; Trait definition
(trait_definition
  name: (identifier) @class.name)

;; Function definition (def)
(function_definition
  name: (identifier) @fn.name)
"#;

static SCALA_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_scala::LANGUAGE.into();
    Query::new(&language, SCALA_CALL_QUERY_SRC).expect("Failed to compile Scala call query")
});

const SCALA_CALL_QUERY_SRC: &str = r#"
;; Function/method call
(call_expression
  function: (identifier) @call.name)

;; Dot call: obj.method()
(call_expression
  function: (field_expression
    field: (identifier) @call.method))
"#;

thread_local! {
    static SCALA_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_scala::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct ScalaParser;

impl ScalaParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        SCALA_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Scala file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for ScalaParser {
    fn lang(&self) -> Lang {
        Lang::Scala
    }

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_scala::LANGUAGE.into())
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*SCALA_SYMBOL_QUERY;
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                let kind = if capture.index == class_idx {
                    SymbolKind::Class
                } else if capture.index == fn_idx {
                    SymbolKind::Function
                } else {
                    continue;
                };

                // Check if the declaration has a "modifiers" child containing "private"
                // Scala AST: (class_definition (modifiers (access_modifier)) name: ...)
                // If no modifier or modifier is not private → exported = true
                let decl_node = node.parent().unwrap();
                let exported = !(0..decl_node.child_count()).any(|i| {
                    let child = decl_node.child(i).unwrap();
                    child.kind() == "modifiers"
                        && child
                            .utf8_text(source)
                            .map(|t| t.contains("private"))
                            .unwrap_or(false)
                });

                let start = node.start_position();
                let end = node.end_position();

                symbols.push(Symbol {
                    exported,
                    name,
                    kind,
                    span: Span {
                        start_line: start.row + 1,
                        start_col: start.column,
                        end_line: end.row + 1,
                        end_col: end.column,
                    },
                });
            }
        }

        symbols
    }

    fn extract_calls(&self, source: &[u8], path: &Path) -> Vec<CallRef> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*SCALA_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                if capture.index != name_idx && capture.index != method_idx {
                    continue;
                }

                let callee_raw = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                let start = node.start_position();
                let end = node.end_position();

                calls.push(CallRef {
                    callee_raw,
                    span: Span {
                        start_line: start.row + 1,
                        start_col: start.column,
                        end_line: end.row + 1,
                        end_col: end.column,
                    },
                });
            }
        }

        calls
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*SCALA_QUERY;
        let mut cursor = QueryCursor::new();

        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                // Extract path by stripping the "import" prefix from declaration text
                let raw = match node.utf8_text(source) {
                    Ok(s) => s.strip_prefix("import").unwrap_or(s).trim().to_string(),
                    Err(_) => continue,
                };

                let start = node.start_position();
                let end = node.end_position();
                let span = Span {
                    start_line: start.row + 1,
                    start_col: start.column,
                    end_line: end.row + 1,
                    end_col: end.column,
                };

                // A single declaration may pull in several paths via comma
                // clauses and brace groups: `import a.b.{C, D}, m.n` ->
                // a.b.C, a.b.D, m.n. Expand so each path stands alone and
                // external_deps lists clean, brace-free names.
                for path in expand_import_clauses(&raw) {
                    if path.is_empty() || !seen.insert(path.clone()) {
                        continue;
                    }
                    imports.push(Import {
                        raw: path,
                        kind: ImportKind::External,
                        resolved: None,
                        span: Some(span),
                    });
                }
            }
        }

        imports
    }
}

/// Expand a Scala import argument (the declaration minus the `import` keyword)
/// into individual import paths. Comma-separated clauses split first
/// (`m.n, o.pp` -> `["m.n", "o.pp"]`), then each clause's brace group expands
/// against its prefix (`a.b.{C, D}` -> `["a.b.C", "a.b.D"]`). Renames
/// (`R => Renamed`, `R as Renamed`) strip to the original name; wildcards
/// (`_`, `*`, `given`) are kept as a trailing segment.
fn expand_import_clauses(arg: &str) -> Vec<String> {
    split_top_level(arg)
        .into_iter()
        .flat_map(|clause| expand_clause(clause.trim()))
        .collect()
}

fn expand_clause(clause: &str) -> Vec<String> {
    let Some(open) = clause.find('{') else {
        return vec![strip_rename(clause)];
    };
    let prefix = clause[..open].trim();
    let Some(close) = matching_brace(&clause[open..]) else {
        return vec![strip_rename(clause)];
    };
    let inner = &clause[open + 1..open + close];

    let mut out = Vec::new();
    for member in split_top_level(inner) {
        let member = strip_rename(member);
        if member.is_empty() {
            continue;
        }
        out.push(format!("{prefix}{member}"));
    }
    if out.is_empty() {
        out.push(prefix.trim_end_matches('.').to_string());
    }
    out
}

/// Strip a rename target, keeping the ORIGINAL name: `R => Renamed` and
/// `R as Renamed` both yield `R`. A given-by-type selector (`given TC`)
/// collapses to the bare `given` wildcard segment.
fn strip_rename(member: &str) -> String {
    let member = member.trim();
    let original = if let Some((head, _)) = member.split_once("=>") {
        head.trim()
    } else if let Some((head, _)) = member.split_once(" as ") {
        head.trim()
    } else {
        member
    };
    // `given Ordering[Int]` imports givens by type; keep just `given`.
    if let Some(pos) = original.find("given ") {
        if pos == 0 || original.as_bytes()[pos - 1] == b'.' {
            return original[..pos + "given".len()].to_string();
        }
    }
    original.to_string()
}

/// Byte index (relative to the leading `{`) of its matching `}`.
fn matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split on top-level commas, ignoring commas nested inside brace groups.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        ScalaParser.parse(src.as_bytes(), Path::new("Test.scala"))
    }

    #[test]
    fn simple_import() {
        let imports = parse("import scala.collection.mutable");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "scala.collection.mutable");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import scala.collection.mutable
import java.util.List
import com.example.MyClass
"#,
        );
        assert_eq!(imports.len(), 3);
    }

    fn raws(src: &str) -> Vec<String> {
        parse(src).into_iter().map(|i| i.raw).collect()
    }

    #[test]
    fn brace_group_expands_to_individual_paths() {
        assert_eq!(raws("import a.b.{C, D}"), vec!["a.b.C", "a.b.D"]);
    }

    #[test]
    fn comma_clauses_split_into_separate_imports() {
        assert_eq!(raws("import m.n, o.pp"), vec!["m.n", "o.pp"]);
    }

    #[test]
    fn scala2_rename_strips_to_original_name() {
        assert_eq!(raws("import p.q.{R => Renamed}"), vec!["p.q.R"]);
    }

    #[test]
    fn scala3_rename_strips_to_original_name() {
        assert_eq!(raws("import p.q.{R as Renamed}"), vec!["p.q.R"]);
    }

    #[test]
    fn scala3_top_level_rename_strips_to_original_name() {
        assert_eq!(
            raws("import java.util.List as JList"),
            vec!["java.util.List"]
        );
    }

    #[test]
    fn wildcard_underscore_retained_as_trailing_segment() {
        assert_eq!(raws("import a.b._"), vec!["a.b._"]);
    }

    #[test]
    fn wildcard_star_retained_as_trailing_segment() {
        assert_eq!(raws("import a.b.*"), vec!["a.b.*"]);
    }

    #[test]
    fn given_retained_as_trailing_segment() {
        assert_eq!(raws("import a.b.given"), vec!["a.b.given"]);
    }

    #[test]
    fn group_with_wildcard_member() {
        assert_eq!(raws("import a.b.{C, _}"), vec!["a.b.C", "a.b._"]);
    }

    #[test]
    fn hide_clause_keeps_wildcard() {
        // `C => _` hides C; the wildcard still imports the rest.
        let r = raws("import a.b.{C => _, _}");
        assert!(r.contains(&"a.b._".to_string()));
        for raw in &r {
            assert!(!raw.contains("=>"), "rename arrow leaked: {raw}");
        }
    }

    #[test]
    fn comma_clause_with_brace_group_expands_both() {
        assert_eq!(
            raws("import a.b.{C, D}, m.n"),
            vec!["a.b.C", "a.b.D", "m.n"]
        );
    }

    #[test]
    fn duplicate_expansions_do_not_double_emit() {
        let r = raws("import a.b.{C, D}\nimport a.b.C");
        assert_eq!(r, vec!["a.b.C", "a.b.D"]);
    }

    #[test]
    fn no_raw_contains_grouping_or_rename_syntax() {
        let r = raws(
            r#"
import a.b.{C, D}
import p.q.{R => Renamed}
import s.t.{U as Aliased}
import m.n, o.pp
import w.x._
import y.z.*
"#,
        );
        assert!(!r.is_empty());
        for raw in &r {
            assert!(!raw.contains('{'), "brace leaked: {raw}");
            assert!(!raw.contains('}'), "brace leaked: {raw}");
            assert!(!raw.contains(','), "comma leaked: {raw}");
            assert!(!raw.contains("=>"), "arrow leaked: {raw}");
            assert!(!raw.contains(" as "), "rename leaked: {raw}");
        }
    }

    #[test]
    fn expanded_imports_share_declaration_span() {
        let imports = parse("import a.b.{C, D}");
        assert_eq!(imports.len(), 2);
        let lines: Vec<_> = imports
            .iter()
            .map(|i| i.span.as_ref().unwrap().start_line)
            .collect();
        assert_eq!(lines, vec![1, 1]);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        ScalaParser.extract_symbols(src.as_bytes(), Path::new("Test.scala"))
    }

    #[test]
    fn symbols_finds_classes() {
        let syms = symbols("class MyClass {}");
        assert!(syms
            .iter()
            .any(|s| s.name == "MyClass" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_objects() {
        let syms = symbols("object MyObject {}");
        assert!(syms
            .iter()
            .any(|s| s.name == "MyObject" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_traits() {
        let syms = symbols("trait Drawable {}");
        assert!(syms
            .iter()
            .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("object Main { def hello(): Unit = {} }");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "hello");
    }

    #[test]
    fn symbols_exported_private() {
        let syms = symbols("class Pub {}\nprivate class Priv {}");
        let pub_class = syms.iter().find(|s| s.name == "Pub").unwrap();
        let priv_class = syms.iter().find(|s| s.name == "Priv");
        assert!(pub_class.exported);
        if let Some(priv_class) = priv_class {
            assert!(!priv_class.exported);
        }
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        ScalaParser.extract_calls(src.as_bytes(), Path::new("Test.scala"))
    }

    #[test]
    fn calls_function_invocation() {
        let c = calls("object T { def f(): Unit = { foo() } }");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
    }

    #[test]
    fn calls_method_invocation() {
        let c = calls("object T { def f(): Unit = { obj.method() } }");
        assert!(c.iter().any(|c| c.callee_raw == "method"));
    }
}

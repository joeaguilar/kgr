use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const GO_QUERY_SRC: &str = r#"
;; Single import
(import_declaration
  (import_spec
    path: (interpreted_string_literal) @import.path))

;; Import block
(import_declaration
  (import_spec_list
    (import_spec
      path: (interpreted_string_literal) @import.path)))
"#;

static GO_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_go::LANGUAGE.into();
    Query::new(&language, GO_QUERY_SRC).expect("Failed to compile Go query")
});

static GO_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_go::LANGUAGE.into();
    Query::new(&language, GO_SYMBOL_QUERY_SRC).expect("Failed to compile Go symbol query")
});

const GO_SYMBOL_QUERY_SRC: &str = r#"
;; Function declaration
(function_declaration
  name: (identifier) @fn.name)

;; Method declaration (note: field_identifier, NOT identifier)
(method_declaration
  name: (field_identifier) @method.name)

;; Type declaration (struct, interface, etc.)
(type_declaration
  (type_spec
    name: (type_identifier) @class.name))
"#;

static GO_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_go::LANGUAGE.into();
    Query::new(&language, GO_CALL_QUERY_SRC).expect("Failed to compile Go call query")
});

const GO_CALL_QUERY_SRC: &str = r#"
;; Simple function call: foo()
(call_expression
  function: (identifier) @call.name)

;; Selector call: obj.Method() or pkg.Function()
(call_expression
  function: (selector_expression
    field: (field_identifier) @call.method))
"#;

thread_local! {
    static GO_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_go::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct GoParser;

impl GoParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        GO_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Go file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for GoParser {
    fn lang(&self) -> Lang {
        Lang::Go
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*GO_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let method_idx = query.capture_index_for_name("method.name").unwrap();

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

                let kind = if capture.index == fn_idx {
                    SymbolKind::Function
                } else if capture.index == class_idx {
                    SymbolKind::Class
                } else if capture.index == method_idx {
                    SymbolKind::Method
                } else {
                    continue;
                };

                let exported = name
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false);

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

        let query = &*GO_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let callee_raw = if capture.index == name_idx {
                    // Simple call: foo()
                    match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == method_idx {
                    // Selector call: get the full `pkg.Method` text from the parent selector_expression
                    let sel_node = node.parent().unwrap();
                    match sel_node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else {
                    continue;
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

        {
            let query = &*GO_QUERY;
            let mut cursor = QueryCursor::new();

            let mut imports = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut matches = cursor.matches(query, tree.root_node(), source);

            while let Some(m) = matches.next() {
                for capture in m.captures {
                    let node = capture.node;
                    let full_text = match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    };

                    // Strip surrounding quotes
                    let raw = full_text
                        .trim_start_matches('"')
                        .trim_end_matches('"')
                        .to_string();

                    if !seen.insert(raw.clone()) {
                        continue;
                    }

                    // Go imports: relative paths (starting with ./) are local,
                    // everything else is external (module paths)
                    let kind = if raw.starts_with("./") || raw.starts_with("../") {
                        ImportKind::Local
                    } else {
                        ImportKind::External
                    };

                    let start = node.start_position();
                    let end = node.end_position();

                    imports.push(Import {
                        raw,
                        kind,
                        resolved: None,
                        span: Some(Span {
                            start_line: start.row + 1,
                            start_col: start.column,
                            end_line: end.row + 1,
                            end_col: end.column,
                        }),
                    });
                }
            }

            imports
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        GoParser.parse(src.as_bytes(), Path::new("main.go"))
    }

    #[test]
    fn single_import() {
        let imports = parse(r#"import "fmt""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "fmt");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn import_block() {
        let imports = parse(
            r#"
import (
    "fmt"
    "os"
    "net/http"
)
"#,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw, "fmt");
        assert_eq!(imports[1].raw, "os");
        assert_eq!(imports[2].raw, "net/http");
    }

    #[test]
    fn relative_import() {
        let imports = parse(r#"import "./utils""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn module_import() {
        let imports = parse(r#"import "github.com/user/repo/pkg""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "github.com/user/repo/pkg");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        GoParser.extract_symbols(src.as_bytes(), Path::new("main.go"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("package main\nfunc foo() {}\nfunc bar() {}\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    #[test]
    fn symbols_finds_types() {
        let syms = symbols("package main\ntype UserService struct {}\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "UserService" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols("package main\ntype Svc struct{}\nfunc (s *Svc) Handle() {}\n");
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "Handle");
    }

    #[test]
    fn symbols_exported_uppercase() {
        let syms = symbols("package main\nfunc Public() {}\nfunc private() {}\n");
        let public = syms.iter().find(|s| s.name == "Public").unwrap();
        let private = syms.iter().find(|s| s.name == "private").unwrap();
        assert!(public.exported);
        assert!(!private.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        GoParser.extract_calls(src.as_bytes(), Path::new("main.go"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("package main\nfunc main() { foo(); bar(); }\n");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
        assert!(c.iter().any(|c| c.callee_raw == "bar"));
    }

    #[test]
    fn calls_selector() {
        let c = calls("package main\nimport \"fmt\"\nfunc main() { fmt.Println(\"hi\") }\n");
        assert!(c.iter().any(|c| c.callee_raw == "fmt.Println"));
    }
}

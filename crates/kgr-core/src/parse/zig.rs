use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const ZIG_IMPORT_QUERY_SRC: &str = r#"
(builtin_function
  (builtin_identifier) @_fn
  (arguments (string (string_content) @import.path)))
"#;

static ZIG_IMPORT_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_zig::LANGUAGE.into();
    Query::new(&language, ZIG_IMPORT_QUERY_SRC).expect("Failed to compile Zig import query")
});

const ZIG_SYMBOL_QUERY_SRC: &str = r#"
;; Function declaration
(function_declaration
  name: (identifier) @fn.name)

;; Top-level variable/const declarations
(variable_declaration
  (identifier) @var.name)
"#;

static ZIG_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_zig::LANGUAGE.into();
    Query::new(&language, ZIG_SYMBOL_QUERY_SRC).expect("Failed to compile Zig symbol query")
});

const ZIG_CALL_QUERY_SRC: &str = r#"
;; Function call
(call_expression
  function: (identifier) @call.name)

;; Method/field call
(call_expression
  function: (field_expression
    member: (identifier) @call.method))
"#;

static ZIG_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_zig::LANGUAGE.into();
    Query::new(&language, ZIG_CALL_QUERY_SRC).expect("Failed to compile Zig call query")
});

thread_local! {
    static ZIG_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_zig::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct ZigParser;

impl ZigParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        ZIG_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Zig file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for ZigParser {
    fn lang(&self) -> Lang {
        Lang::Zig
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*ZIG_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let var_idx = query.capture_index_for_name("var.name").unwrap();

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

                if capture.index != fn_idx && capture.index != var_idx {
                    continue;
                }
                let kind = SymbolKind::Function;

                // Zig uses `pub` for exported symbols.
                // Check the immediate declaration parent (function_declaration or
                // variable_declaration) for a "pub" child node.
                let exported = {
                    let mut exported = false;
                    if let Some(parent) = node.parent() {
                        // Check direct children of the declaration for a "pub" token
                        let mut cursor = parent.walk();
                        if cursor.goto_first_child() {
                            loop {
                                if cursor.node().kind() == "pub" {
                                    exported = true;
                                    break;
                                }
                                // Only check nodes before the identifier itself
                                if cursor.node().id() == node.id() {
                                    break;
                                }
                                if !cursor.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    }
                    exported
                };

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

        let query = &*ZIG_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let callee_raw = if capture.index == name_idx {
                    match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == method_idx {
                    // Get the full `obj.method` text from the parent field_expression
                    let field_node = node.parent().unwrap();
                    match field_node.utf8_text(source) {
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

        let query = &*ZIG_IMPORT_QUERY;
        let fn_idx = query.capture_index_for_name("_fn").unwrap();
        let path_idx = query.capture_index_for_name("import.path").unwrap();

        let mut cursor = QueryCursor::new();
        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            // Check that @_fn is "@import"
            let fn_text = m
                .captures
                .iter()
                .find(|c| c.index == fn_idx)
                .and_then(|c| c.node.utf8_text(source).ok());

            if fn_text != Some("@import") {
                continue;
            }

            for capture in m.captures {
                if capture.index != path_idx {
                    continue;
                }

                let node = capture.node;
                let raw = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                if !seen.insert(raw.clone()) {
                    continue;
                }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        ZigParser.parse(src.as_bytes(), Path::new("main.zig"))
    }

    #[test]
    fn import_std() {
        let imports = parse(r#"const std = @import("std");"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "std");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn import_local() {
        let imports = parse(r#"const utils = @import("./utils.zig");"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils.zig");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn import_parent_relative() {
        let imports = parse(r#"const lib = @import("../lib.zig");"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "../lib.zig");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
const std = @import("std");
const utils = @import("./utils.zig");
"#,
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].raw, "std");
        assert_eq!(imports[1].raw, "./utils.zig");
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        ZigParser.extract_symbols(src.as_bytes(), Path::new("main.zig"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols(
            r#"
fn foo() void {}
fn bar() void {}
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    #[test]
    fn symbols_pub_exported() {
        let syms = symbols("pub fn public_fn() void {}\nfn private_fn() void {}\n");
        let public = syms.iter().find(|s| s.name == "public_fn").unwrap();
        let private = syms.iter().find(|s| s.name == "private_fn").unwrap();
        assert!(public.exported);
        assert!(!private.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        ZigParser.extract_calls(src.as_bytes(), Path::new("main.zig"))
    }

    #[test]
    fn calls_simple() {
        let c = calls(
            r#"
fn main() void {
    foo();
    bar();
}
"#,
        );
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
        assert!(c.iter().any(|c| c.callee_raw == "bar"));
    }

    #[test]
    fn calls_method() {
        let c = calls(
            r#"
fn main() void {
    std.debug.print();
}
"#,
        );
        assert!(c.iter().any(|c| c.callee_raw == "std.debug.print"));
    }
}

use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const RUBY_QUERY_SRC: &str = r#"
;; require 'foo' / require_relative 'foo'
(call
  method: (identifier) @_fn
  arguments: (argument_list (string (string_content) @import.path)))
"#;

static RUBY_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_ruby::LANGUAGE.into();
    Query::new(&language, RUBY_QUERY_SRC).expect("Failed to compile Ruby import query")
});

const RUBY_SYMBOL_QUERY_SRC: &str = r#"
;; Method definition
(method
  name: (identifier) @fn.name)

;; Class definition
(class
  name: (constant) @class.name)

;; Module definition
(module
  name: (constant) @class.name)
"#;

static RUBY_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_ruby::LANGUAGE.into();
    Query::new(&language, RUBY_SYMBOL_QUERY_SRC).expect("Failed to compile Ruby symbol query")
});

const RUBY_CALL_QUERY_SRC: &str = r#"
;; Method call: foo(args)
(call
  method: (identifier) @call.name)
"#;

static RUBY_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_ruby::LANGUAGE.into();
    Query::new(&language, RUBY_CALL_QUERY_SRC).expect("Failed to compile Ruby call query")
});

thread_local! {
    static RUBY_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_ruby::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct RubyParser;

impl RubyParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        RUBY_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = parser.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Ruby file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for RubyParser {
    fn lang(&self) -> Lang {
        Lang::Ruby
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*RUBY_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();

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
                } else {
                    continue;
                };

                let start = node.start_position();
                let end = node.end_position();

                // Ruby convention: all top-level methods/classes are accessible
                symbols.push(Symbol {
                    exported: true,
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

        let query = &*RUBY_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                if capture.index != name_idx {
                    continue;
                }
                let node = capture.node;
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

        let query = &*RUBY_QUERY;
        let fn_capture_idx = query
            .capture_index_for_name("_fn")
            .expect("_fn capture must exist");
        let path_capture_idx = query
            .capture_index_for_name("import.path")
            .expect("import.path capture must exist");

        let mut cursor = QueryCursor::new();
        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            // Get the method name (_fn capture) to filter for require/require_relative
            let fn_capture = match m.captures.iter().find(|c| c.index == fn_capture_idx) {
                Some(c) => c,
                None => continue,
            };

            let fn_name = match fn_capture.node.utf8_text(source) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let is_require_relative = fn_name == "require_relative";
            if fn_name != "require" && !is_require_relative {
                continue;
            }

            // Extract the import path
            let path_capture = match m.captures.iter().find(|c| c.index == path_capture_idx) {
                Some(c) => c,
                None => continue,
            };

            let node = path_capture.node;
            let raw = match node.utf8_text(source) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };

            if !seen.insert(raw.clone()) {
                continue;
            }

            let kind = if is_require_relative || raw.starts_with("./") || raw.starts_with("../") {
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

        imports
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        RubyParser.parse(src.as_bytes(), Path::new("test.rb"))
    }

    #[test]
    fn require_statement() {
        let imports = parse("require 'json'");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "json");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn require_relative() {
        let imports = parse("require_relative 'helper'");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "helper");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn require_with_path_prefix() {
        let imports = parse("require './lib/utils'");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./lib/utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn require_external_gem() {
        let imports = parse("require 'rails'");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "rails");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn multiple_requires() {
        let imports = parse(
            r#"
require 'json'
require 'net/http'
require_relative 'config'
"#,
        );
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn ignores_non_require_calls() {
        let imports = parse("puts 'hello'");
        assert!(imports.is_empty());
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        RubyParser.extract_symbols(src.as_bytes(), Path::new("test.rb"))
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols("def foo\n  puts 'hi'\nend\n\ndef bar\n  puts 'bye'\nend\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    #[test]
    fn symbols_finds_classes() {
        let syms = symbols("class MyClass\nend\n");
        let classes: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "MyClass");
    }

    #[test]
    fn symbols_finds_modules() {
        let syms = symbols("module MyModule\nend\n");
        let classes: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "MyModule");
    }

    #[test]
    fn symbols_all_exported() {
        let syms = symbols("def _private_method\nend\n\ndef public_method\nend\n");
        assert!(syms.iter().all(|s| s.exported));
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        RubyParser.extract_calls(src.as_bytes(), Path::new("test.rb"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("puts 'hello'\nputs 'world'\n");
        assert!(c.iter().any(|call| call.callee_raw == "puts"));
    }

    #[test]
    fn calls_method_with_args() {
        let c = calls("require 'json'\nputs 'hello'\n");
        let names: Vec<_> = c.iter().map(|call| call.callee_raw.as_str()).collect();
        assert!(names.contains(&"require"));
        assert!(names.contains(&"puts"));
    }
}

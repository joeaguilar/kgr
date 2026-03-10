use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const LUA_QUERY_SRC: &str = r#"
;; require("module") or require "module"
(function_call
  name: (identifier) @_fn
  arguments: (arguments (string content: (string_content) @import.path)))
"#;

static LUA_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_lua::LANGUAGE.into();
    Query::new(&language, LUA_QUERY_SRC).expect("Failed to compile Lua import query")
});

const LUA_SYMBOL_QUERY_SRC: &str = r#"
;; Function declaration (both global and local)
(function_declaration
  name: (identifier) @fn.name)
"#;

static LUA_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_lua::LANGUAGE.into();
    Query::new(&language, LUA_SYMBOL_QUERY_SRC).expect("Failed to compile Lua symbol query")
});

const LUA_CALL_QUERY_SRC: &str = r#"
;; Function call: foo()
(function_call
  name: (identifier) @call.name)

;; Method call: obj:method()
(function_call
  name: (method_index_expression
    method: (identifier) @call.method))

;; Dot call: obj.method()
(function_call
  name: (dot_index_expression
    field: (identifier) @call.field))
"#;

static LUA_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_lua::LANGUAGE.into();
    Query::new(&language, LUA_CALL_QUERY_SRC).expect("Failed to compile Lua call query")
});

thread_local! {
    static LUA_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_lua::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct LuaParser;

impl LuaParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        LUA_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = parser.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Lua file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for LuaParser {
    fn lang(&self) -> Lang {
        Lang::Lua
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*LUA_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                if capture.index != fn_idx {
                    continue;
                }
                let node = capture.node;
                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                // Check if the function is local (not exported).
                // Walk up to the function_declaration node and check its source text.
                let fn_node = node.parent().unwrap();
                let fn_src = fn_node.utf8_text(source).unwrap_or("");
                let exported = !fn_src.starts_with("local");

                let start = node.start_position();
                let end = node.end_position();

                symbols.push(Symbol {
                    exported,
                    name,
                    kind: SymbolKind::Function,
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

        let query = &*LUA_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();
        let field_idx = query.capture_index_for_name("call.field").unwrap();

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
                } else if capture.index == method_idx || capture.index == field_idx {
                    // Get the full `obj:method` or `obj.field` text from the parent
                    let parent_node = node.parent().unwrap();
                    match parent_node.utf8_text(source) {
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

        let query = &*LUA_QUERY;
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
            // Filter: only match when _fn is "require"
            let fn_capture = match m.captures.iter().find(|c| c.index == fn_capture_idx) {
                Some(c) => c,
                None => continue,
            };

            let fn_name = match fn_capture.node.utf8_text(source) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if fn_name != "require" {
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

            let kind = if raw.starts_with('.') {
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
        LuaParser.parse(src.as_bytes(), Path::new("test.lua"))
    }

    #[test]
    fn require_statement() {
        let imports = parse(r#"local json = require("json")"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "json");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn require_with_string_arg() {
        let imports = parse(r#"local utils = require("utils")"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "utils");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn require_local_module() {
        let imports = parse(r#"local m = require("./mymodule")"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./mymodule");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn multiple_requires() {
        let imports = parse(
            r#"
local json = require("json")
local http = require("socket.http")
local m = require("./helpers")
"#,
        );
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn ignores_non_require_calls() {
        let imports = parse(r#"print("hello")"#);
        assert!(imports.is_empty());
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        LuaParser.extract_symbols(src.as_bytes(), Path::new("test.lua"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("function foo()\nend\n\nfunction bar()\nend\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    #[test]
    fn symbols_local_not_exported() {
        let syms = symbols("function public_fn()\nend\n\nlocal function private_fn()\nend\n");
        let public = syms.iter().find(|s| s.name == "public_fn").unwrap();
        let private = syms.iter().find(|s| s.name == "private_fn").unwrap();
        assert!(public.exported);
        assert!(!private.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        LuaParser.extract_calls(src.as_bytes(), Path::new("test.lua"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("print('hello')\nfoo(1, 2)\n");
        assert!(c.iter().any(|call| call.callee_raw == "print"));
        assert!(c.iter().any(|call| call.callee_raw == "foo"));
    }

    #[test]
    fn calls_method_call() {
        let c = calls("obj:method()\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "obj:method");
    }

    #[test]
    fn calls_dot_call() {
        let c = calls("math.floor(1.5)\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "math.floor");
    }
}

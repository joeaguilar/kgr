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

// Symbol naming convention: for table-attached functions (`function M.foo()`,
// `function M:bar()`) we emit the *trailing identifier* (`foo`, `bar`), not the
// full dotted text. Consumers alias module tables (`local mod = require("m");
// mod.foo()`), so the table half of the name is caller-specific; the trailing
// identifier is what `callee_matches` suffix-matching can connect to call sites.
const LUA_SYMBOL_QUERY_SRC: &str = r#"
;; Function declaration (both global and local)
(function_declaration
  name: (identifier) @fn.name)

;; Module-table function: function M.foo() end
(function_declaration
  name: (dot_index_expression field: (identifier) @fn.dotted))

;; Method on a table: function M:bar() end
(function_declaration
  name: (method_index_expression method: (identifier) @fn.method))

;; Function assigned to a variable: local f = function() end / g = function() end
(assignment_statement
  (variable_list name: (identifier) @fn.assigned)
  (expression_list value: (function_definition)))
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

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_lua::LANGUAGE.into())
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*LUA_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let dotted_idx = query.capture_index_for_name("fn.dotted").unwrap();
        let method_idx = query.capture_index_for_name("fn.method").unwrap();
        let assigned_idx = query.capture_index_for_name("fn.assigned").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let (kind, exported) = if capture.index == fn_idx {
                    // `function foo()` is global (exported); `local function foo()`
                    // is file-private. The `local` keyword is part of the
                    // function_declaration node's source text.
                    let fn_node = node.parent().unwrap();
                    let fn_src = fn_node.utf8_text(source).unwrap_or("");
                    (SymbolKind::Function, !fn_src.starts_with("local"))
                } else if capture.index == dotted_idx {
                    // `function M.foo()` attaches to a table — the module-table
                    // pattern. Cannot be declared local, so always exported.
                    (SymbolKind::Function, true)
                } else if capture.index == method_idx {
                    // `function M:bar()` — a method on a table; always exported.
                    (SymbolKind::Method, true)
                } else if capture.index == assigned_idx {
                    // `f = function() end` is a global assignment; the `local`
                    // form wraps the assignment_statement in a
                    // variable_declaration node.
                    let is_local = node
                        .parent() // variable_list
                        .and_then(|n| n.parent()) // assignment_statement
                        .and_then(|n| n.parent()) // variable_declaration | block | chunk
                        .is_some_and(|n| n.kind() == "variable_declaration");
                    (SymbolKind::Function, !is_local)
                } else {
                    continue;
                };

                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
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

    #[test]
    fn symbols_module_table_function() {
        // `function M.foo()` — module-table pattern; symbol is the trailing
        // identifier `foo` (documented naming convention) and is exported.
        let syms = symbols("local M = {}\n\nfunction M.foo()\nend\n\nreturn M\n");
        let foo = syms.iter().find(|s| s.name == "foo").unwrap();
        assert_eq!(foo.kind, SymbolKind::Function);
        assert!(foo.exported);
    }

    #[test]
    fn symbols_module_table_method() {
        // `function M:bar()` — Method kind, trailing identifier, exported.
        let syms = symbols("local M = {}\n\nfunction M:bar()\nend\n\nreturn M\n");
        let bar = syms.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
        assert!(bar.exported);
    }

    #[test]
    fn symbols_assigned_function() {
        // `local f = function() end` is file-private; `g = function() end`
        // assigns a global and is exported.
        let syms = symbols("local f = function()\nend\n\ng = function()\nend\n");
        let f = syms.iter().find(|s| s.name == "f").unwrap();
        assert_eq!(f.kind, SymbolKind::Function);
        assert!(!f.exported);
        let g = syms.iter().find(|s| s.name == "g").unwrap();
        assert_eq!(g.kind, SymbolKind::Function);
        assert!(g.exported);
    }

    #[test]
    fn symbols_non_function_assignment_ignored() {
        let syms = symbols("local M = {}\nlocal x = 1\ny = 'str'\n");
        assert!(syms.is_empty());
    }

    #[test]
    fn symbols_idiomatic_module_full() {
        // The standard Lua module shape yields all four symbols.
        let syms = symbols(
            "local M = {}\n\nfunction globalfn()\nend\n\nfunction M.dotted()\nend\n\nfunction M:colonmeth()\nend\n\nlocal helper = function()\nend\n\nreturn M\n",
        );
        assert_eq!(syms.len(), 4);
        assert!(syms
            .iter()
            .any(|s| s.name == "globalfn" && s.kind == SymbolKind::Function && s.exported));
        assert!(syms
            .iter()
            .any(|s| s.name == "dotted" && s.kind == SymbolKind::Function && s.exported));
        assert!(syms
            .iter()
            .any(|s| s.name == "colonmeth" && s.kind == SymbolKind::Method && s.exported));
        assert!(syms
            .iter()
            .any(|s| s.name == "helper" && s.kind == SymbolKind::Function && !s.exported));
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

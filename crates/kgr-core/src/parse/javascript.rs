use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const JS_QUERY_SRC: &str = r#"
;; ESM import
(import_statement
  source: (string (string_fragment) @import.path))

;; Re-export
(export_statement
  source: (string (string_fragment) @import.path))

;; Dynamic import
(call_expression
  function: (import)
  arguments: (arguments (string (string_fragment) @import.path)))

;; CommonJS require — we filter for "require" manually
(call_expression
  function: (identifier) @_fn
  arguments: (arguments (string (string_fragment) @import.path)))
"#;

const JS_SYMBOL_QUERY_SRC: &str = r#"
;; Exported function
(export_statement
  declaration: (function_declaration
    name: (identifier) @fn.exported))

;; Non-exported function
(program
  (function_declaration
    name: (identifier) @fn.name))

;; Exported class
(export_statement
  declaration: (class_declaration
    name: (identifier) @class.exported))

;; Non-exported class
(program
  (class_declaration
    name: (identifier) @class.name))

;; Method inside class
(class_declaration
  body: (class_body
    (method_definition
      name: (property_identifier) @method.name)))
"#;

static JS_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_javascript::LANGUAGE.into();
    Query::new(&language, JS_SYMBOL_QUERY_SRC).expect("Failed to compile JS symbol query")
});

const JS_CALL_QUERY_SRC: &str = r#"
;; Regular call: foo()
(call_expression
  function: (identifier) @call.name)

;; Method call: obj.method()
(call_expression
  function: (member_expression
    property: (property_identifier) @call.method))

;; new expression: new Foo()
(new_expression
  constructor: (identifier) @call.new)
"#;

static JS_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_javascript::LANGUAGE.into();
    Query::new(&language, JS_CALL_QUERY_SRC).expect("Failed to compile JS call query")
});

static JS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_javascript::LANGUAGE.into();
    Query::new(&language, JS_QUERY_SRC).expect("Failed to compile JavaScript query")
});

thread_local! {
    static JS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct JavaScriptParser;

fn parse_tree(source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
    JS_PARSER.with(|parser| {
        let mut p = parser.borrow_mut();
        let tree = p.parse(source, None);
        if tree.is_none() {
            tracing::warn!("Failed to parse JavaScript file: {}", path.display());
        }
        tree
    })
}

impl super::Parser for JavaScriptParser {
    fn lang(&self) -> Lang {
        Lang::JavaScript
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*JS_SYMBOL_QUERY;
        let fn_exported_idx = query.capture_index_for_name("fn.exported").unwrap();
        let fn_name_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_exported_idx = query.capture_index_for_name("class.exported").unwrap();
        let class_name_idx = query.capture_index_for_name("class.name").unwrap();
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

                let (kind, exported) = if capture.index == fn_exported_idx {
                    (SymbolKind::Function, true)
                } else if capture.index == fn_name_idx {
                    (SymbolKind::Function, false)
                } else if capture.index == class_exported_idx {
                    (SymbolKind::Class, true)
                } else if capture.index == class_name_idx {
                    (SymbolKind::Class, false)
                } else if capture.index == method_idx {
                    (SymbolKind::Method, false)
                } else {
                    continue;
                };

                let start = node.start_position();
                let end = node.end_position();

                symbols.push(Symbol {
                    name,
                    kind,
                    exported,
                    span: Span {
                        start_line: start.row + 1,
                        start_col: start.column,
                        end_line: end.row + 1,
                        end_col: end.column,
                    },
                });
            }
        }

        // Deduplicate: exported match may also match non-exported pattern
        let mut seen = std::collections::HashSet::new();
        symbols.retain(|s| seen.insert((s.name.clone(), s.span.start_line)));

        symbols
    }

    fn extract_calls(&self, source: &[u8], path: &Path) -> Vec<CallRef> {
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*JS_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();
        let new_idx = query.capture_index_for_name("call.new").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let callee_raw = if capture.index == name_idx || capture.index == new_idx {
                    match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == method_idx {
                    let member_node = node.parent().unwrap();
                    match member_node.utf8_text(source) {
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
        JS_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => {
                    tracing::warn!("Failed to parse JavaScript file: {}", path.display());
                    return Vec::new();
                }
            };

            let query = &*JS_QUERY;
            let mut cursor = QueryCursor::new();

            let fn_capture_idx = query
                .capture_index_for_name("_fn")
                .expect("_fn capture must exist");
            let path_capture_idx = query
                .capture_index_for_name("import.path")
                .expect("import.path capture must exist");

            let mut imports = Vec::new();
            let mut seen = std::collections::HashSet::new();

            let mut matches = cursor.matches(query, tree.root_node(), source);
            while let Some(m) = matches.next() {
                // Check if this match has a _fn capture (require pattern)
                let fn_capture = m.captures.iter().find(|c| c.index == fn_capture_idx);

                if let Some(fc) = fn_capture {
                    // This is the require() pattern — verify function name is "require"
                    let fn_name = match fc.node.utf8_text(source) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if fn_name != "require" {
                        continue;
                    }
                }

                // Extract the import path
                let path_capture = m.captures.iter().find(|c| c.index == path_capture_idx);

                if let Some(pc) = path_capture {
                    let node = pc.node;
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
            }

            imports
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        JavaScriptParser.parse(src.as_bytes(), Path::new("test.js"))
    }

    #[test]
    fn esm_import() {
        let imports = parse(r#"import { foo } from './utils';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn require_call() {
        let imports = parse(r#"const utils = require('./utils');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn require_external() {
        let imports = parse(r#"const express = require('express');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "express");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn dynamic_import() {
        let imports = parse(r#"const m = import('./lazy');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./lazy");
    }

    #[test]
    fn reexport() {
        let imports = parse(r#"export { foo } from './models';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./models");
    }

    #[test]
    fn ignores_non_require_calls() {
        let imports = parse(r#"const x = someFunc('./path');"#);
        // someFunc is not "require", so ./path should not be captured
        assert!(imports.is_empty());
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        JavaScriptParser.extract_symbols(src.as_bytes(), Path::new("test.js"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("function foo() {}\nfunction bar() {}\n");
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
        let syms = symbols("class MyClass {}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyClass");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols("class Svc {\n  get() {}\n  put() {}\n}\n");
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "get");
        assert_eq!(methods[1].name, "put");
    }

    #[test]
    fn symbols_exported_function() {
        let syms = symbols("export function main() {}\nfunction helper() {}\n");
        let main = syms.iter().find(|s| s.name == "main").unwrap();
        let helper = syms.iter().find(|s| s.name == "helper").unwrap();
        assert!(main.exported);
        assert!(!helper.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        JavaScriptParser.extract_calls(src.as_bytes(), Path::new("test.js"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("foo();\nbar(1, 2);\n");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].callee_raw, "foo");
        assert_eq!(c[1].callee_raw, "bar");
    }

    #[test]
    fn calls_method() {
        let c = calls("utils.normalize(data);\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "utils.normalize");
    }

    #[test]
    fn calls_new_expression() {
        let c = calls("const svc = new UserService();\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "UserService");
    }

    #[test]
    fn mixed_imports() {
        let imports = parse(
            r#"
import { a } from './a';
const b = require('./b');
export * from './c';
"#,
        );
        assert_eq!(imports.len(), 3);
    }
}

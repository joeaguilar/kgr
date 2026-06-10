use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const TS_QUERY_SRC: &str = r#"
;; Named/default import
(import_statement
  source: (string (string_fragment) @import.path))

;; TS CommonJS interop: import x = require('./y')
(import_statement
  (import_require_clause
    source: (string (string_fragment) @import.path)))

;; Re-export
(export_statement
  source: (string (string_fragment) @import.path))

;; Dynamic import (bare call_expression, not wrapped in await)
(call_expression
  function: (import)
  arguments: (arguments (string (string_fragment) @import.path)))

;; CommonJS require — we filter for "require" manually
(call_expression
  function: (identifier) @_fn
  arguments: (arguments (string (string_fragment) @import.path)))
"#;

const TS_SYMBOL_QUERY_SRC: &str = r#"
;; Exported function
(export_statement
  declaration: (function_declaration
    name: (identifier) @fn.exported))

;; Non-exported function
(program
  (function_declaration
    name: (identifier) @fn.name))

;; Exported generator function
(export_statement
  declaration: (generator_function_declaration
    name: (identifier) @fn.exported))

;; Non-exported generator function
(program
  (generator_function_declaration
    name: (identifier) @fn.name))

;; Exported const/let arrow or function expression
(export_statement
  declaration: (lexical_declaration
    (variable_declarator
      name: (identifier) @fn.exported
      value: [(arrow_function) (function_expression) (generator_function)])))

;; Non-exported const/let arrow or function expression
(program
  (lexical_declaration
    (variable_declarator
      name: (identifier) @fn.name
      value: [(arrow_function) (function_expression) (generator_function)])))

;; Exported class
(export_statement
  declaration: (class_declaration
    name: (type_identifier) @class.exported))

;; Non-exported class
(program
  (class_declaration
    name: (type_identifier) @class.name))

;; Exported abstract class
(export_statement
  declaration: (abstract_class_declaration
    name: (type_identifier) @class.exported))

;; Non-exported abstract class
(program
  (abstract_class_declaration
    name: (type_identifier) @class.name))

;; Method inside class
(class_declaration
  body: (class_body
    (method_definition
      name: (property_identifier) @method.name)))

;; Method inside abstract class
(abstract_class_declaration
  body: (class_body
    (method_definition
      name: (property_identifier) @method.name)))
"#;

static TS_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    Query::new(&language, TS_SYMBOL_QUERY_SRC).expect("Failed to compile TS symbol query")
});

static TSX_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    Query::new(&language, TS_SYMBOL_QUERY_SRC).expect("Failed to compile TSX symbol query")
});

const TS_CALL_QUERY_SRC: &str = r#"
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

;; Type annotation: x: MyType
(type_annotation (type_identifier) @type.ref)

;; Generic type: Array<MyType>
(generic_type (type_identifier) @type.generic)

;; Extends clause: class Foo extends Bar
(extends_clause (identifier) @type.extends)

;; Implements clause: class Foo implements Bar
(implements_clause (type_identifier) @type.implements)
"#;

static TS_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    Query::new(&language, TS_CALL_QUERY_SRC).expect("Failed to compile TS call query")
});

static TSX_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    Query::new(&language, TS_CALL_QUERY_SRC).expect("Failed to compile TSX call query")
});

static TS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    Query::new(&language, TS_QUERY_SRC).expect("Failed to compile TypeScript query")
});

static TSX_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    Query::new(&language, TS_QUERY_SRC).expect("Failed to compile TSX query")
});

thread_local! {
    static TS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        p
    });

    static TSX_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).unwrap();
        p
    });
}

pub struct TypeScriptParser;

fn is_tsx(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == "tsx")
}

fn parse_tree(source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
    if is_tsx(path) {
        TSX_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            p.parse(source, None)
        })
    } else {
        TS_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            p.parse(source, None)
        })
    }
}

impl super::Parser for TypeScriptParser {
    fn lang(&self) -> Lang {
        Lang::TypeScript
    }

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
    }

    /// Override the default so `.tsx` files are checked with the TSX grammar
    /// instead of the plain TypeScript grammar (valid JSX would otherwise be
    /// flagged as a syntax error).
    fn parse_errors(&self, source: &[u8], path: &Path) -> Vec<crate::types::ParseError> {
        match parse_tree(source, path) {
            Some(tree) => super::collect_parse_errors(&tree),
            None => Vec::new(),
        }
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = if is_tsx(path) {
            &*TSX_SYMBOL_QUERY
        } else {
            &*TS_SYMBOL_QUERY
        };

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

        let query = if is_tsx(path) {
            &*TSX_CALL_QUERY
        } else {
            &*TS_CALL_QUERY
        };

        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();
        let new_idx = query.capture_index_for_name("call.new").unwrap();
        let type_ref_idx = query.capture_index_for_name("type.ref").unwrap();
        let type_generic_idx = query.capture_index_for_name("type.generic").unwrap();
        let type_extends_idx = query.capture_index_for_name("type.extends").unwrap();
        let type_implements_idx = query.capture_index_for_name("type.implements").unwrap();

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
                } else if capture.index == type_ref_idx
                    || capture.index == type_generic_idx
                    || capture.index == type_extends_idx
                    || capture.index == type_implements_idx
                {
                    // Type reference
                    match node.utf8_text(source) {
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
        if is_tsx(path) {
            TSX_PARSER.with(|parser| parse_with(parser, source, path, &TSX_QUERY))
        } else {
            TS_PARSER.with(|parser| parse_with(parser, source, path, &TS_QUERY))
        }
    }
}

fn parse_with(
    parser: &RefCell<tree_sitter::Parser>,
    source: &[u8],
    path: &Path,
    query: &Query,
) -> Vec<Import> {
    let mut parser = parser.borrow_mut();
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            tracing::warn!("Failed to parse TypeScript file: {}", path.display());
            return Vec::new();
        }
    };

    let fn_capture_idx = query
        .capture_index_for_name("_fn")
        .expect("_fn capture must exist");
    let path_capture_idx = query
        .capture_index_for_name("import.path")
        .expect("import.path capture must exist");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let mut imports = Vec::new();
    let mut seen = std::collections::HashSet::new();

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse_ts(src: &str) -> Vec<Import> {
        TypeScriptParser.parse(src.as_bytes(), Path::new("test.ts"))
    }

    fn parse_tsx(src: &str) -> Vec<Import> {
        TypeScriptParser.parse(src.as_bytes(), Path::new("test.tsx"))
    }

    #[test]
    fn parse_errors_flags_malformed_source() {
        let errors = TypeScriptParser.parse_errors(b"function broken(:", Path::new("broken.ts"));
        assert!(
            !errors.is_empty(),
            "expected syntax errors for malformed typescript source"
        );
    }

    #[test]
    fn parse_errors_uses_tsx_grammar_for_tsx_paths() {
        let src = b"const x = <div>hello</div>;\n";
        let errors = TypeScriptParser.parse_errors(src, Path::new("comp.tsx"));
        assert!(
            errors.is_empty(),
            "valid JSX in a .tsx file should not be flagged as a syntax error"
        );
    }

    #[test]
    fn named_import() {
        let imports = parse_ts(r#"import { foo, bar } from './utils';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn default_import() {
        let imports = parse_ts(r#"import React from 'react';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "react");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn reexport() {
        let imports = parse_ts(r#"export { foo } from './models';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./models");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn dynamic_import() {
        let imports = parse_ts(r#"const m = import('./lazy');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./lazy");
    }

    #[test]
    fn tsx_import() {
        let imports = parse_tsx(r#"import { Component } from './Component';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./Component");
    }

    #[test]
    fn import_equals_require() {
        let imports = parse_ts(r#"import legacy = require('./legacy');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./legacy");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn import_equals_require_external() {
        let imports = parse_ts(r#"import express = require('express');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "express");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn require_call() {
        let imports = parse_ts(r#"const utils = require('./utils');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn require_call_external() {
        let imports = parse_ts(r#"const express = require('express');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "express");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn require_call_tsx() {
        let imports = parse_tsx(r#"const utils = require('./utils');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn ignores_non_require_calls() {
        let imports = parse_ts(r#"const x = someFunc('./path');"#);
        // someFunc is not "require", so ./path should not be captured
        assert!(imports.is_empty());
    }

    #[test]
    fn multiple_imports() {
        let imports = parse_ts(
            r#"
import { a } from './a';
import { b } from './b';
import React from 'react';
export * from './c';
"#,
        );
        assert_eq!(imports.len(), 4);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        TypeScriptParser.extract_symbols(src.as_bytes(), Path::new("test.ts"))
    }

    #[test]
    fn symbols_const_arrow_function() {
        let syms = symbols("const handler = () => {};\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "handler");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(!syms[0].exported);
    }

    #[test]
    fn symbols_const_function_expression() {
        let syms = symbols("const legacy = function () {};\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "legacy");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(!syms[0].exported);
    }

    #[test]
    fn symbols_exported_const_arrow() {
        let syms = symbols("export const arrowExported = () => {};\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "arrowExported");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(syms[0].exported);
    }

    #[test]
    fn symbols_generator_function() {
        let syms = symbols("function* genFn() { yield 1; }\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "genFn");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(!syms[0].exported);
    }

    #[test]
    fn symbols_exported_generator_function() {
        let syms = symbols("export function* genExported() { yield 1; }\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "genExported");
        assert!(syms[0].exported);
    }

    #[test]
    fn symbols_ignores_data_constants() {
        let syms = symbols("const x = 5;\nconst s = 'hi';\nexport const arr = [1, 2];\n");
        assert!(syms.is_empty());
    }

    #[test]
    fn symbols_abstract_class() {
        let syms = symbols("abstract class A {}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "A");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert!(!syms[0].exported);
    }

    #[test]
    fn symbols_exported_abstract_class() {
        let syms = symbols("export abstract class A {}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "A");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert!(syms[0].exported);
    }

    #[test]
    fn symbols_abstract_class_methods() {
        let syms = symbols("export abstract class Svc {\n  run(): void {}\n  stop(): void {}\n}\n");
        let class = syms.iter().find(|s| s.name == "Svc").unwrap();
        assert_eq!(class.kind, SymbolKind::Class);
        assert!(class.exported);
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "run");
        assert_eq!(methods[1].name, "stop");
    }

    #[test]
    fn symbols_tsx_const_arrow() {
        let syms = TypeScriptParser.extract_symbols(
            b"export const App = () => <div />;\n",
            Path::new("test.tsx"),
        );
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "App");
        assert!(syms[0].exported);
    }

    // ── Call / type-ref extraction tests ──────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        TypeScriptParser.extract_calls(src.as_bytes(), Path::new("test.ts"))
    }

    #[test]
    fn calls_type_annotations() {
        let c = calls("function foo(x: MyType): ReturnType { return x; }");
        let names: Vec<&str> = c.iter().map(|c| c.callee_raw.as_str()).collect();
        assert!(names.contains(&"MyType"));
        assert!(names.contains(&"ReturnType"));
    }

    #[test]
    fn calls_extends_class() {
        let c = calls("class Child extends Parent { }");
        let names: Vec<&str> = c.iter().map(|c| c.callee_raw.as_str()).collect();
        assert!(names.contains(&"Parent"));
    }
}

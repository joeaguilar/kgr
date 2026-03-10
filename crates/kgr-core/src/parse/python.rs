use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

static PYTHON_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_python::LANGUAGE.into();
    Query::new(&language, PYTHON_QUERY_SRC).expect("Failed to compile Python query")
});

const PYTHON_QUERY_SRC: &str = r#"
;; Simple import: import foo, import foo.bar
(import_statement
  name: (dotted_name) @import.path)

;; Aliased import: import foo as bar
(import_statement
  name: (aliased_import
    name: (dotted_name) @import.path))

;; From import: from foo import bar
(import_from_statement
  module_name: (dotted_name) @import.path)

;; Relative from import: from . import bar, from ..foo import bar
(import_from_statement
  module_name: (relative_import) @import.path)
"#;

static PYTHON_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_python::LANGUAGE.into();
    Query::new(&language, PYTHON_SYMBOL_QUERY_SRC).expect("Failed to compile Python symbol query")
});

const PYTHON_SYMBOL_QUERY_SRC: &str = r#"
;; Top-level function definition
(module
  (function_definition
    name: (identifier) @fn.name))

;; Class definition
(class_definition
  name: (identifier) @class.name)

;; Method inside class body
(class_definition
  body: (block
    (function_definition
      name: (identifier) @method.name)))
"#;

static PYTHON_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_python::LANGUAGE.into();
    Query::new(&language, PYTHON_CALL_QUERY_SRC).expect("Failed to compile Python call query")
});

const PYTHON_CALL_QUERY_SRC: &str = r#"
;; Simple call: foo()
(call
  function: (identifier) @call.name)

;; Attribute call: foo.bar()
(call
  function: (attribute
    attribute: (identifier) @call.attr))
"#;

thread_local! {
    static PYTHON_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct PythonParser;

impl PythonParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        PYTHON_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = parser.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Python file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for PythonParser {
    fn lang(&self) -> Lang {
        Lang::Python
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*PYTHON_SYMBOL_QUERY;
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

                let start = node.start_position();
                let end = node.end_position();

                symbols.push(Symbol {
                    exported: !name.starts_with('_'),
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

        let query = &*PYTHON_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let attr_idx = query.capture_index_for_name("call.attr").unwrap();

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
                } else if capture.index == attr_idx {
                    // Attribute call: get the full `a.b` text from the parent attribute node
                    let attr_node = node.parent().unwrap();
                    match attr_node.utf8_text(source) {
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
            let query = &*PYTHON_QUERY;
            let mut cursor = QueryCursor::new();

            let mut imports = Vec::new();
            let mut matches = cursor.matches(query, tree.root_node(), source);
            while let Some(m) = matches.next() {
                for capture in m.captures {
                    let node = capture.node;
                    let raw = match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    };

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

            // Deduplicate
            let mut seen = std::collections::HashSet::new();
            imports.retain(|i| seen.insert(i.raw.clone()));

            imports
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        PythonParser.parse(src.as_bytes(), Path::new("test.py"))
    }

    #[test]
    fn simple_import() {
        let imports = parse("import os");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "os");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn from_import() {
        let imports = parse("from os.path import join");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "os.path");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn relative_import() {
        let imports = parse("from . import utils");
        assert_eq!(imports.len(), 1);
        assert!(imports[0].raw.starts_with('.'));
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn relative_import_dotdot() {
        let imports = parse("from ..models import User");
        assert_eq!(imports.len(), 1);
        assert!(imports[0].raw.starts_with(".."));
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn aliased_import() {
        let imports = parse("import numpy as np");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "numpy");
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import os
import sys
from pathlib import Path
from . import utils
"#,
        );
        assert_eq!(imports.len(), 4);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        PythonParser.extract_symbols(src.as_bytes(), Path::new("test.py"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("def foo():\n    pass\n\ndef bar():\n    pass\n");
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
        let syms = symbols("class MyClass:\n    pass\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyClass");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols(
            "class Svc:\n    def get(self):\n        pass\n    def put(self):\n        pass\n",
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "get");
        assert_eq!(methods[1].name, "put");
    }

    #[test]
    fn symbols_exported_heuristic() {
        let syms = symbols("def public():\n    pass\n\ndef _private():\n    pass\n");
        let public = syms.iter().find(|s| s.name == "public").unwrap();
        let private = syms.iter().find(|s| s.name == "_private").unwrap();
        assert!(public.exported);
        assert!(!private.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        PythonParser.extract_calls(src.as_bytes(), Path::new("test.py"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("foo()\nbar(1, 2)\n");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].callee_raw, "foo");
        assert_eq!(c[1].callee_raw, "bar");
    }

    #[test]
    fn calls_attribute() {
        let c = calls("utils.normalize(data)\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "utils.normalize");
    }

    #[test]
    fn calls_class_instantiation() {
        let c = calls("svc = UserService()\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "UserService");
    }
}

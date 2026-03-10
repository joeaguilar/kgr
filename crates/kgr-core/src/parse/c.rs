use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const C_QUERY_SRC: &str = r#"
;; Local include: #include "file.h"
(preproc_include
  path: (string_literal) @import.local)

;; System include: #include <stdio.h>
(preproc_include
  path: (system_lib_string) @import.system)
"#;

static C_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_c::LANGUAGE.into();
    Query::new(&language, C_QUERY_SRC).expect("Failed to compile C query")
});

const C_SYMBOL_QUERY_SRC: &str = r#"
;; Function definition — name inside function_declarator
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @fn.name))

;; Pointer function: int *foo() — name inside pointer_declarator
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @fn.name)))

;; Struct definition
(struct_specifier
  name: (type_identifier) @class.name)

;; Enum definition
(enum_specifier
  name: (type_identifier) @class.name)
"#;

static C_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_c::LANGUAGE.into();
    Query::new(&language, C_SYMBOL_QUERY_SRC).expect("Failed to compile C symbol query")
});

const C_CALL_QUERY_SRC: &str = r#"
;; Function call: foo()
(call_expression
  function: (identifier) @call.name)

;; Member call (rare in C but possible via macros): obj.method() or ptr->method()
(call_expression
  function: (field_expression
    field: (field_identifier) @call.method))
"#;

static C_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_c::LANGUAGE.into();
    Query::new(&language, C_CALL_QUERY_SRC).expect("Failed to compile C call query")
});

thread_local! {
    static C_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct CParser;

impl CParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        C_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse C file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for CParser {
    fn lang(&self) -> Lang {
        Lang::C
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*C_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                let start = node.start_position();

                // Deduplicate by (name, start_line)
                if !seen.insert((name.clone(), start.row)) {
                    continue;
                }

                let kind = if capture.index == fn_idx {
                    SymbolKind::Function
                } else if capture.index == class_idx {
                    SymbolKind::Class
                } else {
                    continue;
                };

                // Determine exported status
                let exported = if kind == SymbolKind::Function {
                    // Walk up to the function_definition node and check for "static" storage class
                    let mut exported = true;
                    let mut parent = node.parent();
                    while let Some(p) = parent {
                        if p.kind() == "function_definition" {
                            let is_static = (0..p.child_count()).any(|i| {
                                let child = p.child(i).unwrap();
                                child.kind() == "storage_class_specifier"
                                    && child
                                        .utf8_text(source)
                                        .map(|t| t == "static")
                                        .unwrap_or(false)
                            });
                            exported = !is_static;
                            break;
                        }
                        parent = p.parent();
                    }
                    exported
                } else {
                    // Structs and enums are always visible in the translation unit
                    true
                };

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

        symbols
    }

    fn extract_calls(&self, source: &[u8], path: &Path) -> Vec<CallRef> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*C_CALL_QUERY;
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
                    // Member call: get the full field_expression text from parent
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
        C_PARSER.with(|parser| parse_c_like(parser, source, path, &C_QUERY))
    }
}

pub fn parse_c_like(
    parser: &RefCell<tree_sitter::Parser>,
    source: &[u8],
    path: &Path,
    query: &Query,
) -> Vec<Import> {
    let mut parser = parser.borrow_mut();
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            tracing::warn!("Failed to parse C/C++ file: {}", path.display());
            return Vec::new();
        }
    };

    let local_idx = query
        .capture_index_for_name("import.local")
        .expect("import.local capture must exist");
    let system_idx = query
        .capture_index_for_name("import.system")
        .expect("import.system capture must exist");

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

            // Strip quotes or angle brackets
            let raw = full_text
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string();

            if !seen.insert(raw.clone()) {
                continue;
            }

            let kind = if capture.index == local_idx {
                ImportKind::Local
            } else if capture.index == system_idx {
                ImportKind::System
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

    fn parse(src: &str) -> Vec<Import> {
        CParser.parse(src.as_bytes(), Path::new("test.c"))
    }

    #[test]
    fn local_include() {
        let imports = parse(r#"#include "myheader.h""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "myheader.h");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn system_include() {
        let imports = parse(r#"#include <stdio.h>"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "stdio.h");
        assert_eq!(imports[0].kind, ImportKind::System);
    }

    #[test]
    fn mixed_includes() {
        let imports = parse(
            r#"
#include <stdio.h>
#include <stdlib.h>
#include "mylib.h"
"#,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].kind, ImportKind::System);
        assert_eq!(imports[1].kind, ImportKind::System);
        assert_eq!(imports[2].kind, ImportKind::Local);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        CParser.extract_symbols(src.as_bytes(), Path::new("test.c"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("int foo() { return 0; }\nvoid bar() {}\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    #[test]
    fn symbols_finds_structs() {
        let syms = symbols("struct Point { int x; int y; };\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Point" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_enums() {
        let syms = symbols("enum Color { RED, GREEN, BLUE };\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_static_not_exported() {
        let syms = symbols("static int helper() { return 0; }\nint public_fn() { return 1; }\n");
        let helper = syms.iter().find(|s| s.name == "helper").unwrap();
        let public = syms.iter().find(|s| s.name == "public_fn").unwrap();
        assert!(!helper.exported);
        assert!(public.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        CParser.extract_calls(src.as_bytes(), Path::new("test.c"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("int main() { foo(); bar(1, 2); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
        assert!(c.iter().any(|c| c.callee_raw == "bar"));
    }
}

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
    declarator: (identifier) @fn.name)) @def

;; Pointer function: int *foo() — name inside pointer_declarator
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @fn.name))) @def

;; Struct definition — body required so forward declarations and
;; usage sites (e.g. `struct Point p;`) don't register symbols
(struct_specifier
  name: (type_identifier) @class.name
  body: (field_declaration_list)) @def

;; Enum definition — body required, same as struct
(enum_specifier
  name: (type_identifier) @class.name
  body: (enumerator_list)) @def

;; typedef struct { ... } Name — the dominant C struct-declaration idiom.
;; Body required so forward typedefs (`typedef struct Foo Foo;`) don't
;; register symbols. When the specifier also has a tag equal to the typedef
;; name (`typedef struct Bar { } Bar;`), extract_symbols skips the
;; declarator capture to avoid duplicates.
(type_definition
  type: (struct_specifier
    body: (field_declaration_list))
  declarator: (type_identifier) @class.name) @def

;; typedef enum { ... } Name
(type_definition
  type: (enum_specifier
    body: (enumerator_list))
  declarator: (type_identifier) @class.name) @def

;; typedef union { ... } Name
(type_definition
  type: (union_specifier
    body: (field_declaration_list))
  declarator: (type_identifier) @class.name) @def
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

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_c::LANGUAGE.into())
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*C_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let def_idx = query.capture_index_for_name("def").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            // Span comes from the enclosing definition node, name from the name node
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index == def_idx)
                .map(|c| c.node);
            for capture in m.captures {
                if capture.index == def_idx {
                    continue;
                }
                let node = capture.node;
                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                // typedef declarator that merely repeats the specifier tag
                // (`typedef struct Bar { ... } Bar;`) — the tag pattern
                // already captured it; skip so multi-line forms don't emit
                // duplicates (line-based dedup only covers single-line ones).
                if let Some(parent) = node.parent() {
                    if parent.kind() == "type_definition" {
                        if let Some(tag) = parent
                            .child_by_field_name("type")
                            .and_then(|spec| spec.child_by_field_name("name"))
                        {
                            if tag.utf8_text(source) == Ok(name.as_str()) {
                                continue;
                            }
                        }
                    }
                }

                let span_node = def_node.unwrap_or(node);
                let start = span_node.start_position();

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

                // Multi-declarator typedef (`typedef struct {...} A, B;`):
                // the shared type_definition span would make every alias
                // claim the whole statement, so each alias stops at its own
                // declarator instead. Single-declarator typedefs keep the
                // full span (the name sits on the closing line anyway).
                let end = if span_node.kind() == "type_definition" && {
                    let mut walk = span_node.walk();
                    span_node
                        .children_by_field_name("declarator", &mut walk)
                        .count()
                        > 1
                } {
                    node.end_position()
                } else {
                    span_node.end_position()
                };

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
    fn symbols_span_covers_full_definition() {
        let src = "int multi(int a) {\n    int b = a + 1;\n    int c = b * 2;\n    return c;\n}\n";
        let syms = symbols(src);
        let f = syms.iter().find(|s| s.name == "multi").unwrap();
        assert_eq!(f.span.start_line, 1);
        assert_eq!(f.span.end_line, 5);
    }

    #[test]
    fn symbols_struct_span_covers_body() {
        let src = "struct Point {\n    int x;\n    int y;\n};\n";
        let syms = symbols(src);
        let s = syms.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(s.span.start_line, 1);
        assert_eq!(s.span.end_line, 4);
    }

    #[test]
    fn symbols_typedef_span_covers_body() {
        let src = "typedef struct {\n    int a;\n    int b;\n} Foo;\n";
        let syms = symbols(src);
        let s = syms.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(s.span.start_line, 1);
        assert_eq!(s.span.end_line, 4);
    }

    #[test]
    fn symbols_multi_declarator_typedef_stops_at_each_alias() {
        // Both aliases share the struct body, but each span ends at its own
        // declarator instead of claiming the whole statement.
        let src = "typedef struct {\n    int a;\n} Foo,\n  Bar;\n";
        let syms = symbols(src);
        let foo = syms.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!((foo.span.start_line, foo.span.end_line), (1, 3));
        let bar = syms.iter().find(|s| s.name == "Bar").unwrap();
        assert_eq!((bar.span.start_line, bar.span.end_line), (1, 4));
    }

    #[test]
    fn symbols_static_not_exported() {
        let syms = symbols("static int helper() { return 0; }\nint public_fn() { return 1; }\n");
        let helper = syms.iter().find(|s| s.name == "helper").unwrap();
        let public = syms.iter().find(|s| s.name == "public_fn").unwrap();
        assert!(!helper.exported);
        assert!(public.exported);
    }

    #[test]
    fn symbols_ignores_bodyless_specifiers() {
        let syms =
            symbols("struct Point;\nstruct Point p;\nenum Color c;\nvoid f(struct Point *q) {}\n");
        assert!(!syms.iter().any(|s| s.kind == SymbolKind::Class));
        assert!(syms
            .iter()
            .any(|s| s.name == "f" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn symbols_definition_not_duplicated_by_usage() {
        let syms = symbols("struct Point { int x; };\nvoid f(struct Point *p) {}\n");
        let points: Vec<_> = syms
            .iter()
            .filter(|s| s.name == "Point" && s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].span.start_line, 1);
    }

    #[test]
    fn symbols_finds_anonymous_typedef_struct() {
        let syms = symbols("typedef struct { int a; } Foo;\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Foo" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_typedef_enum_and_union() {
        let syms = symbols(
            "typedef enum { RED, GREEN } Color;\ntypedef union { int i; float f; } Value;\n",
        );
        assert!(syms
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Class));
        assert!(syms
            .iter()
            .any(|s| s.name == "Value" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_named_typedef_struct_no_duplicates() {
        let syms = symbols("typedef struct Bar { int b; } Bar;\n");
        let bars: Vec<_> = syms
            .iter()
            .filter(|s| s.name == "Bar" && s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(bars.len(), 1);
    }

    #[test]
    fn symbols_named_multiline_typedef_struct_no_duplicates() {
        // Tag and declarator land on different lines, so line-based dedup
        // alone can't catch this — the tag-repeat guard must.
        let syms = symbols("typedef struct Node {\n  struct Node *next;\n} Node;\n");
        let nodes: Vec<_> = syms
            .iter()
            .filter(|s| s.name == "Node" && s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn symbols_typedef_with_distinct_tag_keeps_both() {
        // `struct BarTag` and `Bar` are both real type names.
        let syms = symbols("typedef struct BarTag { int b; } Bar;\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "BarTag" && s.kind == SymbolKind::Class));
        assert!(syms
            .iter()
            .any(|s| s.name == "Bar" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_forward_typedef_not_captured() {
        let syms = symbols("typedef struct Opaque Opaque;\n");
        assert!(!syms.iter().any(|s| s.kind == SymbolKind::Class));
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

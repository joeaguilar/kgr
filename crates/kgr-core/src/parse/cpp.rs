use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, Lang, Span, Symbol, SymbolKind};

// C++ uses the same include patterns as C
const CPP_QUERY_SRC: &str = r#"
;; Local include: #include "file.h"
(preproc_include
  path: (string_literal) @import.local)

;; System include: #include <iostream>
(preproc_include
  path: (system_lib_string) @import.system)
"#;

static CPP_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_cpp::LANGUAGE.into();
    Query::new(&language, CPP_QUERY_SRC).expect("Failed to compile C++ query")
});

const CPP_SYMBOL_QUERY_SRC: &str = r#"
;; Function definition
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @fn.name)) @def

;; Pointer function
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @fn.name))) @def

;; Qualified function (e.g., Foo::bar)
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (identifier) @method.name))) @def

;; Inline method defined in a class body (declarator is a field_identifier)
(function_definition
  declarator: (function_declarator
    declarator: (field_identifier) @method.name)) @def

;; Inline method returning a pointer
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (field_identifier) @method.name))) @def

;; Class definition — body required so forward declarations and
;; usage sites (e.g. `class Fwd;`, `struct Point p;`) don't register symbols
(class_specifier
  name: (type_identifier) @class.name
  body: (field_declaration_list)) @def

;; Struct definition — body required, same as class
(struct_specifier
  name: (type_identifier) @class.name
  body: (field_declaration_list)) @def

;; Enum definition — body required, same as class
(enum_specifier
  name: (type_identifier) @class.name
  body: (enumerator_list)) @def

;; Namespace definition
(namespace_definition
  name: (namespace_identifier) @class.name) @def
"#;

static CPP_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_cpp::LANGUAGE.into();
    Query::new(&language, CPP_SYMBOL_QUERY_SRC).expect("Failed to compile C++ symbol query")
});

const CPP_CALL_QUERY_SRC: &str = r#"
;; Function call: foo()
(call_expression
  function: (identifier) @call.name)

;; Method call: obj.method() or obj->method()
(call_expression
  function: (field_expression
    field: (field_identifier) @call.method))

;; Scoped call: Foo::bar() or std::sort()
(call_expression
  function: (qualified_identifier
    name: (identifier) @call.scoped))

;; Template function call: make_shared<int>() — capture the inner identifier.
;; Template arguments are stripped (CallRef is `make_shared`, not
;; `make_shared<int>`), consistent with the C# generic_name choice.
(call_expression
  function: (template_function
    name: (identifier) @call.name))

;; Scoped template call: std::make_unique<Widget>() — inner identifier,
;; scope and template arguments stripped
(call_expression
  function: (qualified_identifier
    name: (template_function
      name: (identifier) @call.scoped)))

;; new expression: new Foo()
(new_expression
  type: (type_identifier) @call.new)

;; Qualified new: new ns::Foo() — unqualified type name captured
(new_expression
  type: (qualified_identifier
    name: (type_identifier) @call.new))

;; Template new: new Bar<int>() — base type name, template args stripped
(new_expression
  type: (template_type
    name: (type_identifier) @call.new))

;; Qualified template new: new ns::Baz<int>()
(new_expression
  type: (qualified_identifier
    name: (template_type
      name: (type_identifier) @call.new)))
"#;

static CPP_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_cpp::LANGUAGE.into();
    Query::new(&language, CPP_CALL_QUERY_SRC).expect("Failed to compile C++ call query")
});

thread_local! {
    static CPP_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_cpp::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct CppParser;

impl CppParser {
    fn parse_tree(source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        CPP_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse C++ file: {}", path.display());
            }
            tree
        })
    }

    /// Check if a function_definition ancestor has a "static" storage class specifier.
    fn is_static_function(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_definition" {
                // Check children for storage_class_specifier with text "static"
                let child_count = parent.child_count();
                for i in 0..child_count {
                    if let Some(child) = parent.child(i) {
                        if child.kind() == "storage_class_specifier" {
                            if let Ok(text) = child.utf8_text(source) {
                                if text == "static" {
                                    return true;
                                }
                            }
                        }
                    }
                }
                return false;
            }
            current = parent.parent();
        }
        false
    }
}

impl super::Parser for CppParser {
    fn lang(&self) -> Lang {
        Lang::Cpp
    }

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_cpp::LANGUAGE.into())
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        CPP_PARSER.with(|parser| super::c::parse_c_like(parser, source, path, &CPP_QUERY))
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match Self::parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*CPP_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let method_idx = query.capture_index_for_name("method.name").unwrap();
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

                let kind = if capture.index == fn_idx {
                    SymbolKind::Function
                } else if capture.index == class_idx {
                    SymbolKind::Class
                } else if capture.index == method_idx {
                    SymbolKind::Method
                } else {
                    continue;
                };

                let span_node = def_node.unwrap_or(node);
                let start = span_node.start_position();
                let end = span_node.end_position();

                // Deduplicate by (name, start_line)
                let key = (name.clone(), start.row);
                if !seen.insert(key) {
                    continue;
                }

                let exported = match kind {
                    SymbolKind::Function | SymbolKind::Method => {
                        !Self::is_static_function(node, source)
                    }
                    _ => true, // classes, structs, enums, namespaces always exported
                };

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
        let tree = match Self::parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*CPP_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();
        let scoped_idx = query.capture_index_for_name("call.scoped").unwrap();
        let new_idx = query.capture_index_for_name("call.new").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let callee_raw = if capture.index == method_idx {
                    // Method call: get the full field_expression text (e.g., obj.method)
                    let field_expr = node.parent().unwrap();
                    match field_expr.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == name_idx
                    || capture.index == scoped_idx
                    || capture.index == new_idx
                {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;
    use crate::types::ImportKind;

    fn parse(src: &str) -> Vec<Import> {
        CppParser.parse(src.as_bytes(), Path::new("test.cpp"))
    }

    #[test]
    fn local_include() {
        let imports = parse(r#"#include "myclass.hpp""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "myclass.hpp");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn system_include() {
        let imports = parse(r#"#include <iostream>"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "iostream");
        assert_eq!(imports[0].kind, ImportKind::System);
    }

    #[test]
    fn mixed_cpp() {
        let imports = parse(
            r#"
#include <iostream>
#include <vector>
#include "myclass.hpp"
#include "utils/helper.h"
"#,
        );
        assert_eq!(imports.len(), 4);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        CppParser.extract_symbols(src.as_bytes(), Path::new("test.cpp"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("int foo() { return 0; }\nvoid bar() {}\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
    }

    #[test]
    fn symbols_finds_classes() {
        let syms = symbols("class MyClass { };\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "MyClass" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_structs() {
        let syms = symbols("struct Point { int x; int y; };\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Point" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_qualified_methods() {
        let syms = symbols("class Foo {};\nvoid Foo::bar() {}\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "bar" && s.kind == SymbolKind::Method));
    }

    #[test]
    fn symbols_finds_inline_methods() {
        let syms = symbols("class Foo {\npublic:\n  void inlineMethod() {}\n};\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "inlineMethod" && s.kind == SymbolKind::Method));
    }

    #[test]
    fn symbols_finds_inline_pointer_methods() {
        let syms = symbols("class Foo {\npublic:\n  int* getPtr() { return nullptr; }\n};\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "getPtr" && s.kind == SymbolKind::Method));
    }

    #[test]
    fn symbols_static_not_exported() {
        let syms = symbols("static int helper() { return 0; }\nint public_fn() { return 1; }\n");
        let helper = syms.iter().find(|s| s.name == "helper").unwrap();
        let public_fn = syms.iter().find(|s| s.name == "public_fn").unwrap();
        assert!(!helper.exported);
        assert!(public_fn.exported);
    }

    #[test]
    fn symbols_finds_enums_with_body() {
        let syms = symbols("enum Color { RED, GREEN };\nenum class Mode { A, B };\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Class));
        assert!(syms
            .iter()
            .any(|s| s.name == "Mode" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_ignores_forward_declarations() {
        let syms = symbols("class Fwd;\nstruct Point;\nenum Color : int;\n");
        assert!(!syms.iter().any(|s| s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_definition_not_duplicated_by_usage() {
        let syms = symbols(
            "struct Point { int x; };\nvoid f(struct Point *p) { struct Point q; (void)q; }\n",
        );
        let points: Vec<_> = syms
            .iter()
            .filter(|s| s.name == "Point" && s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].span.start_line, 1);
    }

    #[test]
    fn symbols_fn_span_covers_full_definition() {
        let src =
            "int add(int a, int b) {\n    int c = a + b;\n    int d = c * 2;\n    return d;\n}\n";
        let syms = symbols(src);
        let f = syms.iter().find(|s| s.name == "add").unwrap();
        assert_eq!(f.span.start_line, 1);
        assert_eq!(f.span.end_line, 5);
        assert!(f.span.end_line > f.span.start_line);
    }

    #[test]
    fn symbols_method_span_covers_full_definition() {
        let src = "class Foo {};\nvoid Foo::bar() {\n    int x = 1;\n    x += 1;\n}\n";
        let syms = symbols(src);
        let m = syms.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(m.kind, SymbolKind::Method);
        assert_eq!(m.span.start_line, 2);
        assert_eq!(m.span.end_line, 5);
    }

    #[test]
    fn symbols_inline_method_span_covers_body() {
        let src = "class Foo {\npublic:\n  void work() {\n    int x = 1;\n    x += 1;\n  }\n};\n";
        let syms = symbols(src);
        let m = syms.iter().find(|s| s.name == "work").unwrap();
        assert_eq!(m.kind, SymbolKind::Method);
        assert_eq!(m.span.start_line, 3);
        assert_eq!(m.span.end_line, 6);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        CppParser.extract_calls(src.as_bytes(), Path::new("test.cpp"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("int main() { foo(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
    }

    #[test]
    fn calls_method() {
        let c = calls("int main() { obj.process(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw.contains("process")));
    }

    #[test]
    fn calls_new_expression() {
        let c = calls("int main() { auto p = new Widget(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "Widget"));
    }

    #[test]
    fn calls_scoped() {
        let c = calls("int main() { std::sort(v.begin(), v.end()); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "sort"));
    }

    #[test]
    fn calls_scoped_template_function() {
        // std::make_unique<Widget>() — template args stripped: `make_unique`
        let c = calls("int main() { auto p = std::make_unique<Widget>(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "make_unique"));
    }

    #[test]
    fn calls_plain_template_function() {
        // make_shared<int>() — template args stripped: `make_shared`
        let c = calls("int main() { auto p = make_shared<int>(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "make_shared"));
    }

    #[test]
    fn calls_new_qualified() {
        let c = calls("int main() { auto p = new ns::Foo(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "Foo"));
    }

    #[test]
    fn calls_new_template_type() {
        let c = calls("int main() { auto p = new Bar<int>(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "Bar"));
    }

    #[test]
    fn calls_new_qualified_template_type() {
        let c = calls("int main() { auto p = new ns::Baz<int>(); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "Baz"));
    }

    #[test]
    fn calls_template_args_always_stripped() {
        // Decision: template arguments are never part of callee_raw,
        // consistent with the C# parser's generic_name handling.
        let c = calls(
            "int main() { auto a = std::make_unique<Widget>(); auto b = new Bar<int>(); return 0; }\n",
        );
        assert!(!c.is_empty());
        assert!(c.iter().all(|c| !c.callee_raw.contains('<')));
    }
}

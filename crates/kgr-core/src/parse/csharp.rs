use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const CSHARP_QUERY_SRC: &str = r#"
;; using directive: using System.IO;
(using_directive
  (qualified_name) @import.path)

;; using directive with identifier: using Foo;
(using_directive
  (identifier) @import.path)
"#;

static CSHARP_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_c_sharp::LANGUAGE.into();
    Query::new(&language, CSHARP_QUERY_SRC).expect("Failed to compile C# query")
});

static CSHARP_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_c_sharp::LANGUAGE.into();
    Query::new(&language, CSHARP_SYMBOL_QUERY_SRC).expect("Failed to compile C# symbol query")
});

const CSHARP_SYMBOL_QUERY_SRC: &str = r#"
;; Class declaration
(class_declaration
  name: (identifier) @class.name)

;; Interface declaration
(interface_declaration
  name: (identifier) @class.name)

;; Struct declaration
(struct_declaration
  name: (identifier) @class.name)

;; Enum declaration
(enum_declaration
  name: (identifier) @class.name)

;; Method declaration
(method_declaration
  name: (identifier) @method.name)

;; Constructor declaration
(constructor_declaration
  name: (identifier) @method.name)
"#;

static CSHARP_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_c_sharp::LANGUAGE.into();
    Query::new(&language, CSHARP_CALL_QUERY_SRC).expect("Failed to compile C# call query")
});

const CSHARP_CALL_QUERY_SRC: &str = r#"
;; Method invocation
(invocation_expression
  function: (identifier) @call.name)

;; Member access call: obj.Method()
(invocation_expression
  function: (member_access_expression
    name: (identifier) @call.method))

;; Object creation: new ClassName()
(object_creation_expression
  type: (identifier) @call.new)
"#;

thread_local! {
    static CSHARP_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct CSharpParser;

impl CSharpParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        CSHARP_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse C# file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for CSharpParser {
    fn lang(&self) -> Lang {
        Lang::CSharp
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*CSHARP_SYMBOL_QUERY;
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

                let kind = if capture.index == class_idx {
                    SymbolKind::Class
                } else if capture.index == method_idx {
                    SymbolKind::Method
                } else {
                    continue;
                };

                // Check if the declaration (parent of the name identifier) has a
                // "modifiers" child containing "public"
                let decl_node = node.parent().unwrap();
                let exported = (0..decl_node.child_count()).any(|i| {
                    let child = decl_node.child(i).unwrap();
                    child.kind() == "modifier"
                        && child
                            .utf8_text(source)
                            .map(|t| t.contains("public"))
                            .unwrap_or(false)
                });

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

        let query = &*CSHARP_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();
        let new_idx = query.capture_index_for_name("call.new").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                if capture.index != name_idx
                    && capture.index != method_idx
                    && capture.index != new_idx
                {
                    continue;
                }

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

        let query = &*CSHARP_QUERY;
        let mut cursor = QueryCursor::new();

        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let raw = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                if !seen.insert(raw.clone()) {
                    continue;
                }

                let start = node.start_position();
                let end = node.end_position();

                imports.push(Import {
                    raw,
                    kind: ImportKind::External,
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

    fn parse_csharp(src: &str) -> Vec<Import> {
        CSharpParser.parse(src.as_bytes(), Path::new("Test.cs"))
    }

    #[test]
    fn csharp_using_directive() {
        let imports = parse_csharp("using System.IO;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "System.IO");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn csharp_using_simple_identifier() {
        let imports = parse_csharp("using Foo;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Foo");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn csharp_multiple_usings() {
        let imports = parse_csharp(
            r#"
using System;
using System.Collections.Generic;
using Newtonsoft.Json;
"#,
        );
        assert_eq!(imports.len(), 3);
    }

    // -- Symbol extraction tests --

    fn symbols(src: &str) -> Vec<Symbol> {
        CSharpParser.extract_symbols(src.as_bytes(), Path::new("Test.cs"))
    }

    #[test]
    fn csharp_symbols_finds_classes() {
        let syms = symbols("class MyClass { }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyClass");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn csharp_symbols_finds_methods() {
        let syms = symbols("class Svc { void Get() {} void Put() {} }");
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn csharp_symbols_finds_interfaces() {
        let syms = symbols("interface IDrawable { void Draw(); }");
        assert!(syms
            .iter()
            .any(|s| s.name == "IDrawable" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn csharp_symbols_exported_public() {
        let syms = symbols("public class Pub {} class Priv {}");
        let pub_class = syms.iter().find(|s| s.name == "Pub").unwrap();
        let priv_class = syms.iter().find(|s| s.name == "Priv").unwrap();
        assert!(pub_class.exported);
        assert!(!priv_class.exported);
    }

    #[test]
    fn csharp_symbols_finds_structs() {
        let syms = symbols("struct Point { }");
        assert!(syms
            .iter()
            .any(|s| s.name == "Point" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn csharp_symbols_finds_enums() {
        let syms = symbols("enum Color { Red, Green, Blue }");
        assert!(syms
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Class));
    }

    // -- Call extraction tests --

    fn calls(src: &str) -> Vec<CallRef> {
        CSharpParser.extract_calls(src.as_bytes(), Path::new("Test.cs"))
    }

    #[test]
    fn csharp_calls_method_invocation() {
        let c = calls("class T { void F() { Foo(); } }");
        assert!(c.iter().any(|c| c.callee_raw == "Foo"));
    }

    #[test]
    fn csharp_calls_member_access() {
        let c = calls("class T { void F() { Console.WriteLine(); } }");
        assert!(c.iter().any(|c| c.callee_raw == "WriteLine"));
    }

    #[test]
    fn csharp_calls_object_creation() {
        let c = calls("class T { void F() { new List(); } }");
        assert!(c.iter().any(|c| c.callee_raw == "List"));
    }
}

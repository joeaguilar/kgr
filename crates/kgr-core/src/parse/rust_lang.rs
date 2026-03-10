use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const RUST_QUERY_SRC: &str = r#"
;; use declaration: use std::collections::HashMap;
(use_declaration
  argument: (_) @import.use)

;; mod declaration: mod foo;
(mod_item
  name: (identifier) @import.mod)

;; extern crate: extern crate serde;
(extern_crate_declaration
  name: (identifier) @import.extern)
"#;

const RUST_SYMBOL_QUERY_SRC: &str = r#"
;; Pub function (must come before non-pub so dedup keeps exported)
(function_item
  (visibility_modifier)
  name: (identifier) @fn.exported)

;; Top-level function (non-pub)
(function_item
  name: (identifier) @fn.name)

;; Pub struct
(struct_item
  (visibility_modifier)
  name: (type_identifier) @class.exported)

;; Struct (non-pub)
(struct_item
  name: (type_identifier) @class.name)

;; Pub enum
(enum_item
  (visibility_modifier)
  name: (type_identifier) @class.exported)

;; Enum (non-pub)
(enum_item
  name: (type_identifier) @class.name)

;; Pub trait
(trait_item
  (visibility_modifier)
  name: (type_identifier) @class.exported)

;; Trait (non-pub)
(trait_item
  name: (type_identifier) @class.name)

;; Method inside impl block
(impl_item
  body: (declaration_list
    (function_item
      name: (identifier) @method.name)))
"#;

const RUST_CALL_QUERY_SRC: &str = r#"
;; Regular call: foo()
(call_expression
  function: (identifier) @call.name)

;; Method call: obj.method()
(call_expression
  function: (field_expression
    field: (field_identifier) @call.method))

;; Macro invocation: println!()
(macro_invocation
  macro: (identifier) @call.macro)
"#;

static RUST_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    Query::new(&language, RUST_QUERY_SRC).expect("Failed to compile Rust query")
});

static RUST_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    Query::new(&language, RUST_SYMBOL_QUERY_SRC).expect("Failed to compile Rust symbol query")
});

static RUST_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    Query::new(&language, RUST_CALL_QUERY_SRC).expect("Failed to compile Rust call query")
});

thread_local! {
    static RUST_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct RustParser;

fn parse_tree(source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
    RUST_PARSER.with(|parser| {
        let mut p = parser.borrow_mut();
        let tree = p.parse(source, None);
        if tree.is_none() {
            tracing::warn!("Failed to parse Rust file: {}", path.display());
        }
        tree
    })
}

impl super::Parser for RustParser {
    fn lang(&self) -> Lang {
        Lang::Rust
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*RUST_SYMBOL_QUERY;
        let fn_name_idx = query.capture_index_for_name("fn.name").unwrap();
        let fn_exported_idx = query.capture_index_for_name("fn.exported").unwrap();
        let class_name_idx = query.capture_index_for_name("class.name").unwrap();
        let class_exported_idx = query.capture_index_for_name("class.exported").unwrap();
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

        let query = &*RUST_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();
        let macro_idx = query.capture_index_for_name("call.macro").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let callee_raw = if capture.index == name_idx || capture.index == macro_idx {
                    match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == method_idx {
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
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*RUST_QUERY;
        let use_idx = query.capture_index_for_name("import.use").unwrap();
        let mod_idx = query.capture_index_for_name("import.mod").unwrap();
        let _extern_idx = query.capture_index_for_name("import.extern").unwrap();

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

                // mod declarations with a body (mod foo { ... }) are inline, skip them
                if capture.index == mod_idx {
                    if let Some(parent) = node.parent() {
                        // If the mod_item has a body (declaration_list), it's inline
                        if parent.child_by_field_name("body").is_some() {
                            continue;
                        }
                    }
                }

                let kind = if capture.index == mod_idx {
                    ImportKind::Local
                } else if capture.index == use_idx {
                    // use crate:: or use super:: are local
                    if raw.starts_with("crate::")
                        || raw.starts_with("super::")
                        || raw.starts_with("self::")
                    {
                        ImportKind::Local
                    } else {
                        ImportKind::External
                    }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        RustParser.parse(src.as_bytes(), Path::new("lib.rs"))
    }

    #[test]
    fn use_external() {
        let imports = parse("use std::collections::HashMap;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn use_crate_local() {
        let imports = parse("use crate::utils::helper;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn mod_declaration() {
        let imports = parse("mod utils;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn extern_crate() {
        let imports = parse("extern crate serde;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "serde");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn inline_mod_ignored() {
        let imports = parse("mod tests { fn foo() {} }");
        // inline mod with body should be skipped
        assert!(imports.is_empty());
    }

    #[test]
    fn multiple() {
        let imports = parse(
            r#"
use std::io;
use crate::config::Settings;
mod parser;
extern crate log;
"#,
        );
        assert_eq!(imports.len(), 4);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        RustParser.extract_symbols(src.as_bytes(), Path::new("lib.rs"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("fn foo() {}\nfn bar() {}\n");
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
        let syms = symbols("struct MyStruct { x: i32 }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "MyStruct" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_enums() {
        let syms = symbols("enum Color { Red, Green, Blue }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_traits() {
        let syms = symbols("trait Drawable { fn draw(&self); }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_exported_pub() {
        let syms = symbols("pub fn public_fn() {}\nfn private_fn() {}\n");
        let public = syms.iter().find(|s| s.name == "public_fn").unwrap();
        let private = syms.iter().find(|s| s.name == "private_fn").unwrap();
        assert!(public.exported);
        assert!(!private.exported);
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols(
            "struct Foo;\nimpl Foo {\n    fn method_a(&self) {}\n    fn method_b(&self) {}\n}\n",
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        RustParser.extract_calls(src.as_bytes(), Path::new("lib.rs"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("fn main() { foo(); bar(1, 2); }\n");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
        assert!(c.iter().any(|c| c.callee_raw == "bar"));
    }

    #[test]
    fn calls_method() {
        let c = calls("fn main() { obj.process(); }\n");
        assert!(c.iter().any(|c| c.callee_raw.contains("process")));
    }

    #[test]
    fn calls_macro() {
        let c = calls("fn main() { println!(\"hello\"); vec![1,2,3]; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "println"));
        assert!(c.iter().any(|c| c.callee_raw == "vec"));
    }
}

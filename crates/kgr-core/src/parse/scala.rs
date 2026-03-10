use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const SCALA_QUERY_SRC: &str = r#"
;; import scala.collection.mutable
(import_declaration) @import.decl
"#;

static SCALA_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_scala::LANGUAGE.into();
    Query::new(&language, SCALA_QUERY_SRC).expect("Failed to compile Scala import query")
});

static SCALA_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_scala::LANGUAGE.into();
    Query::new(&language, SCALA_SYMBOL_QUERY_SRC).expect("Failed to compile Scala symbol query")
});

const SCALA_SYMBOL_QUERY_SRC: &str = r#"
;; Class definition
(class_definition
  name: (identifier) @class.name)

;; Object definition (singleton)
(object_definition
  name: (identifier) @class.name)

;; Trait definition
(trait_definition
  name: (identifier) @class.name)

;; Function definition (def)
(function_definition
  name: (identifier) @fn.name)
"#;

static SCALA_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_scala::LANGUAGE.into();
    Query::new(&language, SCALA_CALL_QUERY_SRC).expect("Failed to compile Scala call query")
});

const SCALA_CALL_QUERY_SRC: &str = r#"
;; Function/method call
(call_expression
  function: (identifier) @call.name)

;; Dot call: obj.method()
(call_expression
  function: (field_expression
    field: (identifier) @call.method))
"#;

thread_local! {
    static SCALA_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_scala::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct ScalaParser;

impl ScalaParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        SCALA_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Scala file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for ScalaParser {
    fn lang(&self) -> Lang {
        Lang::Scala
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*SCALA_SYMBOL_QUERY;
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();

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
                } else if capture.index == fn_idx {
                    SymbolKind::Function
                } else {
                    continue;
                };

                // Check if the declaration has a "modifiers" child containing "private"
                // Scala AST: (class_definition (modifiers (access_modifier)) name: ...)
                // If no modifier or modifier is not private → exported = true
                let decl_node = node.parent().unwrap();
                let exported = !(0..decl_node.child_count()).any(|i| {
                    let child = decl_node.child(i).unwrap();
                    child.kind() == "modifiers"
                        && child
                            .utf8_text(source)
                            .map(|t| t.contains("private"))
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

        let query = &*SCALA_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                if capture.index != name_idx && capture.index != method_idx {
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

        let query = &*SCALA_QUERY;
        let mut cursor = QueryCursor::new();

        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                // Extract path by stripping "import " prefix from declaration text
                let raw = match node.utf8_text(source) {
                    Ok(s) => s.strip_prefix("import ").unwrap_or(s).trim().to_string(),
                    Err(_) => continue,
                };

                if raw.is_empty() || !seen.insert(raw.clone()) {
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

    fn parse(src: &str) -> Vec<Import> {
        ScalaParser.parse(src.as_bytes(), Path::new("Test.scala"))
    }

    #[test]
    fn simple_import() {
        let imports = parse("import scala.collection.mutable");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "scala.collection.mutable");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import scala.collection.mutable
import java.util.List
import com.example.MyClass
"#,
        );
        assert_eq!(imports.len(), 3);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        ScalaParser.extract_symbols(src.as_bytes(), Path::new("Test.scala"))
    }

    #[test]
    fn symbols_finds_classes() {
        let syms = symbols("class MyClass {}");
        assert!(syms
            .iter()
            .any(|s| s.name == "MyClass" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_objects() {
        let syms = symbols("object MyObject {}");
        assert!(syms
            .iter()
            .any(|s| s.name == "MyObject" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_traits() {
        let syms = symbols("trait Drawable {}");
        assert!(syms
            .iter()
            .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("object Main { def hello(): Unit = {} }");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "hello");
    }

    #[test]
    fn symbols_exported_private() {
        let syms = symbols("class Pub {}\nprivate class Priv {}");
        let pub_class = syms.iter().find(|s| s.name == "Pub").unwrap();
        let priv_class = syms.iter().find(|s| s.name == "Priv");
        assert!(pub_class.exported);
        if let Some(priv_class) = priv_class {
            assert!(!priv_class.exported);
        }
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        ScalaParser.extract_calls(src.as_bytes(), Path::new("Test.scala"))
    }

    #[test]
    fn calls_function_invocation() {
        let c = calls("object T { def f(): Unit = { foo() } }");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
    }

    #[test]
    fn calls_method_invocation() {
        let c = calls("object T { def f(): Unit = { obj.method() } }");
        assert!(c.iter().any(|c| c.callee_raw == "method"));
    }
}

use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const JAVA_QUERY_SRC: &str = r#"
(import_declaration
  (scoped_identifier) @import.path)
"#;

static JAVA_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_java::LANGUAGE.into();
    Query::new(&language, JAVA_QUERY_SRC).expect("Failed to compile Java query")
});

static JAVA_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_java::LANGUAGE.into();
    Query::new(&language, JAVA_SYMBOL_QUERY_SRC).expect("Failed to compile Java symbol query")
});

const JAVA_SYMBOL_QUERY_SRC: &str = r#"
;; Class declaration
(class_declaration
  name: (identifier) @class.name)

;; Interface declaration
(interface_declaration
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

static JAVA_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_java::LANGUAGE.into();
    Query::new(&language, JAVA_CALL_QUERY_SRC).expect("Failed to compile Java call query")
});

const JAVA_CALL_QUERY_SRC: &str = r#"
;; Method invocation: foo() or obj.method()
(method_invocation
  name: (identifier) @call.name)

;; Object creation: new ClassName()
(object_creation_expression
  type: (type_identifier) @call.new)
"#;

thread_local! {
    static JAVA_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_java::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct JavaParser;

impl JavaParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        JAVA_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Java file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for JavaParser {
    fn lang(&self) -> Lang {
        Lang::Java
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*JAVA_SYMBOL_QUERY;
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
                    child.kind() == "modifiers"
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

        let query = &*JAVA_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let new_idx = query.capture_index_for_name("call.new").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                if capture.index != name_idx && capture.index != new_idx {
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

        let query = &*JAVA_QUERY;
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

    fn parse(src: &str) -> Vec<Import> {
        JavaParser.parse(src.as_bytes(), Path::new("Test.java"))
    }

    #[test]
    fn simple_import() {
        let imports = parse("import java.util.List;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "java.util.List");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn static_import() {
        let imports = parse("import static java.lang.Math.PI;");
        assert_eq!(imports.len(), 1);
        assert!(imports[0].raw.contains("java.lang.Math"));
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import java.util.List;
import java.util.Map;
import com.example.MyClass;
"#,
        );
        assert_eq!(imports.len(), 3);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        JavaParser.extract_symbols(src.as_bytes(), Path::new("Test.java"))
    }

    #[test]
    fn symbols_finds_classes() {
        let syms = symbols("class MyClass { }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyClass");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols("class Svc { void get() {} void put() {} }");
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn symbols_finds_interfaces() {
        let syms = symbols("interface Drawable { void draw(); }");
        assert!(syms
            .iter()
            .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_exported_public() {
        let syms = symbols("public class Pub {} class Priv {}");
        let pub_class = syms.iter().find(|s| s.name == "Pub").unwrap();
        let priv_class = syms.iter().find(|s| s.name == "Priv").unwrap();
        assert!(pub_class.exported);
        assert!(!priv_class.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        JavaParser.extract_calls(src.as_bytes(), Path::new("Test.java"))
    }

    #[test]
    fn calls_method_invocation() {
        let c = calls("class T { void f() { foo(); } }");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
    }

    #[test]
    fn calls_object_creation() {
        let c = calls("class T { void f() { new ArrayList(); } }");
        assert!(c.iter().any(|c| c.callee_raw == "ArrayList"));
    }
}

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
  name: (identifier) @class.name) @def

;; Interface declaration
(interface_declaration
  name: (identifier) @class.name) @def

;; Enum declaration
(enum_declaration
  name: (identifier) @class.name) @def

;; Record declaration (Java 16+)
(record_declaration
  name: (identifier) @class.name) @def

;; Method declaration
(method_declaration
  name: (identifier) @method.name) @def

;; Constructor declaration
(constructor_declaration
  name: (identifier) @method.name) @def
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

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_java::LANGUAGE.into())
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*JAVA_SYMBOL_QUERY;
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let method_idx = query.capture_index_for_name("method.name").unwrap();
        let def_idx = query.capture_index_for_name("def").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
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

                let kind = if capture.index == class_idx {
                    SymbolKind::Class
                } else if capture.index == method_idx {
                    SymbolKind::Method
                } else {
                    continue;
                };

                // Check if the declaration (parent of the name identifier) has a
                // "modifiers" child containing the given modifier keyword
                let decl_node = node.parent().unwrap();
                let has_modifier = |needle: &str| {
                    (0..decl_node.child_count()).any(|i| {
                        let child = decl_node.child(i).unwrap();
                        child.kind() == "modifiers"
                            && child
                                .utf8_text(source)
                                .map(|t| t.contains(needle))
                                .unwrap_or(false)
                    })
                };

                // Interface members are implicitly public unless explicitly
                // private (Java 9+ allows private interface methods).
                let in_interface_body = decl_node
                    .parent()
                    .is_some_and(|p| p.kind() == "interface_body");

                let exported = if in_interface_body {
                    !has_modifier("private")
                } else {
                    has_modifier("public")
                };

                let span_node = def_node.unwrap_or(node);
                let start = span_node.start_position();
                let end = span_node.end_position();

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
    fn symbols_span_covers_full_definition() {
        let src = "class Calc {\n    int add(int a, int b) {\n        int sum = a + b;\n        return sum;\n    }\n}\n";
        let syms = symbols(src);
        let class = syms.iter().find(|s| s.name == "Calc").unwrap();
        assert_eq!(class.span.start_line, 1);
        assert_eq!(class.span.end_line, 6);
    }

    #[test]
    fn symbols_method_span_covers_body() {
        let src = "class Calc {\n    int add(int a, int b) {\n        int sum = a + b;\n        return sum;\n    }\n}\n";
        let syms = symbols(src);
        let method = syms.iter().find(|s| s.name == "add").unwrap();
        assert_eq!(method.span.start_line, 2);
        assert_eq!(method.span.end_line, 5);
    }

    #[test]
    fn symbols_exported_public() {
        let syms = symbols("public class Pub {} class Priv {}");
        let pub_class = syms.iter().find(|s| s.name == "Pub").unwrap();
        let priv_class = syms.iter().find(|s| s.name == "Priv").unwrap();
        assert!(pub_class.exported);
        assert!(!priv_class.exported);
    }

    #[test]
    fn symbols_finds_records() {
        let syms = symbols("public record Point(int x, int y) {} record Pair(int a, int b) {}");
        let point = syms.iter().find(|s| s.name == "Point").unwrap();
        let pair = syms.iter().find(|s| s.name == "Pair").unwrap();
        assert_eq!(point.kind, SymbolKind::Class);
        assert!(point.exported);
        assert_eq!(pair.kind, SymbolKind::Class);
        assert!(!pair.exported);
    }

    #[test]
    fn interface_members_implicitly_public() {
        let syms = symbols(
            "interface D { void draw(); default void d() {} static void s() {} private void p() {} }",
        );
        let exported_of = |name: &str| syms.iter().find(|s| s.name == name).unwrap().exported;
        assert!(exported_of("draw"));
        assert!(exported_of("d"));
        assert!(exported_of("s"));
        assert!(!exported_of("p"));
        // The interface itself has no `public` modifier, so it stays
        // package-private.
        assert!(!exported_of("D"));
    }

    #[test]
    fn interface_members_no_duplicates() {
        let syms = symbols("interface D { void draw(); }");
        let draws = syms.iter().filter(|s| s.name == "draw").count();
        assert_eq!(draws, 1);
    }

    #[test]
    fn class_methods_still_require_public() {
        let syms = symbols("class C { void hidden() {} public void shown() {} }");
        let hidden = syms.iter().find(|s| s.name == "hidden").unwrap();
        let shown = syms.iter().find(|s| s.name == "shown").unwrap();
        assert!(!hidden.exported);
        assert!(shown.exported);
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

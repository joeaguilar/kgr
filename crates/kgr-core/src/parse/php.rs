use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const PHP_QUERY_SRC: &str = r#"
;; use statement: use App\Models\User;
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name) @import.path))

;; include
(expression_statement
  (include_expression
    (string (string_content) @import.path)))

;; require
(expression_statement
  (require_expression
    (string (string_content) @import.path)))
"#;

static PHP_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_php::LANGUAGE_PHP.into();
    Query::new(&language, PHP_QUERY_SRC).expect("Failed to compile PHP query")
});

const PHP_SYMBOL_QUERY_SRC: &str = r#"
;; Class declaration
(class_declaration
  name: (name) @class.name)

;; Interface declaration
(interface_declaration
  name: (name) @class.name)

;; Function definition
(function_definition
  name: (name) @fn.name)

;; Method declaration
(method_declaration
  name: (name) @method.name)
"#;

static PHP_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_php::LANGUAGE_PHP.into();
    Query::new(&language, PHP_SYMBOL_QUERY_SRC).expect("Failed to compile PHP symbol query")
});

const PHP_CALL_QUERY_SRC: &str = r#"
;; Function call
(function_call_expression
  function: (name) @call.name)

;; Method call
(member_call_expression
  name: (name) @call.method)

;; Object creation: new ClassName()
(object_creation_expression
  (name) @call.new)
"#;

static PHP_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_php::LANGUAGE_PHP.into();
    Query::new(&language, PHP_CALL_QUERY_SRC).expect("Failed to compile PHP call query")
});

thread_local! {
    static PHP_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_php::LANGUAGE_PHP.into()).unwrap();
        p
    });
}

pub struct PhpParser;

impl PhpParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        PHP_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse PHP file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for PhpParser {
    fn lang(&self) -> Lang {
        Lang::Php
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*PHP_SYMBOL_QUERY;
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
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
                } else if capture.index == fn_idx {
                    SymbolKind::Function
                } else if capture.index == method_idx {
                    SymbolKind::Method
                } else {
                    continue;
                };

                // Check if the declaration (parent of the name node) has a
                // visibility_modifier child containing "public"
                let decl_node = node.parent().unwrap();
                let exported = (0..decl_node.child_count()).any(|i| {
                    let child = decl_node.child(i).unwrap();
                    (child.kind() == "visibility_modifier" || child.kind() == "modifiers")
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

        let query = &*PHP_CALL_QUERY;
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

        let query = &*PHP_QUERY;
        let import_idx = query.capture_index_for_name("import.path").unwrap();
        let mut cursor = QueryCursor::new();

        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                if capture.index != import_idx {
                    continue;
                }
                let node = capture.node;
                let raw = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                if !seen.insert(raw.clone()) {
                    continue;
                }

                // Determine import kind:
                // - include/require with path starting with "." → Local
                // - namespace use statements → External
                // - everything else → External
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        PhpParser.parse(src.as_bytes(), Path::new("test.php"))
    }

    #[test]
    fn use_statement() {
        let imports = parse("<?php use App\\Models\\User; ?>");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "App\\Models\\User");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn multiple_use_statements() {
        let imports = parse(
            r#"<?php
use App\Models\User;
use App\Http\Controllers\Controller;
use Illuminate\Http\Request;
?>"#,
        );
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn include_require() {
        let imports = parse(
            r#"<?php
include './helpers.php';
require './config.php';
?>"#,
        );
        assert_eq!(imports.len(), 2);
        assert!(imports.iter().all(|i| i.kind == ImportKind::Local));
    }

    #[test]
    fn include_external() {
        let imports = parse("<?php include 'vendor/autoload.php'; ?>");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        PhpParser.extract_symbols(src.as_bytes(), Path::new("test.php"))
    }

    #[test]
    fn symbols_finds_classes() {
        let syms = symbols("<?php class MyClass { } ?>");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyClass");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn symbols_finds_interfaces() {
        let syms = symbols("<?php interface Drawable { public function draw(); } ?>");
        assert!(syms
            .iter()
            .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("<?php function helper() {} ?>");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "helper");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols("<?php class Svc { public function get() {} function put() {} } ?>");
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn symbols_exported_public() {
        let syms = symbols(
            "<?php class Svc { public function pub_m() {} private function priv_m() {} } ?>",
        );
        let pub_method = syms.iter().find(|s| s.name == "pub_m").unwrap();
        let priv_method = syms.iter().find(|s| s.name == "priv_m").unwrap();
        assert!(pub_method.exported);
        assert!(!priv_method.exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        PhpParser.extract_calls(src.as_bytes(), Path::new("test.php"))
    }

    #[test]
    fn calls_function() {
        let c = calls("<?php foo(); ?>");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
    }

    #[test]
    fn calls_method() {
        let c = calls("<?php $obj->method(); ?>");
        assert!(c.iter().any(|c| c.callee_raw == "method"));
    }

    #[test]
    fn calls_object_creation() {
        let c = calls("<?php new User(); ?>");
        assert!(c.iter().any(|c| c.callee_raw == "User"));
    }
}

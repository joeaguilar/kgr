use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const PHP_QUERY_SRC: &str = r#"
;; use statement: use App\Models\User; (anchor skips alias: (name) clauses)
(namespace_use_declaration
  (namespace_use_clause
    . (qualified_name) @import.path))

;; bare use statement: use Exception;
(namespace_use_declaration
  (namespace_use_clause
    . (name) @import.path))

;; grouped use: use App\{User, Post as P, Models\Thing};
(namespace_use_declaration
  (namespace_name) @import.group.prefix
  body: (namespace_use_group
    (namespace_use_clause
      . [(name) (qualified_name)] @import.group.member)))

;; include/require (+ _once variants), bare or parenthesized argument,
;; single-quoted (string) or literal double-quoted (encapsed_string).
;; Anchors on encapsed_string skip interpolated paths like "$dir/x.php".
(expression_statement
  [
    (include_expression
      [
        (string (string_content) @import.path)
        (encapsed_string . (string_content) @import.path .)
        (parenthesized_expression
          [
            (string (string_content) @import.path)
            (encapsed_string . (string_content) @import.path .)
          ])
      ])
    (include_once_expression
      [
        (string (string_content) @import.path)
        (encapsed_string . (string_content) @import.path .)
        (parenthesized_expression
          [
            (string (string_content) @import.path)
            (encapsed_string . (string_content) @import.path .)
          ])
      ])
    (require_expression
      [
        (string (string_content) @import.path)
        (encapsed_string . (string_content) @import.path .)
        (parenthesized_expression
          [
            (string (string_content) @import.path)
            (encapsed_string . (string_content) @import.path .)
          ])
      ])
    (require_once_expression
      [
        (string (string_content) @import.path)
        (encapsed_string . (string_content) @import.path .)
        (parenthesized_expression
          [
            (string (string_content) @import.path)
            (encapsed_string . (string_content) @import.path .)
          ])
      ])
  ])
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

;; Trait declaration
(trait_declaration
  name: (name) @class.name)

;; Enum declaration (PHP 8.1+)
(enum_declaration
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

;; Static call: Foo::bar() or \App\Foo::bar()
(scoped_call_expression
  scope: [(name) (qualified_name)] @call.static.scope
  name: (name) @call.static.name)

;; Object creation: new ClassName()
(object_creation_expression
  (name) @call.new)

;; Qualified object creation: new \App\ClassName() — capture final segment
(object_creation_expression
  (qualified_name (name) @call.new))
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

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_php::LANGUAGE_PHP.into())
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
        let static_scope_idx = query.capture_index_for_name("call.static.scope").unwrap();
        let static_name_idx = query.capture_index_for_name("call.static.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            // Static call: Foo::bar() — join scope and name as "Foo::bar"
            // (callee_matches treats :: as a qualifier separator).
            let scope = m.captures.iter().find(|c| c.index == static_scope_idx);
            let name = m.captures.iter().find(|c| c.index == static_name_idx);
            if let (Some(scope), Some(name)) = (scope, name) {
                let (Ok(scope_text), Ok(name_text)) =
                    (scope.node.utf8_text(source), name.node.utf8_text(source))
                else {
                    continue;
                };

                let start = scope.node.start_position();
                let end = name.node.end_position();

                calls.push(CallRef {
                    callee_raw: format!("{scope_text}::{name_text}"),
                    span: Span {
                        start_line: start.row + 1,
                        start_col: start.column,
                        end_line: end.row + 1,
                        end_col: end.column,
                    },
                });
                continue;
            }

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
        let group_prefix_idx = query.capture_index_for_name("import.group.prefix").unwrap();
        let group_member_idx = query.capture_index_for_name("import.group.member").unwrap();
        let mut cursor = QueryCursor::new();

        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            // Grouped use: use App\{User, Post}; — join prefix and member.
            let prefix = m.captures.iter().find(|c| c.index == group_prefix_idx);
            let member = m.captures.iter().find(|c| c.index == group_member_idx);
            if let (Some(prefix), Some(member)) = (prefix, member) {
                let (Ok(prefix_text), Ok(member_text)) =
                    (prefix.node.utf8_text(source), member.node.utf8_text(source))
                else {
                    continue;
                };
                let raw = format!("{prefix_text}\\{member_text}");
                if !seen.insert(raw.clone()) {
                    continue;
                }

                let start = member.node.start_position();
                let end = member.node.end_position();

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
                continue;
            }

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

    #[test]
    fn require_once_include_once() {
        let imports = parse(
            r#"<?php
require_once './config.php';
include_once './helpers.php';
?>"#,
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].raw, "./config.php");
        assert_eq!(imports[1].raw, "./helpers.php");
        assert!(imports.iter().all(|i| i.kind == ImportKind::Local));
    }

    #[test]
    fn parenthesized_require() {
        let imports = parse(
            r#"<?php
require('lib.php');
include_once('./extra.php');
?>"#,
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].raw, "lib.php");
        assert_eq!(imports[0].kind, ImportKind::External);
        assert_eq!(imports[1].raw, "./extra.php");
        assert_eq!(imports[1].kind, ImportKind::Local);
    }

    #[test]
    fn double_quoted_require() {
        let imports = parse("<?php require \"./x.php\"; ?>");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./x.php");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn interpolated_require_skipped() {
        // Interpolated paths cannot be resolved statically; emit nothing.
        let imports = parse("<?php require \"$dir/x.php\"; ?>");
        assert!(imports.is_empty());
    }

    #[test]
    fn bare_use() {
        let imports = parse("<?php use Exception; ?>");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Exception");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn bare_use_alias_captures_name_not_alias() {
        let imports = parse("<?php use Exception as E; ?>");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Exception");
    }

    #[test]
    fn aliased_qualified_use_captures_path_not_alias() {
        let imports = parse("<?php use App\\Models\\User as U; ?>");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "App\\Models\\User");
    }

    #[test]
    fn grouped_use() {
        let imports = parse("<?php use App\\{User, Post}; ?>");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, vec!["App\\User", "App\\Post"]);
        assert!(imports.iter().all(|i| i.kind == ImportKind::External));
    }

    #[test]
    fn grouped_use_with_alias() {
        let imports = parse("<?php use App\\{User as U, Post}; ?>");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, vec!["App\\User", "App\\Post"]);
    }

    #[test]
    fn grouped_use_multisegment_prefix() {
        let imports = parse("<?php use App\\Models\\{User, Post}; ?>");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, vec!["App\\Models\\User", "App\\Models\\Post"]);
    }

    #[test]
    fn grouped_use_qualified_member() {
        let imports = parse("<?php use App\\{Models\\User, Post}; ?>");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, vec!["App\\Models\\User", "App\\Post"]);
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

    #[test]
    fn calls_static() {
        let c = calls("<?php Foo::bar(); ?>");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "Foo::bar");
    }

    #[test]
    fn calls_static_qualified_scope() {
        let c = calls("<?php \\App\\Foo::bar(); ?>");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "\\App\\Foo::bar");
    }

    #[test]
    fn calls_qualified_object_creation() {
        let c = calls("<?php new \\App\\Qualified(); ?>");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "Qualified");
    }

    #[test]
    fn calls_relative_qualified_object_creation() {
        let c = calls("<?php new App\\Qualified(); ?>");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "Qualified");
    }

    // ── Trait / enum symbol tests ────────────────────────────────────────

    #[test]
    fn symbols_finds_traits() {
        let syms = symbols("<?php trait MyTrait { } ?>");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyTrait");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn symbols_finds_enums() {
        let syms = symbols("<?php enum MyEnum { case A; } ?>");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyEnum");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn symbols_finds_trait_methods() {
        let syms = symbols("<?php trait Greets { public function greet() {} } ?>");
        assert!(syms
            .iter()
            .any(|s| s.name == "Greets" && s.kind == SymbolKind::Class));
        let method = syms.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
        assert!(method.exported);
    }

    #[test]
    fn symbols_finds_enum_methods() {
        let syms = symbols("<?php enum Suit { case Hearts; public function color() {} } ?>");
        assert!(syms
            .iter()
            .any(|s| s.name == "Suit" && s.kind == SymbolKind::Class));
        let method = syms.iter().find(|s| s.name == "color").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
        assert!(method.exported);
    }
}

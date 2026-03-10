use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const SWIFT_QUERY_SRC: &str = r#"
;; import Foundation
(import_declaration
  (identifier) @import.path)
"#;

static SWIFT_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_swift::LANGUAGE.into();
    Query::new(&language, SWIFT_QUERY_SRC).expect("Failed to compile Swift import query")
});

const SWIFT_SYMBOL_QUERY_SRC: &str = r#"
;; Function declaration
(function_declaration
  name: (simple_identifier) @fn.name)

;; Class, struct, and enum declarations (all use class_declaration in tree-sitter-swift)
(class_declaration
  name: (type_identifier) @class.name)

;; Protocol declaration
(protocol_declaration
  name: (type_identifier) @protocol.name)
"#;

static SWIFT_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_swift::LANGUAGE.into();
    Query::new(&language, SWIFT_SYMBOL_QUERY_SRC).expect("Failed to compile Swift symbol query")
});

const SWIFT_CALL_QUERY_SRC: &str = r#"
;; Simple call: greet()
(call_expression
  (simple_identifier) @call.name)

;; Member call: obj.doThing()
(call_expression
  (navigation_expression
    suffix: (navigation_suffix
      suffix: (simple_identifier) @call.method)))
"#;

static SWIFT_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_swift::LANGUAGE.into();
    Query::new(&language, SWIFT_CALL_QUERY_SRC).expect("Failed to compile Swift call query")
});

thread_local! {
    static SWIFT_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_swift::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct SwiftParser;

impl SwiftParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        SWIFT_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Swift file: {}", path.display());
            }
            tree
        })
    }

    /// Check if a declaration node has a `public` or `open` access modifier.
    /// If no modifier is present, Swift defaults to `internal` which is visible
    /// within the module, so we treat it as exported.
    fn is_exported(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk siblings before the declaration node looking for modifiers
        // In tree-sitter-swift, modifiers are children of the declaration node
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "modifiers" || kind == "modifier" {
                if let Ok(text) = child.utf8_text(source) {
                    if text.contains("private") || text.contains("fileprivate") {
                        return false;
                    }
                    // public / open are explicitly exported
                    if text.contains("public") || text.contains("open") {
                        return true;
                    }
                }
            }
            // Also check direct visibility_modifier or attribute children
            if kind == "visibility_modifier" || kind == "access_level_modifier" {
                if let Ok(text) = child.utf8_text(source) {
                    if text.contains("private") || text.contains("fileprivate") {
                        return false;
                    }
                }
            }
        }
        // No explicit access modifier → internal (visible within module) → exported
        true
    }
}

impl super::Parser for SwiftParser {
    fn lang(&self) -> Lang {
        Lang::Swift
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*SWIFT_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let protocol_idx = query.capture_index_for_name("protocol.name").unwrap();

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

                let kind = if capture.index == fn_idx {
                    SymbolKind::Function
                } else if capture.index == class_idx || capture.index == protocol_idx {
                    SymbolKind::Class
                } else {
                    continue;
                };

                // The declaration node is the parent of the name identifier
                let decl_node = node.parent().unwrap_or(node);
                let exported = Self::is_exported(decl_node, source);

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

        let query = &*SWIFT_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let callee_raw = if capture.index == name_idx {
                    match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == method_idx {
                    // Get the full navigation expression text (e.g., `obj.doThing`)
                    // node is simple_identifier inside navigation_suffix inside navigation_expression
                    let nav_suffix = node.parent().unwrap();
                    let nav_expr = nav_suffix.parent().unwrap();
                    match nav_expr.utf8_text(source) {
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
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        {
            let query = &*SWIFT_QUERY;
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

                    // All Swift imports are external (framework/module imports)
                    let kind = ImportKind::External;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        SwiftParser.parse(src.as_bytes(), Path::new("main.swift"))
    }

    #[test]
    fn swift_import() {
        let imports = parse("import Foundation");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Foundation");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn swift_multiple_imports() {
        let imports = parse("import Foundation\nimport UIKit\nimport SwiftUI\n");
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw, "Foundation");
        assert_eq!(imports[1].raw, "UIKit");
        assert_eq!(imports[2].raw, "SwiftUI");
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        SwiftParser.extract_symbols(src.as_bytes(), Path::new("main.swift"))
    }

    #[test]
    fn swift_symbols_function() {
        let syms = symbols("func greet() { }\nfunc farewell() { }\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "greet");
        assert_eq!(fns[1].name, "farewell");
    }

    #[test]
    fn swift_symbols_class() {
        let syms = symbols("class UserService { }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "UserService" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn swift_symbols_struct() {
        let syms = symbols("struct Point { }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Point" && s.kind == SymbolKind::Class));
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        SwiftParser.extract_calls(src.as_bytes(), Path::new("main.swift"))
    }

    #[test]
    fn swift_calls_simple() {
        let c = calls("func main() { greet(); farewell() }\n");
        assert!(c.iter().any(|c| c.callee_raw == "greet"));
        assert!(c.iter().any(|c| c.callee_raw == "farewell"));
    }
}

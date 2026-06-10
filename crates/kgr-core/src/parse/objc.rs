use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const OBJC_QUERY_SRC: &str = r#"
;; #import "header.h" and #include "header.h" both parse as preproc_include
(preproc_include
  path: (string_literal) @import.local)

;; #import <Foundation/Foundation.h> and #include <stdio.h>
(preproc_include
  path: (system_lib_string) @import.system)

;; @import Foundation; and @import UIKit.UIView; — modern module imports.
;; Dotted submodule paths produce one path: (identifier) per segment, so we
;; capture the whole module_import node and join the segments in parse().
;; Classified as System for consistency with #import <Foundation/Foundation.h>.
(module_import) @import.module
"#;

static OBJC_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_objc::LANGUAGE.into();
    Query::new(&language, OBJC_QUERY_SRC).expect("Failed to compile Objective-C import query")
});

const OBJC_SYMBOL_QUERY_SRC: &str = r#"
;; C function definition
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @fn.name))

;; Objective-C class interface — identifier is a direct child, no name: field
(class_interface
  (identifier) @class.name)

;; Objective-C class implementation — identifier is a direct child
(class_implementation
  (identifier) @class.name)

;; Method definition — identifier is a direct child, no selector: field.
;; Multi-keyword selectors (doX:withY:) produce one bare identifier child per
;; segment; we anchor to the FIRST segment so each method_definition yields
;; exactly one symbol, named after its first selector segment (doX).
;; With an explicit return type, method_type is the first named child and the
;; first selector identifier immediately follows it:
(method_definition
  . (method_type)
  . (identifier) @method.name)

;; Without a return type (implicit id), the first selector identifier is the
;; first named child:
(method_definition
  . (identifier) @method.name)
"#;

static OBJC_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_objc::LANGUAGE.into();
    Query::new(&language, OBJC_SYMBOL_QUERY_SRC)
        .expect("Failed to compile Objective-C symbol query")
});

const OBJC_CALL_QUERY_SRC: &str = r#"
;; C function call
(call_expression
  function: (identifier) @call.name)

;; Message expression: [obj method] — uses method: field, not selector:
(message_expression
  method: (identifier) @call.name)
"#;

static OBJC_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_objc::LANGUAGE.into();
    Query::new(&language, OBJC_CALL_QUERY_SRC).expect("Failed to compile Objective-C call query")
});

thread_local! {
    static OBJC_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_objc::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct ObjCParser;

impl ObjCParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        OBJC_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Objective-C file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for ObjCParser {
    fn lang(&self) -> Lang {
        Lang::ObjectiveC
    }

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_objc::LANGUAGE.into())
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*OBJC_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let method_idx = query.capture_index_for_name("method.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                let start = node.start_position();

                // Deduplicate by (name, start_line)
                if !seen.insert((name.clone(), start.row)) {
                    continue;
                }

                let kind = if capture.index == fn_idx {
                    SymbolKind::Function
                } else if capture.index == class_idx {
                    SymbolKind::Class
                } else if capture.index == method_idx {
                    SymbolKind::Method
                } else {
                    continue;
                };

                // All Objective-C symbols are effectively public
                let exported = true;

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

        symbols
    }

    fn extract_calls(&self, source: &[u8], path: &Path) -> Vec<CallRef> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*OBJC_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                if capture.index != name_idx {
                    continue;
                }

                let node = capture.node;
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
        OBJC_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => {
                    tracing::warn!("Failed to parse Objective-C file: {}", path.display());
                    return Vec::new();
                }
            };

            let query = &*OBJC_QUERY;
            let local_idx = query
                .capture_index_for_name("import.local")
                .expect("import.local capture must exist");
            let system_idx = query
                .capture_index_for_name("import.system")
                .expect("import.system capture must exist");
            let module_idx = query
                .capture_index_for_name("import.module")
                .expect("import.module capture must exist");

            let mut cursor = QueryCursor::new();
            let mut imports = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut matches = cursor.matches(query, tree.root_node(), source);

            while let Some(m) = matches.next() {
                for capture in m.captures {
                    let node = capture.node;

                    let (raw, kind) = if capture.index == module_idx {
                        // @import UIKit.UIView; — one path: (identifier) per
                        // dotted segment; rejoin them into "UIKit.UIView".
                        // The anonymous "." tokens also carry the path field,
                        // so keep only the named identifier segments.
                        let mut walker = node.walk();
                        let segments: Vec<&str> = node
                            .children_by_field_name("path", &mut walker)
                            .filter(|c| c.is_named())
                            .filter_map(|c| c.utf8_text(source).ok())
                            .collect();
                        if segments.is_empty() {
                            continue;
                        }
                        // System, matching #import <Foundation/Foundation.h>.
                        (segments.join("."), ImportKind::System)
                    } else {
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

                        let kind = if capture.index == local_idx {
                            ImportKind::Local
                        } else if capture.index == system_idx {
                            ImportKind::System
                        } else {
                            ImportKind::External
                        };

                        (raw, kind)
                    };

                    if !seen.insert(raw.clone()) {
                        continue;
                    }

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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        ObjCParser.parse(src.as_bytes(), Path::new("test.m"))
    }

    #[test]
    fn import_directive() {
        let imports = parse(r#"#import "AppDelegate.h""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "AppDelegate.h");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn system_import() {
        let imports = parse(r#"#import <Foundation/Foundation.h>"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Foundation/Foundation.h");
        assert_eq!(imports[0].kind, ImportKind::System);
    }

    #[test]
    fn c_include_fallback() {
        let imports = parse(r#"#include "utils.h""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "utils.h");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn mixed_imports() {
        let imports = parse(
            r#"
#import <Foundation/Foundation.h>
#import "AppDelegate.h"
#include <stdio.h>
#include "helpers.h"
"#,
        );
        assert_eq!(imports.len(), 4);
        assert_eq!(imports[0].kind, ImportKind::System);
        assert_eq!(imports[1].kind, ImportKind::Local);
        assert_eq!(imports[2].kind, ImportKind::System);
        assert_eq!(imports[3].kind, ImportKind::Local);
    }

    #[test]
    fn module_import() {
        let imports = parse("@import Foundation;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Foundation");
        assert_eq!(imports[0].kind, ImportKind::System);
    }

    #[test]
    fn module_import_dotted_submodule() {
        let imports = parse("@import UIKit.UIView;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "UIKit.UIView");
        assert_eq!(imports[0].kind, ImportKind::System);
    }

    #[test]
    fn module_import_mixed_with_preproc_imports() {
        let imports = parse(
            r#"
@import Foundation;
#import <UIKit/UIKit.h>
#import "AppDelegate.h"
"#,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw, "Foundation");
        assert_eq!(imports[0].kind, ImportKind::System);
        assert_eq!(imports[1].raw, "UIKit/UIKit.h");
        assert_eq!(imports[1].kind, ImportKind::System);
        assert_eq!(imports[2].raw, "AppDelegate.h");
        assert_eq!(imports[2].kind, ImportKind::Local);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        ObjCParser.extract_symbols(src.as_bytes(), Path::new("test.m"))
    }

    #[test]
    fn class_interface_detection() {
        let syms = symbols(
            r#"
@interface MyClass : NSObject
@end
"#,
        );
        assert!(syms
            .iter()
            .any(|s| s.name == "MyClass" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn class_implementation_detection() {
        let syms = symbols(
            r#"
@implementation MyClass
@end
"#,
        );
        assert!(syms
            .iter()
            .any(|s| s.name == "MyClass" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_all_exported() {
        let syms = symbols(
            r#"
@interface Foo : NSObject
@end
"#,
        );
        for s in &syms {
            assert!(s.exported, "symbol {} should be exported", s.name);
        }
    }

    #[test]
    fn multi_keyword_method_emits_single_symbol() {
        let syms = symbols(
            r#"
@implementation Foo
- (void)doX:(int)x withY:(int)y {}
@end
"#,
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        // Exactly one symbol per method_definition, named after the FIRST
        // selector segment.
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "doX");
        assert!(!syms.iter().any(|s| s.name == "withY"));
    }

    #[test]
    fn single_keyword_method_detection() {
        let syms = symbols(
            r#"
@implementation Foo
- (void)setValue:(int)v {}
@end
"#,
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "setValue");
    }

    #[test]
    fn no_arg_method_detection() {
        let syms = symbols(
            r#"
@implementation Foo
- (void)reset {}
@end
"#,
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "reset");
    }

    #[test]
    fn untyped_return_method_detection() {
        // Implicit id return type: identifier is the first named child.
        let syms = symbols(
            r#"
@implementation Foo
- initWithName:(int)n andAge:(int)a {}
@end
"#,
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "initWithName");
    }

    #[test]
    fn class_method_detection() {
        let syms = symbols(
            r#"
@implementation Foo
+ (void)makeWithA:(int)a andB:(int)b {}
@end
"#,
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "makeWithA");
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        ObjCParser.extract_calls(src.as_bytes(), Path::new("test.m"))
    }

    #[test]
    fn c_function_calls() {
        let c = calls("int main() { printf(\"hello\"); NSLog(@\"world\"); return 0; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "printf"));
        assert!(c.iter().any(|c| c.callee_raw == "NSLog"));
    }
}

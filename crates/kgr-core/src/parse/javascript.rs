use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{Import, ImportKind, Lang, Span};

const JS_QUERY_SRC: &str = r#"
;; ESM import
(import_statement
  source: (string (string_fragment) @import.path))

;; Re-export
(export_statement
  source: (string (string_fragment) @import.path))

;; Dynamic import
(call_expression
  function: (import)
  arguments: (arguments (string (string_fragment) @import.path)))

;; CommonJS require — we filter for "require" manually
(call_expression
  function: (identifier) @_fn
  arguments: (arguments (string (string_fragment) @import.path)))
"#;

static JS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_javascript::LANGUAGE.into();
    Query::new(&language, JS_QUERY_SRC).expect("Failed to compile JavaScript query")
});

thread_local! {
    static JS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct JavaScriptParser;

impl super::Parser for JavaScriptParser {
    fn lang(&self) -> Lang {
        Lang::JavaScript
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        JS_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => {
                    tracing::warn!("Failed to parse JavaScript file: {}", path.display());
                    return Vec::new();
                }
            };

            let query = &*JS_QUERY;
            let mut cursor = QueryCursor::new();

            let fn_capture_idx = query
                .capture_index_for_name("_fn")
                .expect("_fn capture must exist");
            let path_capture_idx = query
                .capture_index_for_name("import.path")
                .expect("import.path capture must exist");

            let mut imports = Vec::new();
            let mut seen = std::collections::HashSet::new();

            let mut matches = cursor.matches(query, tree.root_node(), source);
            while let Some(m) = matches.next() {
                // Check if this match has a _fn capture (require pattern)
                let fn_capture = m.captures.iter().find(|c| c.index == fn_capture_idx);

                if let Some(fc) = fn_capture {
                    // This is the require() pattern — verify function name is "require"
                    let fn_name = match fc.node.utf8_text(source) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if fn_name != "require" {
                        continue;
                    }
                }

                // Extract the import path
                let path_capture = m.captures.iter().find(|c| c.index == path_capture_idx);

                if let Some(pc) = path_capture {
                    let node = pc.node;
                    let raw = match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    };

                    if !seen.insert(raw.clone()) {
                        continue;
                    }

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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        JavaScriptParser.parse(src.as_bytes(), Path::new("test.js"))
    }

    #[test]
    fn esm_import() {
        let imports = parse(r#"import { foo } from './utils';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn require_call() {
        let imports = parse(r#"const utils = require('./utils');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn require_external() {
        let imports = parse(r#"const express = require('express');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "express");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn dynamic_import() {
        let imports = parse(r#"const m = import('./lazy');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./lazy");
    }

    #[test]
    fn reexport() {
        let imports = parse(r#"export { foo } from './models';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./models");
    }

    #[test]
    fn ignores_non_require_calls() {
        let imports = parse(r#"const x = someFunc('./path');"#);
        // someFunc is not "require", so ./path should not be captured
        assert!(imports.is_empty());
    }

    #[test]
    fn mixed_imports() {
        let imports = parse(
            r#"
import { a } from './a';
const b = require('./b');
export * from './c';
"#,
        );
        assert_eq!(imports.len(), 3);
    }
}

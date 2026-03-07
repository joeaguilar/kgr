use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{Import, ImportKind, Lang, Span};

const TS_QUERY_SRC: &str = r#"
;; Named/default import
(import_statement
  source: (string (string_fragment) @import.path))

;; Re-export
(export_statement
  source: (string (string_fragment) @import.path))

;; Dynamic import (bare call_expression, not wrapped in await)
(call_expression
  function: (import)
  arguments: (arguments (string (string_fragment) @import.path)))
"#;

static TS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    Query::new(&language, TS_QUERY_SRC).expect("Failed to compile TypeScript query")
});

static TSX_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    Query::new(&language, TS_QUERY_SRC).expect("Failed to compile TSX query")
});

thread_local! {
    static TS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        p
    });

    static TSX_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).unwrap();
        p
    });
}

pub struct TypeScriptParser;

impl super::Parser for TypeScriptParser {
    fn lang(&self) -> Lang {
        Lang::TypeScript
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        let is_tsx = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "tsx")
            .unwrap_or(false);

        if is_tsx {
            TSX_PARSER.with(|parser| {
                parse_with(parser, source, path, &TSX_QUERY)
            })
        } else {
            TS_PARSER.with(|parser| {
                parse_with(parser, source, path, &TS_QUERY)
            })
        }
    }
}

fn parse_with(
    parser: &RefCell<tree_sitter::Parser>,
    source: &[u8],
    path: &Path,
    query: &Query,
) -> Vec<Import> {
    let mut parser = parser.borrow_mut();
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            tracing::warn!("Failed to parse TypeScript file: {}", path.display());
            return Vec::new();
        }
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let mut imports = Vec::new();
    let mut seen = std::collections::HashSet::new();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse_ts(src: &str) -> Vec<Import> {
        TypeScriptParser.parse(src.as_bytes(), Path::new("test.ts"))
    }

    fn parse_tsx(src: &str) -> Vec<Import> {
        TypeScriptParser.parse(src.as_bytes(), Path::new("test.tsx"))
    }

    #[test]
    fn named_import() {
        let imports = parse_ts(r#"import { foo, bar } from './utils';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn default_import() {
        let imports = parse_ts(r#"import React from 'react';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "react");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn reexport() {
        let imports = parse_ts(r#"export { foo } from './models';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./models");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn dynamic_import() {
        let imports = parse_ts(r#"const m = import('./lazy');"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./lazy");
    }

    #[test]
    fn tsx_import() {
        let imports = parse_tsx(r#"import { Component } from './Component';"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./Component");
    }

    #[test]
    fn multiple_imports() {
        let imports = parse_ts(
            r#"
import { a } from './a';
import { b } from './b';
import React from 'react';
export * from './c';
"#,
        );
        assert_eq!(imports.len(), 4);
    }
}

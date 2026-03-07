use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{Import, ImportKind, Lang, Span};

const GO_QUERY_SRC: &str = r#"
;; Single import
(import_declaration
  (import_spec
    path: (interpreted_string_literal) @import.path))

;; Import block
(import_declaration
  (import_spec_list
    (import_spec
      path: (interpreted_string_literal) @import.path)))
"#;

static GO_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_go::LANGUAGE.into();
    Query::new(&language, GO_QUERY_SRC).expect("Failed to compile Go query")
});

thread_local! {
    static GO_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_go::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct GoParser;

impl super::Parser for GoParser {
    fn lang(&self) -> Lang {
        Lang::Go
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        GO_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => {
                    tracing::warn!("Failed to parse Go file: {}", path.display());
                    return Vec::new();
                }
            };

            let query = &*GO_QUERY;
            let mut cursor = QueryCursor::new();

            let mut imports = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut matches = cursor.matches(query, tree.root_node(), source);

            while let Some(m) = matches.next() {
                for capture in m.captures {
                    let node = capture.node;
                    let full_text = match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    };

                    // Strip surrounding quotes
                    let raw = full_text
                        .trim_start_matches('"')
                        .trim_end_matches('"')
                        .to_string();

                    if !seen.insert(raw.clone()) {
                        continue;
                    }

                    // Go imports: relative paths (starting with ./) are local,
                    // everything else is external (module paths)
                    let kind = if raw.starts_with("./") || raw.starts_with("../") {
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
        GoParser.parse(src.as_bytes(), Path::new("main.go"))
    }

    #[test]
    fn single_import() {
        let imports = parse(r#"import "fmt""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "fmt");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn import_block() {
        let imports = parse(
            r#"
import (
    "fmt"
    "os"
    "net/http"
)
"#,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw, "fmt");
        assert_eq!(imports[1].raw, "os");
        assert_eq!(imports[2].raw, "net/http");
    }

    #[test]
    fn relative_import() {
        let imports = parse(r#"import "./utils""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn module_import() {
        let imports = parse(r#"import "github.com/user/repo/pkg""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "github.com/user/repo/pkg");
        assert_eq!(imports[0].kind, ImportKind::External);
    }
}

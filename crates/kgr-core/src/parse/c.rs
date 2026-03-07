use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{Import, ImportKind, Lang, Span};

const C_QUERY_SRC: &str = r#"
;; Local include: #include "file.h"
(preproc_include
  path: (string_literal) @import.local)

;; System include: #include <stdio.h>
(preproc_include
  path: (system_lib_string) @import.system)
"#;

static C_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_c::LANGUAGE.into();
    Query::new(&language, C_QUERY_SRC).expect("Failed to compile C query")
});

thread_local! {
    static C_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct CParser;

impl super::Parser for CParser {
    fn lang(&self) -> Lang {
        Lang::C
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        C_PARSER.with(|parser| parse_c_like(parser, source, path, &C_QUERY))
    }
}

pub fn parse_c_like(
    parser: &RefCell<tree_sitter::Parser>,
    source: &[u8],
    path: &Path,
    query: &Query,
) -> Vec<Import> {
    let mut parser = parser.borrow_mut();
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            tracing::warn!("Failed to parse C/C++ file: {}", path.display());
            return Vec::new();
        }
    };

    let local_idx = query
        .capture_index_for_name("import.local")
        .expect("import.local capture must exist");
    let system_idx = query
        .capture_index_for_name("import.system")
        .expect("import.system capture must exist");

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

            // Strip quotes or angle brackets
            let raw = full_text
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string();

            if !seen.insert(raw.clone()) {
                continue;
            }

            let kind = if capture.index == local_idx {
                ImportKind::Local
            } else if capture.index == system_idx {
                ImportKind::System
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

    fn parse(src: &str) -> Vec<Import> {
        CParser.parse(src.as_bytes(), Path::new("test.c"))
    }

    #[test]
    fn local_include() {
        let imports = parse(r#"#include "myheader.h""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "myheader.h");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn system_include() {
        let imports = parse(r#"#include <stdio.h>"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "stdio.h");
        assert_eq!(imports[0].kind, ImportKind::System);
    }

    #[test]
    fn mixed_includes() {
        let imports = parse(
            r#"
#include <stdio.h>
#include <stdlib.h>
#include "mylib.h"
"#,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].kind, ImportKind::System);
        assert_eq!(imports[1].kind, ImportKind::System);
        assert_eq!(imports[2].kind, ImportKind::Local);
    }
}

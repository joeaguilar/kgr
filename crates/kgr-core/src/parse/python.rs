use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{Import, ImportKind, Lang, Span};

static PYTHON_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_python::LANGUAGE.into();
    Query::new(&language, PYTHON_QUERY_SRC).expect("Failed to compile Python query")
});

const PYTHON_QUERY_SRC: &str = r#"
;; Simple import: import foo, import foo.bar
(import_statement
  name: (dotted_name) @import.path)

;; Aliased import: import foo as bar
(import_statement
  name: (aliased_import
    name: (dotted_name) @import.path))

;; From import: from foo import bar
(import_from_statement
  module_name: (dotted_name) @import.path)

;; Relative from import: from . import bar, from ..foo import bar
(import_from_statement
  module_name: (relative_import) @import.path)
"#;

thread_local! {
    static PYTHON_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct PythonParser;

impl super::Parser for PythonParser {
    fn lang(&self) -> Lang {
        Lang::Python
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        PYTHON_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => {
                    tracing::warn!("Failed to parse Python file: {}", path.display());
                    return Vec::new();
                }
            };

            let query = &*PYTHON_QUERY;
            let mut cursor = QueryCursor::new();

            let mut imports = Vec::new();
            let mut matches = cursor.matches(query, tree.root_node(), source);
            while let Some(m) = matches.next() {
                for capture in m.captures {
                    let node = capture.node;
                    let raw = match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    };

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

            // Deduplicate
            let mut seen = std::collections::HashSet::new();
            imports.retain(|i| seen.insert(i.raw.clone()));

            imports
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        PythonParser.parse(src.as_bytes(), Path::new("test.py"))
    }

    #[test]
    fn simple_import() {
        let imports = parse("import os");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "os");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn from_import() {
        let imports = parse("from os.path import join");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "os.path");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn relative_import() {
        let imports = parse("from . import utils");
        assert_eq!(imports.len(), 1);
        assert!(imports[0].raw.starts_with('.'));
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn relative_import_dotdot() {
        let imports = parse("from ..models import User");
        assert_eq!(imports.len(), 1);
        assert!(imports[0].raw.starts_with(".."));
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn aliased_import() {
        let imports = parse("import numpy as np");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "numpy");
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import os
import sys
from pathlib import Path
from . import utils
"#,
        );
        assert_eq!(imports.len(), 4);
    }
}

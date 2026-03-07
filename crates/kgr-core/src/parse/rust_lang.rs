use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{Import, ImportKind, Lang, Span};

const RUST_QUERY_SRC: &str = r#"
;; use declaration: use std::collections::HashMap;
(use_declaration
  argument: (_) @import.use)

;; mod declaration: mod foo;
(mod_item
  name: (identifier) @import.mod)

;; extern crate: extern crate serde;
(extern_crate_declaration
  name: (identifier) @import.extern)
"#;

static RUST_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    Query::new(&language, RUST_QUERY_SRC).expect("Failed to compile Rust query")
});

thread_local! {
    static RUST_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct RustParser;

impl super::Parser for RustParser {
    fn lang(&self) -> Lang {
        Lang::Rust
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        RUST_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => {
                    tracing::warn!("Failed to parse Rust file: {}", path.display());
                    return Vec::new();
                }
            };

            let query = &*RUST_QUERY;
            let use_idx = query.capture_index_for_name("import.use").unwrap();
            let mod_idx = query.capture_index_for_name("import.mod").unwrap();
            let _extern_idx = query.capture_index_for_name("import.extern").unwrap();

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

                    // mod declarations with a body (mod foo { ... }) are inline, skip them
                    if capture.index == mod_idx {
                        if let Some(parent) = node.parent() {
                            // If the mod_item has a body (declaration_list), it's inline
                            if parent.child_by_field_name("body").is_some() {
                                continue;
                            }
                        }
                    }

                    let kind = if capture.index == mod_idx {
                        ImportKind::Local
                    } else if capture.index == use_idx {
                        // use crate:: or use super:: are local
                        if raw.starts_with("crate::") || raw.starts_with("super::") || raw.starts_with("self::") {
                            ImportKind::Local
                        } else {
                            ImportKind::External
                        }
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
        RustParser.parse(src.as_bytes(), Path::new("lib.rs"))
    }

    #[test]
    fn use_external() {
        let imports = parse("use std::collections::HashMap;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn use_crate_local() {
        let imports = parse("use crate::utils::helper;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn mod_declaration() {
        let imports = parse("mod utils;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn extern_crate() {
        let imports = parse("extern crate serde;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "serde");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn inline_mod_ignored() {
        let imports = parse("mod tests { fn foo() {} }");
        // inline mod with body should be skipped
        assert!(imports.is_empty());
    }

    #[test]
    fn multiple() {
        let imports = parse(
            r#"
use std::io;
use crate::config::Settings;
mod parser;
extern crate log;
"#,
        );
        assert_eq!(imports.len(), 4);
    }
}

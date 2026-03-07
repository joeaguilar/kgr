use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{Import, ImportKind, Lang, Span};

const JAVA_QUERY_SRC: &str = r#"
(import_declaration
  (scoped_identifier) @import.path)
"#;

static JAVA_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_java::LANGUAGE.into();
    Query::new(&language, JAVA_QUERY_SRC).expect("Failed to compile Java query")
});

thread_local! {
    static JAVA_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_java::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct JavaParser;

impl super::Parser for JavaParser {
    fn lang(&self) -> Lang {
        Lang::Java
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        JAVA_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => {
                    tracing::warn!("Failed to parse Java file: {}", path.display());
                    return Vec::new();
                }
            };

            let query = &*JAVA_QUERY;
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

                    let start = node.start_position();
                    let end = node.end_position();

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
        JavaParser.parse(src.as_bytes(), Path::new("Test.java"))
    }

    #[test]
    fn simple_import() {
        let imports = parse("import java.util.List;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "java.util.List");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn static_import() {
        let imports = parse("import static java.lang.Math.PI;");
        assert_eq!(imports.len(), 1);
        assert!(imports[0].raw.contains("java.lang.Math"));
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import java.util.List;
import java.util.Map;
import com.example.MyClass;
"#,
        );
        assert_eq!(imports.len(), 3);
    }
}

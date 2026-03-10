use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

static BASH_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_bash::LANGUAGE.into();
    Query::new(&language, BASH_QUERY_SRC).expect("Failed to compile Bash query")
});

const BASH_QUERY_SRC: &str = r#"
;; source ./file.sh or . ./file.sh
(command
  name: (command_name) @_cmd
  argument: (word) @import.path)

(command
  name: (command_name) @_cmd2
  argument: (string) @import.path_str)

(command
  name: (command_name) @_cmd3
  argument: (raw_string) @import.path_raw)
"#;

static BASH_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_bash::LANGUAGE.into();
    Query::new(&language, BASH_SYMBOL_QUERY_SRC).expect("Failed to compile Bash symbol query")
});

const BASH_SYMBOL_QUERY_SRC: &str = r#"
;; Function definition: function foo { ... } or foo() { ... }
(function_definition
  name: (word) @fn.name)
"#;

static BASH_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_bash::LANGUAGE.into();
    Query::new(&language, BASH_CALL_QUERY_SRC).expect("Failed to compile Bash call query")
});

const BASH_CALL_QUERY_SRC: &str = r#"
;; Command/function call
(command
  name: (command_name) @call.name)
"#;

thread_local! {
    static BASH_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_bash::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct BashParser;

impl BashParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        BASH_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = parser.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Bash file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for BashParser {
    fn lang(&self) -> Lang {
        Lang::Bash
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*BASH_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                if capture.index != fn_idx {
                    continue;
                }
                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                let start = node.start_position();
                let end = node.end_position();

                symbols.push(Symbol {
                    exported: true,
                    name,
                    kind: SymbolKind::Function,
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

        let query = &*BASH_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                if capture.index != name_idx {
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

        let query = &*BASH_QUERY;
        let cmd_idx = query.capture_index_for_name("_cmd");
        let cmd2_idx = query.capture_index_for_name("_cmd2");
        let cmd3_idx = query.capture_index_for_name("_cmd3");
        let path_idx = query.capture_index_for_name("import.path");
        let path_str_idx = query.capture_index_for_name("import.path_str");
        let path_raw_idx = query.capture_index_for_name("import.path_raw");

        let mut cursor = QueryCursor::new();
        let mut imports = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            // Check if this is a source/dot command
            let cmd_text = m.captures.iter().find_map(|c| {
                if Some(c.index) == cmd_idx
                    || Some(c.index) == cmd2_idx
                    || Some(c.index) == cmd3_idx
                {
                    c.node.utf8_text(source).ok()
                } else {
                    None
                }
            });

            let cmd = match cmd_text {
                Some(t) => t,
                None => continue,
            };

            if cmd != "source" && cmd != "." {
                continue;
            }

            // Find the path capture
            let path_capture = m.captures.iter().find(|c| {
                Some(c.index) == path_idx
                    || Some(c.index) == path_str_idx
                    || Some(c.index) == path_raw_idx
            });

            let capture = match path_capture {
                Some(c) => c,
                None => continue,
            };

            let node = capture.node;
            let mut raw = match node.utf8_text(source) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };

            // Strip quotes from string/raw_string arguments
            if (raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\''))
            {
                raw = raw[1..raw.len() - 1].to_string();
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

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        imports.retain(|i| seen.insert(i.raw.clone()));

        imports
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        BashParser.parse(src.as_bytes(), Path::new("test.sh"))
    }

    #[test]
    fn source_directive() {
        let imports = parse("source ./lib.sh");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./lib.sh");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn dot_source_directive() {
        let imports = parse(". ./utils.sh");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./utils.sh");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn source_external() {
        let imports = parse("source /etc/profile");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "/etc/profile");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn source_not_matched_for_other_commands() {
        let imports = parse("echo hello\nls -la\n");
        assert_eq!(imports.len(), 0);
    }

    #[test]
    fn multiple_sources() {
        let imports = parse("source ./a.sh\n. ./b.sh\nsource /etc/profile\n");
        assert_eq!(imports.len(), 3);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        BashParser.extract_symbols(src.as_bytes(), Path::new("test.sh"))
    }

    #[test]
    fn function_definition() {
        let syms = symbols("function greet {\n  echo hello\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(syms[0].exported);
    }

    #[test]
    fn function_definition_parens() {
        let syms = symbols("greet() {\n  echo hello\n}\n");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(syms[0].exported);
    }

    #[test]
    fn multiple_functions() {
        let syms = symbols("foo() { :; }\nbar() { :; }\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        BashParser.extract_calls(src.as_bytes(), Path::new("test.sh"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("echo hello\nls -la\n");
        assert!(c.len() >= 2);
        assert_eq!(c[0].callee_raw, "echo");
        assert_eq!(c[1].callee_raw, "ls");
    }

    #[test]
    fn calls_function_invocation() {
        let c = calls("greet world\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].callee_raw, "greet");
    }
}

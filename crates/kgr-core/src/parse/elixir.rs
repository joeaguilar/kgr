use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const ELIXIR_QUERY_SRC: &str = r#"
;; import MyModule / alias MyModule / use MyModule / require MyModule
(call
  target: (identifier) @_fn
  (arguments
    (alias) @import.path))
"#;

static ELIXIR_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_elixir::LANGUAGE.into();
    Query::new(&language, ELIXIR_QUERY_SRC).expect("Failed to compile Elixir import query")
});

const ELIXIR_SYMBOL_QUERY_SRC: &str = r#"
;; Module definition: defmodule Foo do ... end
(call
  target: (identifier) @_kw
  (arguments (alias) @class.name))

;; Function definition: def foo(...) do ... end
(call
  target: (identifier) @_kw2
  (arguments
    (call target: (identifier) @fn.name)))
"#;

static ELIXIR_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_elixir::LANGUAGE.into();
    Query::new(&language, ELIXIR_SYMBOL_QUERY_SRC).expect("Failed to compile Elixir symbol query")
});

const ELIXIR_CALL_QUERY_SRC: &str = r#"
;; Function call: foo(args)
(call
  target: (identifier) @call.name)

;; Dot call: Module.function()
(call
  target: (dot
    right: (identifier) @call.method))
"#;

static ELIXIR_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_elixir::LANGUAGE.into();
    Query::new(&language, ELIXIR_CALL_QUERY_SRC).expect("Failed to compile Elixir call query")
});

thread_local! {
    static ELIXIR_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_elixir::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct ElixirParser;

impl ElixirParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        ELIXIR_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = parser.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Elixir file: {}", path.display());
            }
            tree
        })
    }
}

impl super::Parser for ElixirParser {
    fn lang(&self) -> Lang {
        Lang::Elixir
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*ELIXIR_SYMBOL_QUERY;
        let kw_idx = query
            .capture_index_for_name("_kw")
            .expect("_kw capture must exist");
        let class_idx = query
            .capture_index_for_name("class.name")
            .expect("class.name capture must exist");
        let kw2_idx = query
            .capture_index_for_name("_kw2")
            .expect("_kw2 capture must exist");
        let fn_idx = query
            .capture_index_for_name("fn.name")
            .expect("fn.name capture must exist");

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            // Check for module definition (defmodule)
            if let Some(kw_capture) = m.captures.iter().find(|c| c.index == kw_idx) {
                let kw_name = match kw_capture.node.utf8_text(source) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if kw_name == "defmodule" {
                    if let Some(name_capture) = m.captures.iter().find(|c| c.index == class_idx) {
                        let node = name_capture.node;
                        let name = match node.utf8_text(source) {
                            Ok(s) => s.to_string(),
                            Err(_) => continue,
                        };
                        let start = node.start_position();
                        let end = node.end_position();
                        symbols.push(Symbol {
                            exported: true,
                            name,
                            kind: SymbolKind::Class,
                            span: Span {
                                start_line: start.row + 1,
                                start_col: start.column,
                                end_line: end.row + 1,
                                end_col: end.column,
                            },
                        });
                    }
                }
            }

            // Check for function definition (def/defp)
            if let Some(kw2_capture) = m.captures.iter().find(|c| c.index == kw2_idx) {
                let kw_name = match kw2_capture.node.utf8_text(source) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let exported = match kw_name {
                    "def" => true,
                    "defp" => false,
                    _ => continue,
                };
                if let Some(name_capture) = m.captures.iter().find(|c| c.index == fn_idx) {
                    let node = name_capture.node;
                    let name = match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    };
                    let start = node.start_position();
                    let end = node.end_position();
                    symbols.push(Symbol {
                        exported,
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
        }

        symbols
    }

    fn extract_calls(&self, source: &[u8], path: &Path) -> Vec<CallRef> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*ELIXIR_CALL_QUERY;
        let name_idx = query
            .capture_index_for_name("call.name")
            .expect("call.name capture must exist");
        let method_idx = query
            .capture_index_for_name("call.method")
            .expect("call.method capture must exist");

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
                    // Dot call: get the full `Module.function` text from parent dot node
                    let dot_node = node.parent().unwrap();
                    match dot_node.utf8_text(source) {
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

        let query = &*ELIXIR_QUERY;
        let fn_capture_idx = query
            .capture_index_for_name("_fn")
            .expect("_fn capture must exist");
        let path_capture_idx = query
            .capture_index_for_name("import.path")
            .expect("import.path capture must exist");

        let mut cursor = QueryCursor::new();
        let mut imports = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            // Get the keyword name (_fn capture) to filter for import/alias/use/require
            let fn_capture = match m.captures.iter().find(|c| c.index == fn_capture_idx) {
                Some(c) => c,
                None => continue,
            };

            let fn_name = match fn_capture.node.utf8_text(source) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if !matches!(fn_name, "import" | "alias" | "use" | "require") {
                continue;
            }

            // Extract the import path
            let path_capture = match m.captures.iter().find(|c| c.index == path_capture_idx) {
                Some(c) => c,
                None => continue,
            };

            let node = path_capture.node;
            let raw = match node.utf8_text(source) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };

            if !seen.insert(raw.clone()) {
                continue;
            }

            let start = node.start_position();
            let end = node.end_position();

            // All Elixir imports are External
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

        imports
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        ElixirParser.parse(src.as_bytes(), Path::new("test.ex"))
    }

    #[test]
    fn import_module() {
        let imports = parse("import MyModule");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "MyModule");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn alias_module() {
        let imports = parse("alias MyApp.Repo");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "MyApp.Repo");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn use_module() {
        let imports = parse("use GenServer");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "GenServer");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn require_module() {
        let imports = parse("require Logger");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Logger");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import MyModule
alias MyApp.Repo
use GenServer
require Logger
"#,
        );
        assert_eq!(imports.len(), 4);
    }

    #[test]
    fn ignores_non_import_calls() {
        let imports = parse("IO.puts(\"hello\")");
        assert!(imports.is_empty());
    }

    #[test]
    fn deduplicates_imports() {
        let imports = parse("import MyModule\nimport MyModule\n");
        assert_eq!(imports.len(), 1);
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        ElixirParser.extract_symbols(src.as_bytes(), Path::new("test.ex"))
    }

    #[test]
    fn symbols_finds_module() {
        let syms = symbols(
            r#"
defmodule MyApp.Server do
end
"#,
        );
        let classes: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "MyApp.Server");
        assert!(classes[0].exported);
    }

    #[test]
    fn symbols_finds_public_function() {
        let syms = symbols(
            r#"
defmodule MyApp do
  def hello(name) do
    name
  end
end
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "hello");
        assert!(fns[0].exported);
    }

    #[test]
    fn symbols_private_function_not_exported() {
        let syms = symbols(
            r#"
defmodule MyApp do
  defp internal_helper(x) do
    x
  end
end
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "internal_helper");
        assert!(!fns[0].exported);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        ElixirParser.extract_calls(src.as_bytes(), Path::new("test.ex"))
    }

    #[test]
    fn calls_simple_function() {
        let c = calls("foo(1, 2)\n");
        assert!(c.iter().any(|call| call.callee_raw == "foo"));
    }

    #[test]
    fn calls_dot_call() {
        let c = calls("String.upcase(\"hello\")\n");
        assert!(c.iter().any(|call| call.callee_raw == "String.upcase"));
    }
}

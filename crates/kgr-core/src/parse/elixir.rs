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

;; Paren-less definition: def foo do ... end
(call
  target: (identifier) @_kw2
  (arguments (identifier) @fn.name))

;; Guarded definition: def foo(...) when ... do ... end
(call
  target: (identifier) @_kw2
  (arguments
    (binary_operator
      left: (call target: (identifier) @fn.name)
      operator: "when")))

;; Paren-less guarded definition: def foo when ... do ... end
(call
  target: (identifier) @_kw2
  (arguments
    (binary_operator
      left: (identifier) @fn.name
      operator: "when")))
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

/// Definition/import keywords that parse as identifier-target calls in the
/// Elixir grammar but are not function calls for our purposes.
const ELIXIR_DEF_KEYWORDS: &[&str] = &[
    "def",
    "defp",
    "defmodule",
    "defmacro",
    "defmacrop",
    "use",
    "import",
    "alias",
    "require",
];

/// Keywords whose call argument position holds a definition head
/// (`def foo(x) do` — the inner `foo(x)` call is the head, not a call site).
const ELIXIR_DEF_HEAD_KEYWORDS: &[&str] = &["def", "defp", "defmacro", "defmacrop"];

/// Returns true when `ident` (the target identifier of a call node) belongs to
/// the head of a function definition, i.e. the `foo` in `def foo(x) do` or in
/// `def foo(x) when guard do`. Mirrors the shapes matched by the symbol query:
///
/// - plain head: `(call target: def (arguments (call target: @ident ...)))`
/// - guarded head: `(call target: def (arguments (binary_operator
///   left: (call target: @ident ...) operator: "when" ...)))`
///
/// Paren-less heads (`def foo do`) are bare identifiers, not call nodes, so
/// they never reach the call query and need no exclusion here.
fn is_def_head(ident: tree_sitter::Node, source: &[u8]) -> bool {
    // `ident` is the target of its enclosing call node.
    let head_call = match ident.parent() {
        Some(n) => n,
        None => return false,
    };
    let parent = match head_call.parent() {
        Some(n) => n,
        None => return false,
    };

    let arguments = if parent.kind() == "arguments" {
        parent
    } else if parent.kind() == "binary_operator" {
        // Guarded head: the head call must be the *left* operand. Calls in the
        // guard expression itself (right side) are real call sites.
        match parent.child_by_field_name("left") {
            Some(left) if left.id() == head_call.id() => {}
            _ => return false,
        }
        match parent.parent() {
            Some(n) if n.kind() == "arguments" => n,
            _ => return false,
        }
    } else {
        return false;
    };

    let def_call = match arguments.parent() {
        Some(n) if n.kind() == "call" => n,
        _ => return false,
    };
    let target = match def_call.child_by_field_name("target") {
        Some(n) => n,
        None => return false,
    };
    matches!(target.utf8_text(source), Ok(kw) if ELIXIR_DEF_HEAD_KEYWORDS.contains(&kw))
}

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

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_elixir::LANGUAGE.into())
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
                    "def" | "defmacro" => true,
                    "defp" | "defmacrop" => false,
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
                    let text = match node.utf8_text(source) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    // Definition keywords parse as calls but are not call sites.
                    if ELIXIR_DEF_KEYWORDS.contains(&text) {
                        continue;
                    }
                    // The function's own name in a def head is not a call site.
                    if is_def_head(node, source) {
                        continue;
                    }
                    text.to_string()
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

    #[test]
    fn symbols_parenless_def() {
        let syms = symbols(
            r#"
defmodule MyApp do
  def no_parens do
    :ok
  end
end
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "no_parens");
        assert!(fns[0].exported);
    }

    #[test]
    fn symbols_guarded_def() {
        let syms = symbols(
            r#"
defmodule MyApp do
  def guarded(x) when x > 0 do
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
        assert_eq!(fns[0].name, "guarded");
        assert!(fns[0].exported);
    }

    #[test]
    fn symbols_guarded_defp_keyword_do() {
        let syms = symbols(
            r#"
defmodule MyApp do
  defp guarded_priv(x) when is_integer(x), do: x
end
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "guarded_priv");
        assert!(!fns[0].exported);
    }

    #[test]
    fn symbols_parenless_guarded_def() {
        let syms = symbols(
            r#"
defmodule MyApp do
  def maybe when true do
    :ok
  end
end
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "maybe");
        assert!(fns[0].exported);
    }

    #[test]
    fn symbols_defmacro_exported() {
        let syms = symbols(
            r#"
defmodule MyApp do
  defmacro my_macro(x) do
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
        assert_eq!(fns[0].name, "my_macro");
        assert!(fns[0].exported);
    }

    #[test]
    fn symbols_defmacrop_not_exported() {
        let syms = symbols(
            r#"
defmodule MyApp do
  defmacrop priv_macro do
    :ok
  end
end
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "priv_macro");
        assert!(!fns[0].exported);
    }

    #[test]
    fn symbols_no_duplicates_across_def_forms() {
        let syms = symbols(
            r#"
defmodule MyApp do
  def plain(x) do
    x
  end
  def no_parens do
    :ok
  end
  def guarded(x) when x > 0 do
    x
  end
  defmacro mac(x) do
    x
  end
end
"#,
        );
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 4);
        let names: Vec<&str> = fns.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["plain", "no_parens", "guarded", "mac"]);
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

    #[test]
    fn calls_exclude_definition_keywords() {
        let c = calls(
            r#"
defmodule MyApp do
  use GenServer
  import Enum
  alias MyApp.Repo
  require Logger

  def pub(x) do
    x
  end

  defp priv(x) do
    x
  end

  defmacro mac(x) do
    x
  end

  defmacrop macp(x) do
    x
  end
end
"#,
        );
        let names: Vec<&str> = c.iter().map(|call| call.callee_raw.as_str()).collect();
        for kw in super::ELIXIR_DEF_KEYWORDS {
            assert!(
                !names.contains(kw),
                "keyword {kw:?} must not be emitted as a call, got {names:?}"
            );
        }
    }

    #[test]
    fn calls_exclude_def_head() {
        let c = calls(
            r#"
defmodule MyApp do
  def hello(name) do
    String.upcase(name)
  end

  defp internal(x) do
    x
  end
end
"#,
        );
        let names: Vec<&str> = c.iter().map(|call| call.callee_raw.as_str()).collect();
        assert!(
            !names.contains(&"hello"),
            "def head must not be a call, got {names:?}"
        );
        assert!(
            !names.contains(&"internal"),
            "defp head must not be a call, got {names:?}"
        );
        assert!(names.contains(&"String.upcase"));
    }

    #[test]
    fn calls_exclude_guarded_def_head_but_keep_guard_calls() {
        let c = calls(
            r#"
defmodule MyApp do
  def guarded(x) when is_integer(x) do
    helper(x)
  end
end
"#,
        );
        let names: Vec<&str> = c.iter().map(|call| call.callee_raw.as_str()).collect();
        assert!(
            !names.contains(&"guarded"),
            "guarded def head must not be a call, got {names:?}"
        );
        assert!(names.contains(&"is_integer"), "guard call must survive");
        assert!(names.contains(&"helper"), "body call must survive");
    }

    #[test]
    fn calls_in_body_survive_including_recursion() {
        let c = calls(
            r#"
defmodule MyApp do
  def fact(n) do
    fact(n - 1)
  end

  def run do
    foo(1)
    String.upcase("hi")
  end
end
"#,
        );
        let names: Vec<&str> = c.iter().map(|call| call.callee_raw.as_str()).collect();
        // The recursive call site in the body is real, only the def head is not.
        assert_eq!(
            names.iter().filter(|n| **n == "fact").count(),
            1,
            "exactly the recursive body call should remain, got {names:?}"
        );
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"String.upcase"));
        assert!(!names.contains(&"run"));
        assert!(!names.contains(&"def"));
    }

    #[test]
    fn calls_keyword_do_def_body_survives() {
        let c = calls(
            r#"
defmodule MyApp do
  defp short(x) when x > 0, do: helper(x)
end
"#,
        );
        let names: Vec<&str> = c.iter().map(|call| call.callee_raw.as_str()).collect();
        assert!(
            !names.contains(&"short"),
            "guarded keyword-do def head must not be a call, got {names:?}"
        );
        assert!(
            names.contains(&"helper"),
            "keyword-do body call must survive"
        );
    }
}

use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const HASKELL_QUERY_SRC: &str = r#"
;; import Data.List
;; import qualified Data.Map as Map
(import
  module: (module) @import.path)
"#;

static HASKELL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_haskell::LANGUAGE.into();
    Query::new(&language, HASKELL_QUERY_SRC).expect("Failed to compile Haskell import query")
});

const HASKELL_SYMBOL_QUERY_SRC: &str = r#"
;; Function binding: foo x = x + 1
(function
  name: (variable) @fn.name) @def

;; Zero-argument top-level binding: main = ..., point-free defs, constants.
;; Anchored to the top-level (declarations ...) node so that where/let-bound
;; locals (which live under local_binds) don't flood the symbol list.
(declarations
  (bind
    name: (variable) @fn.name) @def)

;; Type synonym: type Foo = Bar
(type_synomym
  name: (name) @class.name) @def

;; Newtype: newtype Foo = MkFoo Bar
(newtype
  name: (name) @class.name) @def

;; Algebraic data type: data Foo = Bar | Baz
(data_type
  name: (name) @class.name) @def

;; Type class: class Functor f where ...
(class
  name: (name) @class.name) @def
"#;

static HASKELL_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_haskell::LANGUAGE.into();
    Query::new(&language, HASKELL_SYMBOL_QUERY_SRC).expect("Failed to compile Haskell symbol query")
});

const HASKELL_CALL_QUERY_SRC: &str = r#"
;; Function application
(apply
  function: (variable) @call.name)
"#;

static HASKELL_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_haskell::LANGUAGE.into();
    Query::new(&language, HASKELL_CALL_QUERY_SRC).expect("Failed to compile Haskell call query")
});

thread_local! {
    static HASKELL_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_haskell::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct HaskellParser;

impl HaskellParser {
    fn parse_tree(&self, source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
        HASKELL_PARSER.with(|parser| {
            let mut p = parser.borrow_mut();
            let tree = p.parse(source, None);
            if tree.is_none() {
                tracing::warn!("Failed to parse Haskell file: {}", path.display());
            }
            tree
        })
    }
}

/// Parse the module header's export list, if any.
///
/// Returns `None` when the module has no header or the header has no export
/// list (`module M where` / headerless file), meaning every top-level binding
/// is exported. Returns `Some(names)` with the exported identifiers otherwise.
///
/// Entries are handled conservatively: value exports (`foo`), type/class
/// exports (`Color`, `Color(..)`), and operator exports (`(<+>)`) contribute
/// their head name; a `module M` self re-export means everything is exported,
/// so it collapses back to `None`. Other module re-exports are ignored.
fn export_list(
    tree: &tree_sitter::Tree,
    source: &[u8],
) -> Option<std::collections::HashSet<String>> {
    let root = tree.root_node();
    let mut root_cursor = root.walk();
    let header = root
        .named_children(&mut root_cursor)
        .find(|n| n.kind() == "header")?;
    let exports = header.child_by_field_name("exports")?;

    let module_name = header
        .child_by_field_name("module")
        .and_then(|m| m.utf8_text(source).ok());

    let mut names = std::collections::HashSet::new();
    let mut cursor = exports.walk();
    for entry in exports.named_children(&mut cursor) {
        match entry.kind() {
            "export" => {
                let name_node = entry
                    .child_by_field_name("variable")
                    .or_else(|| entry.child_by_field_name("type"))
                    .or_else(|| entry.child_by_field_name("operator"));
                if let Some(node) = name_node {
                    if let Ok(text) = node.utf8_text(source) {
                        // Operators render as `(<+>)`; strip the parens so the
                        // stored name matches the bare identifier.
                        let trimmed = text.trim_matches(|c| c == '(' || c == ')');
                        names.insert(trimmed.to_string());
                    }
                }
            }
            "module_export" => {
                // `module M` re-export of the module itself exports every
                // top-level binding: behave as if there were no export list.
                let re_exported = entry
                    .child_by_field_name("module")
                    .and_then(|m| m.utf8_text(source).ok());
                if module_name.is_some() && re_exported == module_name {
                    return None;
                }
            }
            _ => {}
        }
    }

    Some(names)
}

impl super::Parser for HaskellParser {
    fn lang(&self) -> Lang {
        Lang::Haskell
    }

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_haskell::LANGUAGE.into())
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let exports = export_list(&tree, source);

        let query = &*HASKELL_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();
        let def_idx = query.capture_index_for_name("def").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            // Span comes from the enclosing definition node, name from the name node
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index == def_idx)
                .map(|c| c.node);
            for capture in m.captures {
                if capture.index == def_idx {
                    continue;
                }
                let node = capture.node;
                let name = match node.utf8_text(source) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                let kind = if capture.index == fn_idx {
                    SymbolKind::Function
                } else if capture.index == class_idx {
                    SymbolKind::Class
                } else {
                    continue;
                };

                // Without an export list every top-level binding is exported;
                // with one, only the names it lists are.
                let exported = exports.as_ref().map_or(true, |names| names.contains(&name));

                let span_node = def_node.unwrap_or(node);
                let start = span_node.start_position();
                let end = span_node.end_position();

                symbols.push(Symbol {
                    exported,
                    name,
                    kind,
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

        let query = &*HASKELL_CALL_QUERY;
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

        {
            let query = &*HASKELL_QUERY;
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

                    // All Haskell imports are external (package/module imports)
                    let kind = ImportKind::External;

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        HaskellParser.parse(src.as_bytes(), Path::new("Main.hs"))
    }

    #[test]
    fn simple_import() {
        let imports = parse("import Data.List");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Data.List");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn qualified_import() {
        let imports = parse("import qualified Data.Map as Map");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "Data.Map");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn multiple_imports() {
        let imports = parse(
            r#"
import Data.List
import Data.Maybe
import Control.Monad
"#,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw, "Data.List");
        assert_eq!(imports[1].raw, "Data.Maybe");
        assert_eq!(imports[2].raw, "Control.Monad");
    }

    // -- Symbol extraction tests ------------------------------------------

    fn symbols(src: &str) -> Vec<Symbol> {
        HaskellParser.extract_symbols(src.as_bytes(), Path::new("Main.hs"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("foo x = x + 1\nbar y = y * 2\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    #[test]
    fn symbols_finds_data_types() {
        let syms = symbols("data Color = Red | Green | Blue\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_span_covers_full_definition() {
        let src = "addTwo x =\n  let y = x + 1\n  in y + 1\n";
        let syms = symbols(src);
        let f = syms.iter().find(|s| s.name == "addTwo").unwrap();
        assert_eq!(f.span.start_line, 1);
        assert_eq!(f.span.end_line, 3);
        assert!(f.span.end_line > f.span.start_line);
    }

    #[test]
    fn symbols_all_exported() {
        let syms = symbols("foo x = x + 1\n");
        assert!(!syms.is_empty());
        assert!(syms[0].exported);
    }

    // -- Call extraction test (verify query compiles) ---------------------

    fn calls(src: &str) -> Vec<CallRef> {
        HaskellParser.extract_calls(src.as_bytes(), Path::new("Main.hs"))
    }

    #[test]
    fn symbols_finds_zero_arg_bindings() {
        // main = ..., point-free definitions, and constants are bind nodes.
        let syms = symbols("main = putStrLn \"hello\"\n\nanswer = 42\n");
        let names: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, vec!["main", "answer"]);
    }

    #[test]
    fn symbols_skips_local_binds() {
        // where/let-bound locals must not appear as top-level symbols.
        let syms = symbols("c = let inner = 2 in inner\n\nd = e\n  where\n    e = 3\n");
        let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["c", "d"]);
    }

    #[test]
    fn symbols_export_list_filters_exported() {
        let syms = symbols("module M (foo, Color (..)) where\n\nfoo = 1\n\nbar x = x\n\ndata Color = Red | Green\n");
        let foo = syms.iter().find(|s| s.name == "foo").unwrap();
        let bar = syms.iter().find(|s| s.name == "bar").unwrap();
        let color = syms.iter().find(|s| s.name == "Color").unwrap();
        assert!(foo.exported);
        assert!(!bar.exported);
        assert!(color.exported);
    }

    #[test]
    fn symbols_no_export_list_all_exported() {
        // `module M where` (no export list) keeps the all-exported default.
        let syms = symbols("module M where\n\nfoo = 1\n\nbar x = x\n");
        assert_eq!(syms.len(), 2);
        assert!(syms.iter().all(|s| s.exported));
    }

    #[test]
    fn symbols_self_module_reexport_all_exported() {
        // `module M (module M) where` re-exports everything in the module.
        let syms = symbols("module M (module M) where\n\nfoo = 1\n");
        let foo = syms.iter().find(|s| s.name == "foo").unwrap();
        assert!(foo.exported);
    }

    #[test]
    fn calls_query_compiles() {
        // Just verify the call query compiles; Haskell function application
        // is implicit (juxtaposition), so extraction may be limited.
        let c = calls("main = putStrLn \"hello\"\n");
        // We don't assert specific results since the grammar may or may not
        // represent function application as (apply ...).
        let _ = c;
    }
}

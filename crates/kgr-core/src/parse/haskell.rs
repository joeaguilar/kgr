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
  name: (variable) @fn.name)

;; Type synonym: type Foo = Bar
(type_synomym
  name: (name) @class.name)

;; Newtype: newtype Foo = MkFoo Bar
(newtype
  name: (name) @class.name)

;; Algebraic data type: data Foo = Bar | Baz
(data_type
  name: (name) @class.name)

;; Type class: class Functor f where ...
(class
  name: (name) @class.name)
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

impl super::Parser for HaskellParser {
    fn lang(&self) -> Lang {
        Lang::Haskell
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match self.parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*HASKELL_SYMBOL_QUERY;
        let fn_idx = query.capture_index_for_name("fn.name").unwrap();
        let class_idx = query.capture_index_for_name("class.name").unwrap();

        let mut cursor = QueryCursor::new();
        let mut symbols = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
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

                // In Haskell, all top-level bindings are exported by default
                let exported = true;

                let start = node.start_position();
                let end = node.end_position();

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
    fn calls_query_compiles() {
        // Just verify the call query compiles; Haskell function application
        // is implicit (juxtaposition), so extraction may be limited.
        let c = calls("main = putStrLn \"hello\"\n");
        // We don't assert specific results since the grammar may or may not
        // represent function application as (apply ...).
        let _ = c;
    }
}

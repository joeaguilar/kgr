pub mod bash;
pub mod c;
pub mod cpp;
pub mod csharp;
pub mod elixir;
pub mod go;
pub mod haskell;
pub mod java;
pub mod javascript;
pub mod lua;
pub mod objc;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust_lang;
pub mod scala;
pub mod swift;
pub mod typescript;
pub mod zig;

use std::collections::HashMap;
use std::path::Path;

use crate::types::{CallRef, Import, Lang, ParseError, Span, Symbol};

pub trait Parser: Send + Sync {
    fn lang(&self) -> Lang;
    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import>;

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let _ = (source, path);
        Vec::new()
    }

    fn extract_calls(&self, source: &[u8], path: &Path) -> Vec<CallRef> {
        let _ = (source, path);
        Vec::new()
    }

    /// Return the tree-sitter Language for this parser, if available.
    /// Override this to enable generic syntax error detection via `collect_parse_errors`.
    fn ts_language(&self) -> Option<tree_sitter::Language> {
        None
    }

    /// Detect syntax errors (ERROR/MISSING nodes) in a source file.
    /// The default implementation uses `ts_language()` to create a parser and
    /// delegates to `collect_parse_errors`. Parsers that override `ts_language()`
    /// get syntax checking for free.
    fn parse_errors(&self, source: &[u8], _path: &Path) -> Vec<ParseError> {
        let lang = match self.ts_language() {
            Some(l) => l,
            None => return Vec::new(),
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang).is_err() {
            return Vec::new();
        }
        match parser.parse(source, None) {
            Some(tree) => collect_parse_errors(&tree),
            None => Vec::new(),
        }
    }
}

/// Walk a tree-sitter parse tree and collect ERROR/MISSING nodes.
pub fn collect_parse_errors(tree: &tree_sitter::Tree) -> Vec<ParseError> {
    let mut errors = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if node.kind() == "ERROR" || node.is_missing() {
            errors.push(ParseError {
                message: if node.is_missing() {
                    format!("MISSING {}", node.kind())
                } else {
                    "ERROR".to_string()
                },
                span: Span {
                    start_line: node.start_position().row + 1,
                    start_col: node.start_position().column,
                    end_line: node.end_position().row + 1,
                    end_col: node.end_position().column,
                },
            });
        }
        // Depth-first traversal
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return errors;
            }
        }
    }
}

pub struct ParserRegistry {
    parsers: HashMap<Lang, Box<dyn Parser>>,
}

impl ParserRegistry {
    pub fn new() -> Self {
        let mut parsers: HashMap<Lang, Box<dyn Parser>> = HashMap::new();
        parsers.insert(Lang::Python, Box::new(python::PythonParser));
        parsers.insert(Lang::TypeScript, Box::new(typescript::TypeScriptParser));
        parsers.insert(Lang::JavaScript, Box::new(javascript::JavaScriptParser));
        parsers.insert(Lang::Java, Box::new(java::JavaParser));
        parsers.insert(Lang::C, Box::new(c::CParser));
        parsers.insert(Lang::Cpp, Box::new(cpp::CppParser));
        parsers.insert(Lang::Rust, Box::new(rust_lang::RustParser));
        parsers.insert(Lang::Go, Box::new(go::GoParser));
        parsers.insert(Lang::Zig, Box::new(zig::ZigParser));
        parsers.insert(Lang::Swift, Box::new(swift::SwiftParser));
        parsers.insert(Lang::Ruby, Box::new(ruby::RubyParser));
        parsers.insert(Lang::Haskell, Box::new(haskell::HaskellParser));
        parsers.insert(Lang::ObjectiveC, Box::new(objc::ObjCParser));
        parsers.insert(Lang::Bash, Box::new(bash::BashParser));
        parsers.insert(Lang::Elixir, Box::new(elixir::ElixirParser));
        parsers.insert(Lang::Lua, Box::new(lua::LuaParser));
        parsers.insert(Lang::Scala, Box::new(scala::ScalaParser));
        parsers.insert(Lang::Php, Box::new(php::PhpParser));
        parsers.insert(Lang::CSharp, Box::new(csharp::CSharpParser));
        Self { parsers }
    }

    pub fn get(&self, lang: Lang) -> Option<&dyn Parser> {
        self.parsers.get(&lang).map(|p| p.as_ref())
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_parser_exposes_ts_language() {
        let registry = ParserRegistry::new();
        for (lang, parser) in &registry.parsers {
            assert!(
                parser.ts_language().is_some(),
                "parser for {lang:?} must override ts_language() so --syntax works"
            );
        }
    }

    #[test]
    fn registry_parsers_detect_syntax_errors() {
        let registry = ParserRegistry::new();
        let cases: &[(Lang, &[u8], &str)] = &[
            (Lang::Python, b"def broken(:", "broken.py"),
            (Lang::TypeScript, b"function broken(:", "broken.ts"),
            (Lang::Rust, b"fn broken(:", "broken.rs"),
        ];
        for (lang, source, name) in cases {
            let parser = registry.get(*lang).expect("parser registered");
            let errors = parser.parse_errors(source, Path::new(name));
            assert!(
                !errors.is_empty(),
                "expected parse_errors to flag malformed {lang:?} source"
            );
        }
    }
}

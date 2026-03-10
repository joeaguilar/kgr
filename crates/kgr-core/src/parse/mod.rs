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

use crate::types::{CallRef, Import, Lang, Symbol};

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

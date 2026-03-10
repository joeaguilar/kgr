use std::path::Path;

use crate::types::Lang;

pub fn detect_lang(path: &Path) -> Lang {
    match path.extension().and_then(|e| e.to_str()) {
        Some("py" | "pyi") => Lang::Python,
        Some("ts") => Lang::TypeScript,
        Some("tsx") => Lang::TypeScript,
        Some("js" | "jsx" | "mjs" | "cjs") => Lang::JavaScript,
        Some("java") => Lang::Java,
        Some("c" | "h") => Lang::C,
        Some("cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx") => Lang::Cpp,
        Some("rs") => Lang::Rust,
        Some("go") => Lang::Go,
        Some("zig") => Lang::Zig,
        Some("cs") => Lang::CSharp,
        Some("m") => Lang::ObjectiveC,
        Some("swift") => Lang::Swift,
        Some("rb" | "rake" | "gemspec") => Lang::Ruby,
        Some("php") => Lang::Php,
        Some("scala" | "sc") => Lang::Scala,
        Some("lua") => Lang::Lua,
        Some("ex" | "exs") => Lang::Elixir,
        Some("hs") => Lang::Haskell,
        Some("sh" | "bash") => Lang::Bash,
        _ => Lang::Unknown,
    }
}

pub fn lang_extensions(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Python => &["py", "pyi"],
        Lang::TypeScript => &["ts", "tsx"],
        Lang::JavaScript => &["js", "jsx", "mjs", "cjs"],
        Lang::Java => &["java"],
        Lang::C => &["c", "h"],
        Lang::Cpp => &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        Lang::Rust => &["rs"],
        Lang::Go => &["go"],
        Lang::Zig => &["zig"],
        Lang::CSharp => &["cs"],
        Lang::ObjectiveC => &["m"],
        Lang::Swift => &["swift"],
        Lang::Ruby => &["rb", "rake", "gemspec"],
        Lang::Php => &["php"],
        Lang::Scala => &["scala", "sc"],
        Lang::Lua => &["lua"],
        Lang::Elixir => &["ex", "exs"],
        Lang::Haskell => &["hs"],
        Lang::Bash => &["sh", "bash"],
        Lang::Unknown => &[],
    }
}

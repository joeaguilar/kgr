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
        Lang::Unknown => &[],
    }
}

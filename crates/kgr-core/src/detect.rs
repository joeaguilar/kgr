use std::path::Path;

use crate::types::Lang;

pub fn detect_lang(path: &Path) -> Lang {
    match path.extension().and_then(|e| e.to_str()) {
        Some("py" | "pyi") => Lang::Python,
        Some("ts" | "tsx" | "mts" | "cts") => Lang::TypeScript,
        Some("js" | "jsx" | "mjs" | "cjs") => Lang::JavaScript,
        Some("java") => Lang::Java,
        Some("c" | "h") => Lang::C,
        Some("cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx") => Lang::Cpp,
        Some("rs") => Lang::Rust,
        Some("go") => Lang::Go,
        Some("zig") => Lang::Zig,
        Some("cs") => Lang::CSharp,
        Some("m" | "mm") => Lang::ObjectiveC,
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
        Lang::TypeScript => &["ts", "tsx", "mts", "cts"],
        Lang::JavaScript => &["js", "jsx", "mjs", "cjs"],
        Lang::Java => &["java"],
        Lang::C => &["c", "h"],
        Lang::Cpp => &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        Lang::Rust => &["rs"],
        Lang::Go => &["go"],
        Lang::Zig => &["zig"],
        Lang::CSharp => &["cs"],
        Lang::ObjectiveC => &["m", "mm"],
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

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_LANGS: &[Lang] = &[
        Lang::Python,
        Lang::TypeScript,
        Lang::JavaScript,
        Lang::Java,
        Lang::C,
        Lang::Cpp,
        Lang::Rust,
        Lang::Go,
        Lang::Zig,
        Lang::CSharp,
        Lang::ObjectiveC,
        Lang::Swift,
        Lang::Ruby,
        Lang::Php,
        Lang::Scala,
        Lang::Lua,
        Lang::Elixir,
        Lang::Haskell,
        Lang::Bash,
        Lang::Unknown,
    ];

    #[test]
    fn detects_typescript_module_extensions() {
        assert_eq!(detect_lang(Path::new("src/main.mts")), Lang::TypeScript);
        assert_eq!(detect_lang(Path::new("src/main.cts")), Lang::TypeScript);
    }

    #[test]
    fn detects_objective_c_plus_plus_as_objc() {
        assert_eq!(detect_lang(Path::new("src/view.mm")), Lang::ObjectiveC);
    }

    #[test]
    fn language_extension_lists_include_detected_extensions() {
        assert!(lang_extensions(Lang::TypeScript).contains(&"mts"));
        assert!(lang_extensions(Lang::TypeScript).contains(&"cts"));
        assert!(lang_extensions(Lang::ObjectiveC).contains(&"mm"));
    }

    #[test]
    fn language_extensions_round_trip_through_detection() {
        for &lang in ALL_LANGS {
            for ext in lang_extensions(lang) {
                let file_name = format!("source.{ext}");
                assert_eq!(
                    detect_lang(Path::new(&file_name)),
                    lang,
                    "{ext} must detect as {lang:?}"
                );
            }
        }

        assert!(lang_extensions(Lang::Unknown).is_empty());
        assert_eq!(detect_lang(Path::new("source.unknown")), Lang::Unknown);
    }
}

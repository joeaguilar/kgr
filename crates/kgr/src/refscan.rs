//! Text-level reference scanning for `refs`, `dead` and `impact`.
//!
//! Parsers record call sites as raw callee text, which misses references that
//! are not plain `name(...)` calls: functions passed as values, calls inside
//! macro token trees, JSX tags, decorators, and calls through import aliases.
//! These helpers recover those from source text so that `dead` never reports a
//! referenced symbol as removable.
//!
//! [`ScannedSource`] classifies every byte of a file as code, string literal
//! or comment from the language's tree-sitter parse (see
//! `kgr_core::parse::literals`). String interpolation (`f"{x}"`, `` `${x}` ``,
//! `"$(x)"`) is code, because it is code. Anything the grammar could not
//! parse cleanly stays code, so a misclassification can only make a name
//! look referenced, never hide a reference.

use std::path::Path;
use std::sync::LazyLock;

use kgr_core::parse::literals::LiteralKind;
use kgr_core::parse::ParserRegistry;
use kgr_core::types::Lang;

static REGISTRY: LazyLock<ParserRegistry> = LazyLock::new(ParserRegistry::new);

/// Lexical class of a byte of source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Code,
    Str,
    Comment,
}

/// A whole-identifier occurrence of a name in a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    /// Byte offset into the source.
    pub offset: usize,
    /// 1-based line.
    pub line: usize,
    /// 0-based byte column, matching tree-sitter spans.
    pub col: usize,
    pub region: Region,
}

/// A source file with every byte classified by [`Region`]. The tree-sitter
/// classification runs lazily, only for files that mention a queried name.
pub struct ScannedSource {
    pub source: String,
    pub lang: Lang,
    path: std::path::PathBuf,
    analysis: std::cell::OnceCell<Analysis>,
    line_starts: Vec<usize>,
}

struct Analysis {
    regions: Vec<Region>,
    /// `source` with string and comment bytes blanked to spaces (newlines
    /// kept), so code-only searches keep their byte offsets.
    code: String,
}

impl ScannedSource {
    pub fn new(source: String, lang: Lang, path: &Path) -> Self {
        let line_starts = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            source,
            lang,
            path: path.to_path_buf(),
            analysis: std::cell::OnceCell::new(),
            line_starts,
        }
    }

    fn analysis(&self) -> &Analysis {
        self.analysis.get_or_init(|| {
            let source = &self.source;
            let mut regions = vec![Region::Code; source.len()];
            for range in REGISTRY
                .literal_ranges(self.lang, source.as_bytes(), &self.path)
                .unwrap_or_default()
            {
                let region = match range.kind {
                    LiteralKind::Str => Region::Str,
                    LiteralKind::Comment => Region::Comment,
                };
                regions[range.start..range.end.min(source.len())].fill(region);
            }
            let code_bytes: Vec<u8> = source
                .bytes()
                .zip(&regions)
                .map(|(b, r)| {
                    if *r == Region::Code || b == b'\n' {
                        b
                    } else {
                        b' '
                    }
                })
                .collect();
            // Literal and comment ranges are whole tree-sitter tokens, so
            // blanking them never splits a multi-byte character.
            let code = String::from_utf8(code_bytes)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
            Analysis { regions, code }
        })
    }

    /// 1-based line and 0-based byte column of `offset`.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let line_index = self.line_starts.partition_point(|&start| start <= offset) - 1;
        (line_index + 1, offset - self.line_starts[line_index])
    }

    /// The trimmed text of 1-based `line`.
    pub fn line_text(&self, line: usize) -> &str {
        let Some(&start) = self.line_starts.get(line.wrapping_sub(1)) else {
            return "";
        };
        let end = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.source.len());
        self.source[start..end].trim()
    }

    /// Every whole-identifier occurrence of `name`, in any region.
    pub fn occurrences(&self, name: &str) -> Vec<Occurrence> {
        let offsets = word_occurrences(&self.source, name);
        if offsets.is_empty() {
            return Vec::new();
        }
        let regions = &self.analysis().regions;
        offsets
            .into_iter()
            .map(|offset| {
                let (line, col) = self.position(offset);
                Occurrence {
                    offset,
                    line,
                    col,
                    region: regions[offset],
                }
            })
            .collect()
    }

    /// The identifier qualifying the code occurrence at `offset`, if any:
    /// `used` for `used.helper`, `util` for `crate::util::helper`.
    pub fn qualifier_before(&self, offset: usize) -> Option<&str> {
        qualifier_before(&self.analysis().code, offset, self.lang)
    }

    /// True when an import statement in this file mentions `ident` in code,
    /// i.e. `ident` is plausibly bound by an import (`import other`,
    /// `import a as other`, `from pkg import other`, `use x::other`).
    pub fn import_binds(&self, ident: &str) -> bool {
        let statements = statement_starts(&self.analysis().code);
        word_occurrences(&self.analysis().code, ident)
            .into_iter()
            .any(|offset| {
                let index = statements.partition_point(|&start| start <= offset) - 1;
                let end = statements
                    .get(index + 1)
                    .copied()
                    .unwrap_or(self.analysis().code.len());
                is_import_statement(&self.analysis().code[statements[index]..end])
            })
    }

    /// True when every code occurrence of `ident` is either inside an import
    /// statement or used as a qualifier (`ident.x`, `ident::x`, `ident->x`).
    /// Any other use (a parameter, an assignment target) may rebind the name,
    /// so a qualifier spelled `ident` cannot be trusted to mean the module.
    pub fn used_only_as_module(&self, ident: &str) -> bool {
        let statements = statement_starts(&self.analysis().code);
        word_occurrences(&self.analysis().code, ident)
            .into_iter()
            .all(|offset| {
                let after = self.analysis().code[offset + ident.len()..].trim_start();
                if after.starts_with('.') || after.starts_with("::") || after.starts_with("->") {
                    return true;
                }
                let index = statements.partition_point(|&start| start <= offset) - 1;
                let end = statements
                    .get(index + 1)
                    .copied()
                    .unwrap_or(self.analysis().code.len());
                is_import_statement(&self.analysis().code[statements[index]..end])
            })
    }

    /// Local names `name` is imported under: `name as alias` inside an
    /// import statement (`import`/`from`/`use`/`export`), found in code only.
    /// When `module_hints` is non-empty the statement must also mention one
    /// of them, which ties the alias to the module that defines `name`.
    pub fn import_aliases(&self, name: &str, module_hints: &[String]) -> Vec<String> {
        let mut aliases = Vec::new();
        if !self.source.contains(name) {
            return aliases;
        }
        let statements = statement_starts(&self.analysis().code);
        for offset in word_occurrences(&self.analysis().code, name) {
            let rest = self.analysis().code[offset + name.len()..].trim_start();
            let Some(after_as) = rest.strip_prefix("as") else {
                continue;
            };
            if !after_as.starts_with(char::is_whitespace) {
                continue;
            }
            let alias: String = after_as
                .trim_start()
                .chars()
                .take_while(|c| is_ident_char(*c))
                .collect();
            if alias.is_empty() || alias == name || aliases.contains(&alias) {
                continue;
            }
            let index = statements.partition_point(|&start| start <= offset) - 1;
            let start = statements[index];
            let end = statements
                .get(index + 1)
                .copied()
                .unwrap_or(self.analysis().code.len());
            if !is_import_statement(&self.analysis().code[start..end]) {
                continue;
            }
            let raw = &self.source[start..end];
            if module_hints.is_empty() || module_hints.iter().any(|hint| raw.contains(hint)) {
                aliases.push(alias);
            }
        }
        aliases
    }
}

/// True when a recorded callee refers to `name`. `.`, `::`, `->` and `:`
/// all count as qualifier separators, so `name`, `obj.name`, `path::name`,
/// `ptr->name` and Lua's `obj:name` all match; balanced generic argument
/// lists (`parse::<u32>`, `Vec::<u8>::new`) are ignored.
pub fn callee_matches(callee_raw: &str, name: &str) -> bool {
    let matches = |callee: &str| {
        callee == name
            || [".", ":", "->"].iter().any(|sep| {
                callee
                    .strip_suffix(name)
                    .is_some_and(|head| head.ends_with(sep))
            })
    };
    matches(callee_raw) || strip_generic_args(callee_raw).is_some_and(|c| matches(&c))
}

/// The qualifier segment of a callee (`b` for `b.helper`, `util` for
/// `crate::util::helper`), with generic arguments removed. A receiver that is
/// not a plain identifier (`B().helper`, `super().helper`) is returned whole,
/// so it is never mistaken for an unqualified call.
pub fn callee_qualifier(callee_raw: &str) -> Option<String> {
    let callee = strip_generic_args(callee_raw).unwrap_or_else(|| callee_raw.to_string());
    let end = callee.rfind(['.', ':', '>'])?;
    let head = callee[..end].trim_end_matches([':', '-', '?', '.']);
    let qualifier: String = head
        .chars()
        .rev()
        .take_while(|c| is_ident_char(*c))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Some(if qualifier.is_empty() {
        head.to_string()
    } else {
        qualifier
    })
}

/// Removes balanced `<...>` generic argument lists, together with a turbofish
/// `::` that introduces one. Returns `None` when there is nothing to strip or
/// the angle brackets do not balance (a comparison such as `items[i < j]`).
fn strip_generic_args(callee: &str) -> Option<String> {
    if !callee.contains('<') {
        return None;
    }
    let mut out = String::with_capacity(callee.len());
    let mut depth = 0usize;
    let mut prev = '\0';
    for c in callee.chars() {
        match c {
            '<' => {
                if depth == 0 && out.ends_with("::") {
                    out.truncate(out.len() - 2);
                }
                depth += 1;
            }
            '>' if prev == '-' => {
                if depth == 0 {
                    out.push(c);
                }
            }
            '>' if depth > 0 => depth -= 1,
            '>' => return None,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
        prev = c;
    }
    (depth == 0).then_some(out)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Byte offsets of whole-identifier occurrences of `name` in `text`.
pub fn word_occurrences(text: &str, name: &str) -> Vec<usize> {
    if name.is_empty() {
        return Vec::new();
    }
    text.match_indices(name)
        .filter(|(start, _)| {
            let before = text[..*start].chars().next_back();
            let after = text[start + name.len()..].chars().next();
            !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
        })
        .map(|(start, _)| start)
        .collect()
}

/// The identifier immediately qualifying the occurrence at `start` in `code`.
/// A single `:` is a qualifier separator only in Lua (`obj:method`); elsewhere
/// it is a type annotation or map key.
fn qualifier_before(code: &str, start: usize, lang: Lang) -> Option<&str> {
    let head = code[..start].trim_end();
    let head = if let Some(h) = head.strip_suffix("::") {
        h
    } else if let Some(h) = head.strip_suffix("->") {
        h
    } else if let Some(h) = head.strip_suffix('.') {
        h.strip_suffix('?').unwrap_or(h)
    } else if lang == Lang::Lua {
        head.strip_suffix(':')?
    } else {
        return None;
    };
    let head = head.trim_end();
    let qualifier_start = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident_char(*c))
        .last()
        .map(|(i, _)| i);
    // A receiver that is not a plain identifier (`B().helper`, `xs[0].helper`)
    // is reported as its closing character, so it is never mistaken for an
    // unqualified reference.
    Some(match qualifier_start {
        Some(i) => &head[i..],
        None => head.get(head.len().saturating_sub(1)..).unwrap_or_default(),
    })
}

/// Byte offsets where statements start in blanked code text: after a `;` or
/// a newline at bracket depth zero. Brackets keep multi-line imports such as
/// `from x import (\n a as b,\n)` in one statement.
fn statement_starts(code: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let mut depth = 0usize;
    for (i, b) in code.bytes().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b';' | b'\n' if depth == 0 => starts.push(i + 1),
            _ => {}
        }
    }
    starts
}

fn is_import_statement(code: &str) -> bool {
    let t = code.trim_start();
    let t = t.strip_prefix("pub").map_or(t, |rest| {
        let rest = rest.trim_start();
        rest.strip_prefix("(crate)").unwrap_or(rest).trim_start()
    });
    ["import", "from", "use", "export"].iter().any(|kw| {
        t.strip_prefix(kw)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace) || rest.starts_with('{'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kgr_test_support::assert_empty;

    fn scan(src: &str, lang: Lang) -> ScannedSource {
        let path = match lang {
            Lang::Python => "a.py",
            Lang::JavaScript => "a.jsx",
            Lang::TypeScript => "a.tsx",
            Lang::Rust => "a.rs",
            Lang::C => "a.c",
            Lang::Cpp => "a.cpp",
            Lang::Ruby => "a.rb",
            Lang::Lua => "a.lua",
            Lang::Bash => "a.sh",
            Lang::Swift => "a.swift",
            Lang::Haskell => "a.hs",
            _ => "a.txt",
        };
        ScannedSource::new(src.to_string(), lang, Path::new(path))
    }

    fn code_mentions(src: &str, lang: Lang, name: &str) -> usize {
        scan(src, lang)
            .occurrences(name)
            .iter()
            .filter(|o| o.region == Region::Code)
            .count()
    }

    #[test]
    fn callee_matches_bare_and_qualified() {
        assert!(callee_matches("helper", "helper"));
        assert!(callee_matches("obj.helper", "helper"));
        assert!(callee_matches("util::helper", "helper"));
        assert!(callee_matches("p->m", "m"));
        assert!(callee_matches("obj:m", "m"));
    }

    #[test]
    fn callee_matches_ignores_generic_args() {
        assert!(callee_matches("parse_as::<u32>", "parse_as"));
        assert!(callee_matches("Vec::<u8>::new", "new"));
        assert!(callee_matches("make<T>", "make"));
        assert!(callee_matches("p->get<int>", "get"));
    }

    #[test]
    fn callee_matches_keeps_comparisons_intact() {
        assert!(callee_matches("items[i < j].helper", "helper"));
        assert!(callee_matches("a[i > j].helper", "helper"));
    }

    #[test]
    fn callee_matches_rejects_suffix_overlap() {
        assert!(!callee_matches("my_helper", "helper"));
        assert!(!callee_matches("obj.my_helper", "helper"));
        assert!(!callee_matches("p->xm", "m"));
    }

    #[test]
    fn callee_qualifier_extracts_last_segment() {
        assert_eq!(callee_qualifier("b.helper").as_deref(), Some("b"));
        assert_eq!(
            callee_qualifier("crate::util::helper").as_deref(),
            Some("util")
        );
        assert_eq!(callee_qualifier("self.step").as_deref(), Some("self"));
        assert_eq!(callee_qualifier("p->m").as_deref(), Some("p"));
        assert_eq!(callee_qualifier("helper"), None);
    }

    #[test]
    fn word_occurrences_respects_identifier_boundaries() {
        assert_eq!(word_occurrences("map(callback, [1])", "callback"), vec![4]);
        assert_empty!(word_occurrences("my_callback()", "callback"));
        assert_empty!(word_occurrences("callback_fn()", "callback"));
        assert_eq!(word_occurrences("<Button/>", "Button"), vec![1]);
        assert_eq!(word_occurrences("a(x); a(y)", "a").len(), 2);
    }

    #[test]
    fn strings_and_comments_are_not_code() {
        assert_eq!(
            code_mentions("const s = \"internal()\";\n", Lang::TypeScript, "internal"),
            0
        );
        assert_eq!(
            code_mentions("// helper()\nfn x() {}\n", Lang::Rust, "helper"),
            0
        );
        assert_eq!(code_mentions("# helper()\n", Lang::Python, "helper"), 0);
        assert_eq!(code_mentions("-- helper()\n", Lang::Lua, "helper"), 0);
        assert_eq!(
            code_mentions(
                "/*\nhelper is obsolete\n*/\nfn x() {}\n",
                Lang::Rust,
                "helper"
            ),
            0
        );
        assert_eq!(
            code_mentions(
                "const msg = `helper is obsolete`;\n",
                Lang::TypeScript,
                "helper"
            ),
            0
        );
        assert_eq!(
            code_mentions("x = 'he said \\'helper\\''\n", Lang::Python, "helper"),
            0
        );
    }

    #[test]
    fn code_after_comments_and_derefs_is_code() {
        assert_eq!(
            code_mentions("/* setup */ let cb = helper;\n", Lang::Rust, "helper"),
            1
        );
        assert_eq!(
            code_mentions("    * slot = helper;\n", Lang::C, "helper"),
            1
        );
        assert_eq!(
            code_mentions("helper() // trailing\n", Lang::Rust, "helper"),
            1
        );
        assert_eq!(
            code_mentions("#[derive(helper)]\n", Lang::Rust, "helper"),
            1
        );
    }

    #[test]
    fn interpolation_is_code() {
        assert_eq!(
            code_mentions(
                "print(f\"{callbacks.append(helper)}\")\n",
                Lang::Python,
                "helper"
            ),
            1
        );
        assert_eq!(
            code_mentions("print(f\"{{helper}}\")\n", Lang::Python, "helper"),
            0
        );
        assert_eq!(
            code_mentions("const s = `${helper()}`;\n", Lang::TypeScript, "helper"),
            1
        );
        assert_eq!(
            code_mentions("puts \"#{helper}\"\n", Lang::Ruby, "helper"),
            1
        );
    }

    #[test]
    fn char_literals_and_lifetimes() {
        assert_eq!(
            code_mentions("let quote = '\"'; let cb = helper;\n", Lang::Rust, "helper"),
            1
        );
        assert_eq!(
            code_mentions("fn f<'a>(x: &'a str) { helper(x) }\n", Lang::Rust, "helper"),
            1
        );
        assert_eq!(
            code_mentions("let s = r#\"helper\"#;\n", Lang::Rust, "helper"),
            0
        );
        assert_eq!(
            code_mentions("go = foldl' helper 0\n", Lang::Haskell, "helper"),
            1
        );
    }

    #[test]
    fn malformed_literals_stay_code() {
        // Unterminated strings and comments are parse errors, so their text
        // stays code.
        assert_eq!(
            code_mentions("x = \"oops; helper()\n", Lang::Python, "helper"),
            1
        );
        assert_eq!(
            code_mentions("/* never closed\nhelper()\n", Lang::Rust, "helper"),
            1
        );
        // A genuine multi-line Ruby string is literal text, per the grammar.
        assert_eq!(
            code_mentions("x = \"a\nhelper()\n\"\n", Lang::Ruby, "helper"),
            0
        );
        // A quote right after an identifier is prose, not an opener.
        assert_eq!(
            code_mentions("<p>it's <Helper/> isn't</p>\n", Lang::JavaScript, "Helper"),
            1
        );
    }

    #[test]
    fn language_specific_literals() {
        assert_eq!(
            code_mentions("let q = b'\"'; helper();\n", Lang::Rust, "helper"),
            1
        );
        assert_eq!(
            code_mentions("/* a /* b */ \" */ helper();\n", Lang::Rust, "helper"),
            1
        );
        assert_eq!(
            code_mentions("s = r\"\\\"\"; helper()\n", Lang::Python, "helper"),
            1
        );
        assert_eq!(
            code_mentions("auto s = R\"t(\" /*)t\"; helper();\n", Lang::Cpp, "helper"),
            1
        );
        assert_eq!(
            code_mentions("local s = [==[ helper ]==]\n", Lang::Lua, "helper"),
            0
        );
        assert_eq!(
            code_mentions("--[[ helper ]] helper()\n", Lang::Lua, "helper"),
            1
        );
        assert_eq!(code_mentions("s=$'\\''; helper\n", Lang::Bash, "helper"), 1);
        assert_eq!(
            code_mentions("let s = \"\\(helper())\"\n", Lang::Swift, "helper"),
            1
        );
        assert_eq!(
            code_mentions("let s = \"helper\"\n", Lang::Swift, "helper"),
            0
        );
    }

    #[test]
    fn qualifier_before_keeps_non_identifier_receivers() {
        let s = scan("    return B().helper\n", Lang::Python);
        assert_eq!(
            s.qualifier_before(s.occurrences("helper")[0].offset),
            Some(")")
        );
    }

    #[test]
    fn callee_qualifier_keeps_non_identifier_receivers() {
        assert_eq!(callee_qualifier("B().helper").as_deref(), Some("B()"));
        assert_eq!(
            callee_qualifier("super().helper").as_deref(),
            Some("super()")
        );
    }

    #[test]
    fn positions_are_line_and_byte_column() {
        let scanned = scan("a\n  helper()\n", Lang::Python);
        let occ = scanned.occurrences("helper");
        assert_eq!((occ[0].line, occ[0].col), (2, 2));
    }

    #[test]
    fn qualifier_before_is_language_aware() {
        let s = scan("    used.helper()\n", Lang::Python);
        assert_eq!(
            s.qualifier_before(s.occurrences("helper")[0].offset),
            Some("used")
        );
        let s = scan("crate::util::helper();\n", Lang::Rust);
        assert_eq!(
            s.qualifier_before(s.occurrences("helper")[0].offset),
            Some("util")
        );
        let s = scan("registry = {other: helper}\n", Lang::Python);
        assert_eq!(s.qualifier_before(s.occurrences("helper")[0].offset), None);
        let s = scan("Obj:helper()\n", Lang::Lua);
        assert_eq!(
            s.qualifier_before(s.occurrences("helper")[0].offset),
            Some("Obj")
        );
    }

    fn aliases(src: &str, lang: Lang, name: &str, hints: &[&str]) -> Vec<String> {
        let hints: Vec<String> = hints.iter().map(|h| h.to_string()).collect();
        scan(src, lang).import_aliases(name, &hints)
    }

    #[test]
    fn import_aliases_found_in_import_statements() {
        assert_eq!(
            aliases(
                "from pkg.util import helper as h\nh()\n",
                Lang::Python,
                "helper",
                &[]
            ),
            vec!["h"]
        );
        assert_eq!(
            aliases(
                "import { add as plus } from './math';",
                Lang::TypeScript,
                "add",
                &["math"]
            ),
            vec!["plus"]
        );
        assert_eq!(
            aliases(
                "use crate::util::helper as h;",
                Lang::Rust,
                "helper",
                &["util"]
            ),
            vec!["h"]
        );
        assert_eq!(
            aliases(
                "from util import (\n    helper as h,\n)\nh()\n",
                Lang::Python,
                "helper",
                &["util"]
            ),
            vec!["h"]
        );
        assert_eq!(
            aliases(
                "import {\n  add as plus,\n} from \"./lib/math\";\n",
                Lang::TypeScript,
                "add",
                &["math"]
            ),
            vec!["plus"]
        );
    }

    #[test]
    fn import_aliases_ignore_non_imports() {
        assert_empty!(aliases("let assets = helper;", Lang::Rust, "helper", &[]));
        assert_empty!(aliases("helper.as_ref()", Lang::Rust, "helper", &[]));
        assert_empty!(aliases(
            "q = \"SELECT helper as unrelated FROM util\"\n",
            Lang::Python,
            "helper",
            &[]
        ));
        assert_empty!(aliases(
            "# util.helper as unrelated\n",
            Lang::Python,
            "helper",
            &[]
        ));
        assert_empty!(aliases(
            "with helper as h:\n    pass\n",
            Lang::Python,
            "helper",
            &[]
        ));
        assert_empty!(aliases(
            "const x = helper as Foo;\n",
            Lang::TypeScript,
            "helper",
            &[]
        ));
    }

    #[test]
    fn import_aliases_honour_module_hints() {
        let src =
            "import { add as plus } from './math';\nimport { add as cat } from './strings';\n";
        assert_eq!(
            aliases(src, Lang::TypeScript, "add", &["math"]),
            vec!["plus"]
        );
        assert_eq!(
            aliases(src, Lang::TypeScript, "add", &["strings"]),
            vec!["cat"]
        );
    }
}

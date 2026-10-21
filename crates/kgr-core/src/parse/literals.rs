//! String-literal and comment ranges from a tree-sitter parse tree.
//!
//! Used by reference scanning to tell a name mentioned in code from one that
//! only appears in a string or a comment. The classification is fail-safe:
//! a range is reported only when the grammar parsed it cleanly as a comment
//! or a string-like literal, and any named child of a literal that is not
//! itself literal text (interpolation, substitution, expansion) is left out,
//! so the code inside `f"{x}"`, `` `${x}` `` or `"$(x)"` stays code. Nothing
//! inside an `ERROR` node is reported.

use tree_sitter::{Node, Tree};

/// Lexical class of a non-code byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralKind {
    Str,
    Comment,
}

/// A half-open byte range `start..end` of literal text or comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiteralRange {
    pub start: usize,
    pub end: usize,
    pub kind: LiteralKind,
}

/// String-literal and comment ranges of `tree`, in source order.
pub fn literal_ranges(tree: &Tree) -> Vec<LiteralRange> {
    let mut out = Vec::new();
    visit_code(tree.root_node(), &mut out);
    out
}

fn is_comment(kind: &str) -> bool {
    kind.contains("comment")
}

/// Node kinds that are string-like literals across the supported grammars:
/// `string`, `string_literal`, `template_string`, `raw_string_literal`,
/// `interpolated_string_expression`, `char_literal`, `rune_literal`,
/// `heredoc_body`, `regex`, `jsx_text`, `text_block`, `charlist`, ...
fn is_literal(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    if kind.contains("type") {
        return false;
    }
    kind.contains("string")
        || kind.contains("char_literal")
        || kind.contains("character_literal")
        || kind.contains("rune_literal")
        || kind == "char"
        || kind.contains("heredoc")
        || kind.contains("nowdoc")
        || kind.contains("regex")
        || kind == "jsx_text"
        || kind == "text_block"
        || kind == "charlist"
}

/// Named children of a literal that are part of its text rather than code.
fn is_literal_part(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    [
        "string",
        "content",
        "fragment",
        "escape",
        "char",
        "start",
        "end",
        "delimiter",
        "quote",
        "heredoc",
        "regex",
        "text",
        "flags",
        "pattern",
    ]
    .iter()
    .any(|part| kind.contains(part))
}

fn visit_code(node: Node<'_>, out: &mut Vec<LiteralRange>) {
    if node.is_error() || node.is_missing() {
        return;
    }
    let kind = node.kind();
    if node.is_named() && is_comment(kind) {
        if !node.has_error() {
            push(
                out,
                node.start_byte(),
                node.end_byte(),
                LiteralKind::Comment,
            );
        }
        return;
    }
    if node.is_named() && is_literal(kind) && !node.has_error() {
        let mut cursor = node.start_byte();
        visit_literal(node, &mut cursor, out);
        push(out, cursor, node.end_byte(), LiteralKind::Str);
        return;
    }
    let mut walker = node.walk();
    for child in node.children(&mut walker) {
        visit_code(child, out);
    }
}

/// Walks the children of a literal, emitting literal text up to each child
/// that is code and scanning that child as code.
fn visit_literal(node: Node<'_>, cursor: &mut usize, out: &mut Vec<LiteralRange>) {
    let mut walker = node.walk();
    for child in node.children(&mut walker) {
        if child.is_named() && !is_literal_part(child.kind()) {
            push(out, *cursor, child.start_byte(), LiteralKind::Str);
            visit_code(child, out);
            *cursor = child.end_byte();
        } else if child.child_count() > 0 {
            visit_literal(child, cursor, out);
        }
    }
}

fn push(out: &mut Vec<LiteralRange>, start: usize, end: usize, kind: LiteralKind) {
    if start < end {
        out.push(LiteralRange { start, end, kind });
    }
}

#[cfg(test)]
mod tests {
    use crate::parse::ParserRegistry;
    use crate::types::Lang;
    use std::path::Path;

    use super::LiteralKind;

    /// The classification of the `nth` occurrence of `needle` in `src`:
    /// `None` for code.
    fn class_of(
        lang: Lang,
        path: &str,
        src: &str,
        needle: &str,
        nth: usize,
    ) -> Option<LiteralKind> {
        let ranges = ParserRegistry::new()
            .literal_ranges(lang, src.as_bytes(), Path::new(path))
            .expect("grammar available");
        let offset = src
            .match_indices(needle)
            .nth(nth)
            .expect("needle present")
            .0;
        ranges
            .iter()
            .find(|r| r.start <= offset && offset < r.end)
            .map(|r| r.kind)
    }

    #[test]
    fn python_strings_comments_and_fstring_fields() {
        let src = "x = \"helper\"  # helper\ny = f\"{helper()}\"\nz = rf\"\\{helper.__name__}\"\n";
        assert_eq!(
            class_of(Lang::Python, "a.py", src, "helper", 0),
            Some(LiteralKind::Str)
        );
        assert_eq!(
            class_of(Lang::Python, "a.py", src, "helper", 1),
            Some(LiteralKind::Comment)
        );
        assert_eq!(class_of(Lang::Python, "a.py", src, "helper", 2), None);
        assert_eq!(class_of(Lang::Python, "a.py", src, "helper", 3), None);
    }

    #[test]
    fn javascript_regex_template_and_jsx() {
        let src = "const p = /[/*]/;\nconst cb = helper;\nconst t = `${helper()} helper`;\n";
        assert_eq!(class_of(Lang::JavaScript, "a.js", src, "helper", 0), None);
        assert_eq!(class_of(Lang::JavaScript, "a.js", src, "helper", 1), None);
        assert_eq!(
            class_of(Lang::JavaScript, "a.js", src, "helper", 2),
            Some(LiteralKind::Str)
        );
        let jsx = "const v = <div>Click \"<Helper />\" don't</div>;\n";
        assert_eq!(class_of(Lang::JavaScript, "a.jsx", jsx, "Helper", 0), None);
        let tsx = "const v = <div>say `hi`\n <Helper /> it's</div>;\n";
        assert_eq!(class_of(Lang::TypeScript, "a.tsx", tsx, "Helper", 0), None);
    }

    #[test]
    fn rust_chars_raw_strings_and_nested_comments() {
        let src = "fn main() { let q = b'\"'; let r = r#\"helper\"#; /* a /* b */ \" */ let cb = helper; }\n";
        assert_eq!(
            class_of(Lang::Rust, "a.rs", src, "helper", 0),
            Some(LiteralKind::Str)
        );
        assert_eq!(class_of(Lang::Rust, "a.rs", src, "helper", 1), None);
    }

    #[test]
    fn cpp_wide_chars_and_raw_strings() {
        let src = "int main() { wchar_t q = L'\"'; auto s = R\"t(\" /*)t\"; auto cb = helper; puts(\"x\"); }\n";
        assert_eq!(class_of(Lang::Cpp, "a.cpp", src, "helper", 0), None);
    }

    #[test]
    fn ruby_percent_literals_regex_and_interpolation() {
        let src = "w = %w[\"]; r = /\"/; cb = method(:helper); puts \"#{helper}\"\n";
        assert_eq!(class_of(Lang::Ruby, "a.rb", src, "helper", 0), None);
        assert_eq!(class_of(Lang::Ruby, "a.rb", src, "helper", 1), None);
    }

    #[test]
    fn bash_substitutions_are_code() {
        let src = "out=\"$(cb=helper; $cb)\"\ns='helper'\n";
        assert_eq!(class_of(Lang::Bash, "a.sh", src, "helper", 0), None);
        assert_eq!(
            class_of(Lang::Bash, "a.sh", src, "helper", 1),
            Some(LiteralKind::Str)
        );
    }

    #[test]
    fn swift_and_csharp_interpolation_are_code() {
        let swift = "let s = \"\"\"\nCount: \\([helper].count)\n\"\"\"\n";
        assert_eq!(class_of(Lang::Swift, "a.swift", swift, "helper", 0), None);
        let cs = "class C { string s = $\"{helper()}\"; string t = \"helper\"; }\n";
        assert_eq!(class_of(Lang::CSharp, "a.cs", cs, "helper", 0), None);
        assert_eq!(
            class_of(Lang::CSharp, "a.cs", cs, "helper", 1),
            Some(LiteralKind::Str)
        );
    }

    #[test]
    fn unterminated_string_is_not_reported() {
        let src = "x = \"oops\nhelper()\n";
        assert_eq!(class_of(Lang::Python, "a.py", src, "helper", 0), None);
    }
}

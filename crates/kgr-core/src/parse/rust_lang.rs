use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::types::{CallRef, Import, ImportKind, Lang, Span, Symbol, SymbolKind};

const RUST_QUERY_SRC: &str = r#"
;; use declaration: use std::collections::HashMap;
(use_declaration
  argument: (_) @import.use)

;; mod declaration: mod foo;
(mod_item
  name: (identifier) @import.mod)

;; extern crate: extern crate serde;
(extern_crate_declaration
  name: (identifier) @import.extern)
"#;

const RUST_SYMBOL_QUERY_SRC: &str = r#"
;; Pub function (must come before non-pub so dedup keeps exported)
(function_item
  (visibility_modifier)
  name: (identifier) @fn.exported) @def

;; Top-level function (non-pub)
(function_item
  name: (identifier) @fn.name) @def

;; Pub struct
(struct_item
  (visibility_modifier)
  name: (type_identifier) @class.exported) @def

;; Struct (non-pub)
(struct_item
  name: (type_identifier) @class.name) @def

;; Pub enum
(enum_item
  (visibility_modifier)
  name: (type_identifier) @class.exported) @def

;; Enum (non-pub)
(enum_item
  name: (type_identifier) @class.name) @def

;; Enum variants
(enum_variant
  name: (identifier) @variant.name) @def

;; Pub trait
(trait_item
  (visibility_modifier)
  name: (type_identifier) @class.exported) @def

;; Trait (non-pub)
(trait_item
  name: (type_identifier) @class.name) @def

;; Method inside impl block
(impl_item
  body: (declaration_list
    (function_item
      name: (identifier) @method.name) @def))

;; Required trait method (signature only): trait T { fn required(&self); }
(trait_item
  body: (declaration_list
    (function_signature_item
      name: (identifier) @method.name) @def))

;; Pub type alias: pub type Alias = u32;
(type_item
  (visibility_modifier)
  name: (type_identifier) @class.exported) @def

;; Type alias (non-pub)
(type_item
  name: (type_identifier) @class.name) @def

;; Pub union
(union_item
  (visibility_modifier)
  name: (type_identifier) @class.exported) @def

;; Union (non-pub)
(union_item
  name: (type_identifier) @class.name) @def

;; Macro definition: macro_rules! mymac { ... }
;; Exported-ness (#[macro_export]) is decided Rust-side from sibling attributes.
(macro_definition
  name: (identifier) @macro.name) @def
"#;

const RUST_CALL_QUERY_SRC: &str = r#"
;; Regular call: foo()
(call_expression
  function: (identifier) @call.name)

;; Scoped call: Foo::bar(), util::helper(), String::from()
(call_expression
  function: (scoped_identifier) @call.name)

;; Method call: obj.method()
(call_expression
  function: (field_expression
    field: (field_identifier) @call.method))

;; Macro invocation: println!()
(macro_invocation
  macro: (identifier) @call.macro)

;; Scoped macro invocation: tracing::warn!()
(macro_invocation
  macro: (scoped_identifier) @call.macro)

;; Type identifier in any position (annotations, fields, etc.)
(type_identifier) @type.ref

;; Tuple/struct enum variant patterns: Foo::Bar(x), Foo::Bar { x }, Bar(x)
(tuple_struct_pattern
  type: [(identifier) (scoped_identifier)] @type.ref)

(struct_pattern
  type: [(type_identifier) (scoped_type_identifier)] @type.ref)

;; Attribute paths: #[my_attr], #[path::my_attr], #[derive(MyDerive)]
(attribute
  [(identifier) (scoped_identifier)] @type.ref)

(attribute
  arguments: (token_tree
    [(identifier) (scoped_identifier)] @type.ref))

;; Trait bounds: impl MyTrait for Foo — trait names captured via type_identifier above
;; Generic type args: Vec<MyType> — inner types captured via type_identifier above
"#;

static RUST_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    Query::new(&language, RUST_QUERY_SRC).expect("Failed to compile Rust query")
});

static RUST_SYMBOL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    Query::new(&language, RUST_SYMBOL_QUERY_SRC).expect("Failed to compile Rust symbol query")
});

static RUST_CALL_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    Query::new(&language, RUST_CALL_QUERY_SRC).expect("Failed to compile Rust call query")
});

thread_local! {
    static RUST_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct RustParser;

fn parse_tree(source: &[u8], path: &Path) -> Option<tree_sitter::Tree> {
    RUST_PARSER.with(|parser| {
        let mut p = parser.borrow_mut();
        let tree = p.parse(source, None);
        if tree.is_none() {
            tracing::warn!("Failed to parse Rust file: {}", path.display());
        }
        tree
    })
}

impl super::Parser for RustParser {
    fn lang(&self) -> Lang {
        Lang::Rust
    }

    fn ts_language(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_rust::LANGUAGE.into())
    }

    fn extract_symbols(&self, source: &[u8], path: &Path) -> Vec<Symbol> {
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*RUST_SYMBOL_QUERY;
        let fn_name_idx = query.capture_index_for_name("fn.name").unwrap();
        let fn_exported_idx = query.capture_index_for_name("fn.exported").unwrap();
        let class_name_idx = query.capture_index_for_name("class.name").unwrap();
        let class_exported_idx = query.capture_index_for_name("class.exported").unwrap();
        let variant_idx = query.capture_index_for_name("variant.name").unwrap();
        let method_idx = query.capture_index_for_name("method.name").unwrap();
        let macro_idx = query.capture_index_for_name("macro.name").unwrap();
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

                let (kind, exported) = if capture.index == fn_exported_idx {
                    (SymbolKind::Function, true)
                } else if capture.index == fn_name_idx {
                    (SymbolKind::Function, false)
                } else if capture.index == class_exported_idx {
                    (SymbolKind::Class, true)
                } else if capture.index == class_name_idx || capture.index == variant_idx {
                    (SymbolKind::Class, false)
                } else if capture.index == method_idx {
                    let exported = node.parent().is_some_and(rust_decl_has_visibility);
                    (SymbolKind::Method, exported)
                } else if capture.index == macro_idx {
                    // macro_rules! macros are exported via #[macro_export],
                    // which sits as a sibling attribute_item, not a child.
                    let exported = node.parent().is_some_and(|d| macro_is_exported(d, source));
                    (SymbolKind::Function, exported)
                } else {
                    continue;
                };

                let span_node = def_node.unwrap_or(node);
                let start = span_node.start_position();
                let end = span_node.end_position();

                symbols.push(Symbol {
                    name,
                    kind,
                    exported,
                    span: Span {
                        start_line: start.row + 1,
                        start_col: start.column,
                        end_line: end.row + 1,
                        end_col: end.column,
                    },
                });
            }
        }

        // Deduplicate: exported match may also match non-exported pattern
        let mut seen = std::collections::HashSet::new();
        symbols.retain(|s| seen.insert((s.name.clone(), s.span.start_line)));

        symbols
    }

    fn extract_calls(&self, source: &[u8], path: &Path) -> Vec<CallRef> {
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*RUST_CALL_QUERY;
        let name_idx = query.capture_index_for_name("call.name").unwrap();
        let method_idx = query.capture_index_for_name("call.method").unwrap();
        let macro_idx = query.capture_index_for_name("call.macro").unwrap();
        let type_ref_idx = query.capture_index_for_name("type.ref").unwrap();

        let mut cursor = QueryCursor::new();
        let mut calls = Vec::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;

                let callee_raw = if capture.index == name_idx || capture.index == macro_idx {
                    match node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == method_idx {
                    let field_node = node.parent().unwrap();
                    match field_node.utf8_text(source) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    }
                } else if capture.index == type_ref_idx {
                    // Type reference
                    match node.utf8_text(source) {
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
        let tree = match parse_tree(source, path) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let query = &*RUST_QUERY;
        let use_idx = query.capture_index_for_name("import.use").unwrap();
        let mod_idx = query.capture_index_for_name("import.mod").unwrap();
        let _extern_idx = query.capture_index_for_name("import.extern").unwrap();

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

                let start = node.start_position();
                let end = node.end_position();
                let span = Span {
                    start_line: start.row + 1,
                    start_col: start.column,
                    end_line: end.row + 1,
                    end_col: end.column,
                };

                // A single `use` may pull in several paths via brace groups:
                // `use a::b::{c, d};` -> a::b::c, a::b::d. Expand so each path
                // resolves independently and render projections receive clean,
                // brace-free import paths. Paths inside inline modules
                // (`mod tests { use super::*; }`) are rebased against the
                // inline-module chain so self/super point at the right module.
                if capture.index == use_idx {
                    let chain = inline_module_chain(node, source);
                    for path in expand_use_paths(&raw) {
                        let path = rebase_use_path(&path, &chain);
                        if !seen.insert(path.clone()) {
                            continue;
                        }
                        let kind = if path.starts_with("crate::")
                            || path.starts_with("super::")
                            || path.starts_with("self::")
                            || matches!(path.as_str(), "crate" | "super" | "self")
                        {
                            ImportKind::Local
                        } else {
                            ImportKind::External
                        };
                        imports.push(Import {
                            raw: path,
                            kind,
                            resolved: None,
                            span: Some(span),
                        });
                    }
                    continue;
                }

                // mod declarations with a body (mod foo { ... }) are inline, skip them
                if capture.index == mod_idx {
                    let Some(mod_item) = node.parent() else {
                        continue;
                    };
                    if mod_item.child_by_field_name("body").is_some() {
                        continue;
                    }
                    // `#[path = "..."]` wins outright. Otherwise a declaration
                    // nested in inline modules (`mod outer { mod inner; }`)
                    // targets the inline chain's directory: self::outer::inner.
                    let chain = inline_module_chain(node, source);
                    let raw = match mod_path_attribute(mod_item, source) {
                        Some(path) => path,
                        None if chain.is_empty() => raw,
                        None => format!("self::{}::{raw}", chain.join("::")),
                    };
                    if !seen.insert(raw.clone()) {
                        continue;
                    }
                    imports.push(Import {
                        raw,
                        kind: ImportKind::Local,
                        resolved: None,
                        span: Some(span),
                    });
                    continue;
                }

                // extern crate
                if !seen.insert(raw.clone()) {
                    continue;
                }
                imports.push(Import {
                    raw,
                    kind: ImportKind::External,
                    resolved: None,
                    span: Some(span),
                });
            }
        }

        imports
    }
}

/// Read a `#[path = "..."]` attribute attached to a `mod_item`, if present.
/// Like other Rust attributes in tree-sitter, it appears as a sibling directly
/// before the item it annotates.
fn mod_path_attribute(mod_item: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut sibling = mod_item.prev_sibling();
    while let Some(node) = sibling {
        match node.kind() {
            "attribute_item" => {
                if let Ok(text) = node.utf8_text(source) {
                    if let Some(path) = path_attribute_value(text) {
                        return Some(path);
                    }
                }
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        sibling = node.prev_sibling();
    }
    None
}

fn path_attribute_value(attribute: &str) -> Option<String> {
    let inner = attribute
        .trim()
        .strip_prefix("#[")?
        .strip_suffix(']')?
        .trim();
    let rest = inner.strip_prefix("path")?;
    if rest
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    let value = rest.trim_start().strip_prefix('=')?.trim_start();
    read_rust_string_literal(value)
}

fn read_rust_string_literal(value: &str) -> Option<String> {
    let value = value.trim_start();
    if let Some(rest) = value.strip_prefix('"') {
        return read_quoted_rust_string(rest);
    }
    read_raw_rust_string(value)
}

fn read_quoted_rust_string(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            match ch {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '0' => out.push('\0'),
                other => out.push(other),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn read_raw_rust_string(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.first() != Some(&b'r') {
        return None;
    }

    let mut pos = 1usize;
    while bytes.get(pos) == Some(&b'#') {
        pos += 1;
    }
    if bytes.get(pos) != Some(&b'"') {
        return None;
    }
    pos += 1;

    let hashes = "#".repeat(pos - 2);
    let terminator = format!("\"{hashes}");
    let rest = &value[pos..];
    let end = rest.find(&terminator)?;
    Some(rest[..end].to_string())
}

fn rust_decl_has_visibility(decl_node: tree_sitter::Node) -> bool {
    (0..decl_node.child_count()).any(|i| {
        decl_node
            .child(i)
            .is_some_and(|child| child.kind() == "visibility_modifier")
    })
}

/// True if a `macro_definition` node is preceded by a `#[macro_export]`
/// attribute. Attributes are siblings of the item they annotate, so walk
/// backwards over any attribute_items (and doc comments) directly above it.
fn macro_is_exported(macro_def: tree_sitter::Node, source: &[u8]) -> bool {
    let mut sibling = macro_def.prev_sibling();
    while let Some(node) = sibling {
        match node.kind() {
            "attribute_item" => {
                if let Ok(text) = node.utf8_text(source) {
                    if text.contains("macro_export") {
                        return true;
                    }
                }
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        sibling = node.prev_sibling();
    }
    false
}

/// Names of the inline (`mod name { ... }`) module ancestors enclosing `node`,
/// outermost first. File-backed `mod name;` declarations never enclose other
/// items, so only mod_items WITH a body count.
fn inline_module_chain(node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if ancestor.kind() == "mod_item" && ancestor.child_by_field_name("body").is_some() {
            if let Some(name) = ancestor
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
            {
                chain.push(name.to_string());
            }
        }
        current = ancestor.parent();
    }
    chain.reverse();
    chain
}

/// Rebase a use path written inside inline modules onto the file's own module.
///
/// Inside `mod tests { use super::*; }` the `super` names the FILE's module,
/// not its parent — taking it literally would draw a phantom edge to the
/// parent module file. Each leading `super::` pops one inline ancestor;
/// whatever remains is re-expressed relative to the file (`self::...`), and
/// only supers that outlive the chain keep pointing above the file.
/// `crate::` and bare 2018-edition paths are absolute and pass through.
fn rebase_use_path(path: &str, chain: &[String]) -> String {
    if chain.is_empty() {
        return path.to_string();
    }
    if let Some(rest) = path.strip_prefix("self::") {
        return format!("self::{}::{rest}", chain.join("::"));
    }
    if path == "self" {
        // `use super::x::{self}` style: names the innermost inline module,
        // which still lives in this very file.
        return format!("self::{}", chain.join("::"));
    }

    let mut supers = 0usize;
    let mut rest = path;
    while let Some(stripped) = rest.strip_prefix("super::") {
        supers += 1;
        rest = stripped;
    }
    if rest == "super" {
        supers += 1;
        rest = "";
    }
    if supers == 0 {
        return path.to_string();
    }

    let popped = supers.min(chain.len());
    let remaining_supers = supers - popped;
    let mut parts: Vec<&str> = if remaining_supers > 0 {
        vec!["super"; remaining_supers]
    } else {
        let mut kept: Vec<&str> = vec!["self"];
        kept.extend(chain[..chain.len() - popped].iter().map(String::as_str));
        kept
    };
    if !rest.is_empty() {
        parts.push(rest);
    }
    parts.join("::")
}

/// Expand a Rust `use` argument into individual import paths, distributing the
/// shared prefix across brace groups: `a::b::{c, d}` -> `["a::b::c", "a::b::d"]`.
/// Aliases (`as x`) are stripped and globs (`*`) are kept as a trailing segment.
fn expand_use_paths(arg: &str) -> Vec<String> {
    let arg = arg.trim();
    let Some(open) = arg.find('{') else {
        return vec![strip_alias(arg)];
    };
    let prefix = &arg[..open];
    let Some(close) = matching_brace(&arg[open..]) else {
        return vec![strip_alias(arg)];
    };
    let inner = &arg[open + 1..open + close];

    let mut out = Vec::new();
    for member in split_top_level(inner) {
        let member = member.trim();
        if member.is_empty() {
            continue;
        }
        if member == "self" {
            // `a::b::{self, ...}` re-exports the module `a::b` itself.
            out.push(prefix.trim_end_matches("::").to_string());
        } else if member.contains('{') {
            out.extend(expand_use_paths(&format!("{prefix}{member}")));
        } else {
            out.push(strip_alias(&format!("{prefix}{member}")));
        }
    }
    if out.is_empty() {
        out.push(prefix.trim_end_matches("::").to_string());
    }
    out
}

fn strip_alias(path: &str) -> String {
    match path.trim().split_once(" as ") {
        Some((head, _)) => head.trim().to_string(),
        None => path.trim().to_string(),
    }
}

/// Byte index (relative to the leading `{`) of its matching `}`.
fn matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split on top-level commas, ignoring commas nested inside brace groups.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn parse(src: &str) -> Vec<Import> {
        RustParser.parse(src.as_bytes(), Path::new("lib.rs"))
    }

    #[test]
    fn parse_errors_flags_malformed_source() {
        let errors = RustParser.parse_errors(b"fn broken(:", Path::new("broken.rs"));
        assert!(
            !errors.is_empty(),
            "expected syntax errors for malformed rust source"
        );
    }

    #[test]
    fn parse_errors_clean_on_valid_source() {
        let errors = RustParser.parse_errors(b"fn main() {}\n", Path::new("ok.rs"));
        assert!(errors.is_empty());
    }

    #[test]
    fn use_external() {
        let imports = parse("use std::collections::HashMap;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn use_crate_local() {
        let imports = parse("use crate::utils::helper;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn mod_declaration() {
        let imports = parse("mod utils;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "utils");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn path_attribute_mod_declaration_uses_attribute_path() {
        let imports = parse(
            r#"
#[path = "other/dir/foo.rs"]
mod foo;
"#,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "other/dir/foo.rs");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn extern_crate() {
        let imports = parse("extern crate serde;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "serde");
        assert_eq!(imports[0].kind, ImportKind::External);
    }

    #[test]
    fn inline_mod_ignored() {
        let imports = parse("mod tests { fn foo() {} }");
        // inline mod with body should be skipped
        assert!(imports.is_empty());
    }

    #[test]
    fn multiple() {
        let imports = parse(
            r#"
use std::io;
use crate::config::Settings;
mod parser;
extern crate log;
"#,
        );
        assert_eq!(imports.len(), 4);
    }

    #[test]
    fn grouped_use_expands_to_individual_paths() {
        let imports = parse("use std::collections::{HashMap, HashSet};");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(
            raws,
            ["std::collections::HashMap", "std::collections::HashSet"]
        );
        assert!(imports.iter().all(|i| i.kind == ImportKind::External));
        // No brace groups leak into raw import strings.
        assert!(imports.iter().all(|i| !i.raw.contains('{')));
    }

    #[test]
    fn grouped_crate_local_each_classified_local() {
        let imports = parse("use crate::{config, rules};");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, ["crate::config", "crate::rules"]);
        assert!(imports.iter().all(|i| i.kind == ImportKind::Local));
    }

    #[test]
    fn bare_module_import_is_external_until_resolved() {
        // 2018-edition `use <local_mod>::Item;` (the ../itr smell). At parse
        // time it's External; the resolver upgrades it to Local once the local
        // module file is found.
        let imports = parse("use cli::{Command, Flag};");
        assert_eq!(
            imports.iter().map(|i| i.raw.as_str()).collect::<Vec<_>>(),
            ["cli::Command", "cli::Flag"]
        );
        assert!(imports.iter().all(|i| i.kind == ImportKind::External));
    }

    #[test]
    fn use_alias_is_stripped() {
        let imports = parse("use foo::Bar as Baz;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "foo::Bar");
    }

    #[test]
    fn group_self_member_keeps_module_path() {
        let imports = parse("use crate::config::{self, Settings};");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, ["crate::config", "crate::config::Settings"]);
    }

    #[test]
    fn test_module_super_glob_rebases_to_self() {
        // `use super::*;` inside `mod tests` names THIS file's module, not the
        // file's parent — it must not look like a parent-module dependency.
        let imports = parse("mod tests {\n    use super::*;\n}\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "self::*");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn test_module_super_items_rebase_to_self() {
        let imports = parse("mod tests {\n    use super::{helper, Config};\n}\n");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, ["self::helper", "self::Config"]);
        assert!(imports.iter().all(|i| i.kind == ImportKind::Local));
    }

    #[test]
    fn nested_inline_modules_rebase_supers_one_level_each() {
        let imports = parse(
            r#"
mod outer {
    mod inner {
        use super::super::Top;
        use super::Mid;
        use crate::cfg::Z;
        use serde::Serialize;
    }
}
"#,
        );
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(
            raws,
            [
                "self::Top",
                "self::outer::Mid",
                "crate::cfg::Z",
                "serde::Serialize"
            ]
        );
    }

    #[test]
    fn super_beyond_inline_chain_keeps_pointing_above_the_file() {
        // One inline level, two supers: the second super escapes the file.
        let imports = parse("mod tests {\n    use super::super::shared::util;\n}\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "super::shared::util");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn self_paths_inside_inline_module_gain_the_chain_prefix() {
        let imports = parse("mod outer {\n    use self::detail::Inner;\n}\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "self::outer::detail::Inner");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn nested_mod_declaration_targets_the_inline_chain_dir() {
        let imports = parse("mod outer {\n    mod inner;\n}\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "self::outer::inner");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn glob_reexport_keeps_trailing_star_segment() {
        let imports = parse("pub use crate::models::*;\n");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "crate::models::*");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn pub_use_reexports_are_captured_like_plain_use() {
        let imports = parse("pub use crate::a::Thing;\npub use super::b::Other;\n");
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert_eq!(raws, ["crate::a::Thing", "super::b::Other"]);
        assert!(imports.iter().all(|i| i.kind == ImportKind::Local));
    }

    // ── Symbol extraction tests ──────────────────────────────────────────

    fn symbols(src: &str) -> Vec<Symbol> {
        RustParser.extract_symbols(src.as_bytes(), Path::new("lib.rs"))
    }

    #[test]
    fn symbols_finds_functions() {
        let syms = symbols("fn foo() {}\nfn bar() {}\n");
        let fns: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, "foo");
        assert_eq!(fns[1].name, "bar");
    }

    #[test]
    fn symbols_finds_structs() {
        let syms = symbols("struct MyStruct { x: i32 }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "MyStruct" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_enums() {
        let syms = symbols("enum Color { Red, Green, Blue }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_finds_enum_variants() {
        let syms = symbols("enum Flow { Ready(String), Waiting { id: u32 }, Done }\n");
        for variant in ["Ready", "Waiting", "Done"] {
            assert!(
                syms.iter()
                    .any(|s| s.name == variant && s.kind == SymbolKind::Class),
                "missing enum variant symbol {variant}"
            );
        }
    }

    #[test]
    fn symbols_finds_traits() {
        let syms = symbols("trait Drawable { fn draw(&self); }\n");
        assert!(syms
            .iter()
            .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn symbols_exported_pub() {
        let syms = symbols("pub fn public_fn() {}\nfn private_fn() {}\n");
        let public = syms.iter().find(|s| s.name == "public_fn").unwrap();
        let private = syms.iter().find(|s| s.name == "private_fn").unwrap();
        assert!(public.exported);
        assert!(!private.exported);
    }

    #[test]
    fn symbols_finds_methods() {
        let syms = symbols(
            "struct Foo;\nimpl Foo {\n    fn method_a(&self) {}\n    fn method_b(&self) {}\n}\n",
        );
        let methods: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn symbols_exports_pub_impl_methods() {
        let syms = symbols(
            "struct Foo;\nimpl Foo {\n    pub fn method_a(&self) {}\n    pub(crate) fn method_b(&self) {}\n    fn method_c(&self) {}\n}\n",
        );
        let public = syms.iter().find(|s| s.name == "method_a").unwrap();
        let crate_visible = syms.iter().find(|s| s.name == "method_b").unwrap();
        let private = syms.iter().find(|s| s.name == "method_c").unwrap();

        assert_eq!(public.kind, SymbolKind::Method);
        assert!(public.exported);
        assert_eq!(crate_visible.kind, SymbolKind::Method);
        assert!(crate_visible.exported);
        assert_eq!(private.kind, SymbolKind::Method);
        assert!(!private.exported);
    }

    #[test]
    fn symbols_finds_type_aliases() {
        let syms = symbols("pub type Alias = u32;\ntype Private = i8;\n");
        let alias = syms.iter().find(|s| s.name == "Alias").unwrap();
        assert_eq!(alias.kind, SymbolKind::Class);
        assert!(alias.exported);
        let private = syms.iter().find(|s| s.name == "Private").unwrap();
        assert_eq!(private.kind, SymbolKind::Class);
        assert!(!private.exported);
    }

    #[test]
    fn symbols_finds_unions() {
        let syms = symbols("pub union U { a: u32, b: f32 }\nunion V { x: u8 }\n");
        let public = syms.iter().find(|s| s.name == "U").unwrap();
        assert_eq!(public.kind, SymbolKind::Class);
        assert!(public.exported);
        let private = syms.iter().find(|s| s.name == "V").unwrap();
        assert_eq!(private.kind, SymbolKind::Class);
        assert!(!private.exported);
    }

    #[test]
    fn symbols_finds_macro_definitions() {
        let syms = symbols(
            "#[macro_export]\nmacro_rules! mymac { () => {}; }\nmacro_rules! privmac { () => {}; }\n",
        );
        let exported = syms.iter().find(|s| s.name == "mymac").unwrap();
        assert_eq!(exported.kind, SymbolKind::Function);
        assert!(exported.exported);
        let private = syms.iter().find(|s| s.name == "privmac").unwrap();
        assert_eq!(private.kind, SymbolKind::Function);
        assert!(!private.exported);
    }

    #[test]
    fn symbols_macro_export_seen_past_doc_comments_and_other_attrs() {
        let syms = symbols(
            "#[macro_export]\n// helper macro\n#[allow(unused)]\nmacro_rules! spaced { () => {}; }\n",
        );
        let mac = syms.iter().find(|s| s.name == "spaced").unwrap();
        assert!(mac.exported);
    }

    #[test]
    fn symbols_finds_required_trait_methods() {
        let syms = symbols("trait T { fn required(&self); fn defaulted(&self) {} }\n");
        let required = syms.iter().find(|s| s.name == "required").unwrap();
        assert_eq!(required.kind, SymbolKind::Method);
        // Default method bodies are function_items; captured once, no dupes.
        let defaulted: Vec<_> = syms.iter().filter(|s| s.name == "defaulted").collect();
        assert_eq!(defaulted.len(), 1);
    }

    #[test]
    fn symbols_span_covers_full_definition() {
        let src =
            "pub fn multi(a: i32) -> i32 {\n    let b = a + 1;\n    let c = b * 2;\n    c\n}\n";
        let syms = symbols(src);
        let f = syms.iter().find(|s| s.name == "multi").unwrap();
        assert_eq!(f.span.start_line, 1);
        assert_eq!(f.span.end_line, 5);
        assert!(f.exported);
    }

    #[test]
    fn symbols_method_span_covers_body() {
        let src = "struct Foo;\nimpl Foo {\n    fn m(&self) {\n        let x = 1;\n        let _ = x;\n    }\n}\n";
        let syms = symbols(src);
        let m = syms.iter().find(|s| s.name == "m").unwrap();
        assert_eq!(m.span.start_line, 3);
        assert_eq!(m.span.end_line, 6);
    }

    #[test]
    fn symbols_struct_span_covers_fields() {
        let src = "pub struct Wide {\n    a: i32,\n    b: i32,\n}\n";
        let syms = symbols(src);
        let s = syms.iter().find(|s| s.name == "Wide").unwrap();
        assert_eq!(s.span.start_line, 1);
        assert_eq!(s.span.end_line, 4);
    }

    // ── Call extraction tests ────────────────────────────────────────────

    fn calls(src: &str) -> Vec<CallRef> {
        RustParser.extract_calls(src.as_bytes(), Path::new("lib.rs"))
    }

    #[test]
    fn calls_simple() {
        let c = calls("fn main() { foo(); bar(1, 2); }\n");
        assert!(c.iter().any(|c| c.callee_raw == "foo"));
        assert!(c.iter().any(|c| c.callee_raw == "bar"));
    }

    #[test]
    fn calls_method() {
        let c = calls("fn main() { obj.process(); }\n");
        assert!(c.iter().any(|c| c.callee_raw.contains("process")));
    }

    #[test]
    fn calls_macro() {
        let c = calls("fn main() { println!(\"hello\"); vec![1,2,3]; }\n");
        assert!(c.iter().any(|c| c.callee_raw == "println"));
        assert!(c.iter().any(|c| c.callee_raw == "vec"));
    }

    #[test]
    fn calls_scoped() {
        let c = calls("fn main() { Foo::bar(); util::helper(); let s = String::from(\"x\"); }\n");
        let names: Vec<&str> = c.iter().map(|c| c.callee_raw.as_str()).collect();
        assert!(names.contains(&"Foo::bar"));
        assert!(names.contains(&"util::helper"));
        assert!(names.contains(&"String::from"));
    }

    #[test]
    fn calls_scoped_nested_path() {
        let c = calls("fn main() { crate::util::helper(); }\n");
        assert!(c.iter().any(|c| c.callee_raw == "crate::util::helper"));
    }

    #[test]
    fn calls_enum_variant_patterns() {
        let c = calls(
            r#"
fn handle(flow: Flow) {
    match flow {
        Flow::Ready(value) => drop(value),
        Flow::Waiting { id } => drop(id),
        Done(value) => drop(value),
    }
}
"#,
        );
        let names: Vec<&str> = c.iter().map(|c| c.callee_raw.as_str()).collect();
        assert!(names.contains(&"Flow::Ready"));
        assert!(names.contains(&"Flow::Waiting"));
        assert!(names.contains(&"Done"));
    }

    #[test]
    fn calls_scoped_macro() {
        let c = calls("fn main() { tracing::warn!(\"uh oh\"); }\n");
        assert!(c.iter().any(|c| c.callee_raw == "tracing::warn"));
    }

    #[test]
    fn calls_type_annotations() {
        let c = calls("fn foo(x: MyType) -> ReturnType { todo!() }\n");
        let names: Vec<&str> = c.iter().map(|c| c.callee_raw.as_str()).collect();
        assert!(names.contains(&"MyType"));
        assert!(names.contains(&"ReturnType"));
    }

    #[test]
    fn calls_trait_bounds() {
        let c = calls("impl<T> MyTrait for MyStruct where T: OtherTrait {}\n");
        let names: Vec<&str> = c.iter().map(|c| c.callee_raw.as_str()).collect();
        assert!(names.contains(&"MyTrait"));
        assert!(names.contains(&"MyStruct"));
        assert!(names.contains(&"OtherTrait"));
    }

    #[test]
    fn calls_attribute_and_derive_paths() {
        let c = calls(
            r#"
#[crate::macros::tracked]
#[derive(Clone, crate::macros::Track)]
struct Event;
"#,
        );
        let names: Vec<&str> = c.iter().map(|c| c.callee_raw.as_str()).collect();
        assert!(names.contains(&"crate::macros::tracked"));
        assert!(names.contains(&"Clone"));
        assert!(names.contains(&"Track"));
    }
}

use std::cell::RefCell;
use std::path::Path;
use std::sync::LazyLock;

use tree_sitter::{Language, Query};

use crate::types::{Import, Lang};

// C++ uses the same include patterns as C
const CPP_QUERY_SRC: &str = r#"
;; Local include: #include "file.h"
(preproc_include
  path: (string_literal) @import.local)

;; System include: #include <iostream>
(preproc_include
  path: (system_lib_string) @import.system)
"#;

static CPP_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language: Language = tree_sitter_cpp::LANGUAGE.into();
    Query::new(&language, CPP_QUERY_SRC).expect("Failed to compile C++ query")
});

thread_local! {
    static CPP_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_cpp::LANGUAGE.into()).unwrap();
        p
    });
}

pub struct CppParser;

impl super::Parser for CppParser {
    fn lang(&self) -> Lang {
        Lang::Cpp
    }

    fn parse(&self, source: &[u8], path: &Path) -> Vec<Import> {
        CPP_PARSER.with(|parser| super::c::parse_c_like(parser, source, path, &CPP_QUERY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;
    use crate::types::ImportKind;

    fn parse(src: &str) -> Vec<Import> {
        CppParser.parse(src.as_bytes(), Path::new("test.cpp"))
    }

    #[test]
    fn local_include() {
        let imports = parse(r#"#include "myclass.hpp""#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "myclass.hpp");
        assert_eq!(imports[0].kind, ImportKind::Local);
    }

    #[test]
    fn system_include() {
        let imports = parse(r#"#include <iostream>"#);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "iostream");
        assert_eq!(imports[0].kind, ImportKind::System);
    }

    #[test]
    fn mixed_cpp() {
        let imports = parse(
            r#"
#include <iostream>
#include <vector>
#include "myclass.hpp"
#include "utils/helper.h"
"#,
        );
        assert_eq!(imports.len(), 4);
    }
}

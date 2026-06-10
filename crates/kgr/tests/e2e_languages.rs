use std::path::Path;

use serde_json::Value;

enum ImportCoverage {
    ResolvedLocalEdge,
    ParsedImportOnly(&'static str),
}

struct LanguageCase {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    coverage: ImportCoverage,
}

fn kgr() -> assert_cmd::Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("kgr");
    strip_host_kgr_env(&mut cmd);
    cmd.env("KGR_NO_CACHE", "1");
    cmd
}

fn strip_host_kgr_env(cmd: &mut assert_cmd::Command) {
    for key in std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_string_lossy().starts_with("KGR_"))
    {
        cmd.env_remove(key);
    }
}

fn write_fixture(root: &Path, files: &[(&str, &str)]) {
    for (relative_path, source) in files {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, source).unwrap();
    }
}

fn graph_json(root: &Path) -> Value {
    let output = kgr()
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(root)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    serde_json::from_slice(&output).unwrap()
}

fn files(json: &Value) -> &[Value] {
    json["files"].as_array().unwrap()
}

fn local_edges(json: &Value) -> usize {
    json["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| edge["kind"].as_str() == Some("local"))
        .count()
}

fn parsed_imports(json: &Value) -> usize {
    files(json)
        .iter()
        .filter_map(|file| file["imports"].as_array())
        .map(Vec::len)
        .sum()
}

#[test]
fn graph_json_covers_every_advertised_language() {
    let cases = [
        LanguageCase {
            name: "python",
            files: &[
                ("main.py", "import helper\n\nprint(helper.VALUE)\n"),
                ("helper.py", "VALUE = 1\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "typescript",
            files: &[
                (
                    "main.ts",
                    "import { value } from './helper';\nconsole.log(value);\n",
                ),
                ("helper.ts", "export const value = 1;\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "javascript",
            files: &[
                (
                    "main.js",
                    "import { value } from './helper.js';\nconsole.log(value);\n",
                ),
                ("helper.js", "export const value = 1;\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "java",
            files: &[
                (
                    "com/example/App.java",
                    "package com.example;\nimport com.example.Helper;\nclass App { Helper h; }\n",
                ),
                (
                    "com/example/Helper.java",
                    "package com.example;\nclass Helper {}\n",
                ),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "c",
            files: &[
                ("main.c", "#include \"helper.h\"\nint main(void) { return helper(); }\n"),
                ("helper.h", "int helper(void);\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "cpp",
            files: &[
                (
                    "main.cpp",
                    "#include \"helper.hpp\"\nint main() { return helper(); }\n",
                ),
                ("helper.hpp", "int helper();\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "rust",
            files: &[
                ("main.rs", "mod helper;\nfn main() { helper::value(); }\n"),
                ("helper.rs", "pub fn value() {}\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "go",
            files: &[
                (
                    "main.go",
                    "package main\n\nimport \"./util\"\n\nfunc main() { util.Value() }\n",
                ),
                ("util/helper.go", "package util\n\nfunc Value() {}\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "zig",
            files: &[
                (
                    "main.zig",
                    "const helper = @import(\"helper.zig\");\npub fn main() void { helper.value(); }\n",
                ),
                ("helper.zig", "pub fn value() void {}\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "csharp",
            files: &[
                (
                    "App.cs",
                    "using Example.Helper;\nnamespace Example { class App { } }\n",
                ),
                (
                    "Helper.cs",
                    "namespace Example.Helper { class Utility { } }\n",
                ),
            ],
            coverage: ImportCoverage::ParsedImportOnly("C# resolver support is not implemented"),
        },
        LanguageCase {
            name: "objc",
            files: &[
                (
                    "App.m",
                    "#import \"Helper.m\"\nint main(void) { return helper(); }\n",
                ),
                ("Helper.m", "int helper(void) { return 0; }\n"),
            ],
            coverage: ImportCoverage::ParsedImportOnly(
                "Objective-C resolver support is not implemented",
            ),
        },
        LanguageCase {
            name: "swift",
            files: &[
                (
                    "App.swift",
                    "import Foundation\nfunc run() { print(Helper.value) }\n",
                ),
                ("Helper.swift", "struct Helper { static let value = 1 }\n"),
            ],
            coverage: ImportCoverage::ParsedImportOnly("Swift imports are module-only today"),
        },
        LanguageCase {
            name: "ruby",
            files: &[
                ("main.rb", "require_relative 'helper'\nputs Helper::VALUE\n"),
                ("helper.rb", "module Helper\n  VALUE = 1\nend\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "php",
            files: &[
                (
                    "main.php",
                    "<?php\nrequire_once './helper.php';\necho helper();\n",
                ),
                ("helper.php", "<?php\nfunction helper() { return 1; }\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "scala",
            files: &[
                (
                    "App.scala",
                    "package example\nimport example.Helper\nobject App { val value = Helper.value }\n",
                ),
                (
                    "Helper.scala",
                    "package example\nobject Helper { val value = 1 }\n",
                ),
            ],
            coverage: ImportCoverage::ParsedImportOnly("Scala resolver support is not implemented"),
        },
        LanguageCase {
            name: "lua",
            files: &[
                (
                    "main.lua",
                    "local helper = require(\"./helper\")\nprint(helper.value)\n",
                ),
                ("helper.lua", "return { value = 1 }\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
        LanguageCase {
            name: "elixir",
            files: &[
                (
                    "app.ex",
                    "defmodule MyApp.App do\n  alias MyApp.Helper\nend\n",
                ),
                ("helper.ex", "defmodule MyApp.Helper do\nend\n"),
            ],
            coverage: ImportCoverage::ParsedImportOnly(
                "Elixir resolver support is not implemented",
            ),
        },
        LanguageCase {
            name: "haskell",
            files: &[
                (
                    "Main.hs",
                    "module Main where\nimport Helper\nmain = pure ()\n",
                ),
                ("Helper.hs", "module Helper where\nvalue = 1\n"),
            ],
            coverage: ImportCoverage::ParsedImportOnly(
                "Haskell resolver support is not implemented",
            ),
        },
        LanguageCase {
            name: "bash",
            files: &[
                ("main.sh", "source ./helper.sh\necho \"$VALUE\"\n"),
                ("helper.sh", "VALUE=1\n"),
            ],
            coverage: ImportCoverage::ResolvedLocalEdge,
        },
    ];

    for case in cases {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), case.files);

        let json = graph_json(tmp.path());
        assert_eq!(
            files(&json).len(),
            case.files.len(),
            "{} should discover every temp fixture file",
            case.name
        );

        match case.coverage {
            ImportCoverage::ResolvedLocalEdge => assert!(
                local_edges(&json) > 0,
                "{} should resolve at least one local graph edge:\n{json:#}",
                case.name
            ),
            ImportCoverage::ParsedImportOnly(reason) => assert!(
                parsed_imports(&json) > 0,
                "{} should parse at least one import ({reason}):\n{json:#}",
                case.name
            ),
        }
    }
}

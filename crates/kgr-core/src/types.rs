use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Python,
    TypeScript,
    JavaScript,
    Java,
    C,
    Cpp,
    Rust,
    Go,
    Unknown,
}

impl std::fmt::Display for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lang::Python => write!(f, "python"),
            Lang::TypeScript => write!(f, "typescript"),
            Lang::JavaScript => write!(f, "javascript"),
            Lang::Java => write!(f, "java"),
            Lang::C => write!(f, "c"),
            Lang::Cpp => write!(f, "cpp"),
            Lang::Rust => write!(f, "rust"),
            Lang::Go => write!(f, "go"),
            Lang::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportKind {
    Local,
    External,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Span {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    pub raw: String,
    pub kind: ImportKind,
    pub resolved: Option<PathBuf>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub path: PathBuf,
    pub lang: Lang,
    pub imports: Vec<Import>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepEdge {
    pub from: PathBuf,
    pub to: PathBuf,
    pub kind: ImportKind,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DepGraph {
    pub root: PathBuf,
    pub files: Vec<FileNode>,
    pub edges: Vec<DepEdge>,
    pub cycles: Vec<Vec<PathBuf>>,
    pub roots: Vec<PathBuf>,
    pub orphans: Vec<PathBuf>,
    pub test_entries: Vec<PathBuf>,
}

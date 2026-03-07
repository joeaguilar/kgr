use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default)]
    pub no_external: bool,
    #[serde(default)]
    pub no_color: bool,
    #[serde(default)]
    pub no_progress: bool,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub entry: Option<PathBuf>,
}

fn default_format() -> String {
    "tree".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            languages: None,
            exclude: Vec::new(),
            format: default_format(),
            no_external: false,
            no_color: false,
            no_progress: false,
            depth: None,
            entry: None,
        }
    }
}

#[allow(dead_code)]
pub fn load_config(root: &Path) -> Config {
    let config_path = root.join(".kgr.toml");

    Figment::new()
        .merge(Serialized::defaults(Config::default()))
        .merge(Toml::file(&config_path))
        .merge(Env::prefixed("KGR_"))
        .extract()
        .unwrap_or_default()
}

pub fn init_config(root: &Path) -> std::io::Result<PathBuf> {
    let config_path = root.join(".kgr.toml");

    let mut detected = std::collections::HashSet::new();

    for e in ignore::Walk::new(root).flatten() {
        if let Some(ext) = e.path().extension().and_then(|e| e.to_str()) {
            match ext {
                "py" | "pyi" => { detected.insert("py"); }
                "ts" | "tsx" => { detected.insert("ts"); }
                "js" | "jsx" | "mjs" | "cjs" => { detected.insert("js"); }
                "java" => { detected.insert("java"); }
                "c" | "h" => { detected.insert("c"); }
                "cpp" | "cc" | "cxx" | "hpp" => { detected.insert("cpp"); }
                "rs" => { detected.insert("rs"); }
                "go" => { detected.insert("go"); }
                _ => {}
            }
        }
    }

    let mut langs: Vec<&str> = detected.into_iter().collect();
    langs.sort();
    let langs: Vec<String> = langs.iter().map(|l| format!("\"{}\"", l)).collect();

    let content = format!(
        r#"# .kgr.toml — project configuration for kgr

[project]
languages = [{}]
exclude = ["dist", "build", "node_modules", "__pycache__", ".venv"]

[check]
cycles = "error"
orphans = "warn"

[output]
format = "tree"
no_external = false
"#,
        langs.join(", ")
    );

    std::fs::write(&config_path, content)?;
    Ok(config_path)
}

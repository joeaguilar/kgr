mod agent_docs;
mod baseline;
mod cache;
mod config;
mod pipeline;
mod render;
mod rules;
mod walk;

#[cfg(test)]
pub(crate) mod test_env {
    use std::ffi::OsString;
    use std::sync::Mutex;

    pub(crate) static KGR_ENV_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) struct CleanKgrEnv {
        saved: Vec<(OsString, OsString)>,
    }

    impl CleanKgrEnv {
        pub(crate) fn new() -> Self {
            let saved = std::env::vars_os()
                .filter(|(key, _)| is_kgr_key(key))
                .collect();
            clear_current_kgr_env();
            Self { saved }
        }
    }

    impl Default for CleanKgrEnv {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Drop for CleanKgrEnv {
        fn drop(&mut self) {
            clear_current_kgr_env();
            for (key, value) in &self.saved {
                std::env::set_var(key, value);
            }
        }
    }

    fn clear_current_kgr_env() {
        let keys: Vec<_> = std::env::vars_os()
            .map(|(key, _)| key)
            .filter(is_kgr_key)
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }
    }

    fn is_kgr_key(key: &OsString) -> bool {
        key.to_string_lossy().starts_with("KGR_")
    }
}

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

use clap::{ArgGroup, Parser, Subcommand};

use kgr_core::graph::KGraph;
use kgr_core::parse::ParserRegistry;
use kgr_core::resolve::Resolver;

#[derive(Parser)]
#[command(
    name = "kgr",
    version = env!("KGR_VERSION"),
    about = "Polyglot source dependency knowledge graph"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze and display the full dependency graph
    Graph {
        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: tree, json, table, dot, mermaid [default: tree]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language: py, ts, js, rs, java, c, cpp, go, zig, cs, objc, swift, rb, php, scala, lua, ex, hs, sh
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Hide external dependencies
        #[arg(long)]
        no_external: bool,

        /// Show external package names in tree/table output
        #[arg(long)]
        show_external: bool,

        /// Exclude vendored paths (vendor/, third_party/, external/ by
        /// default; override with `vendor_globs` in .kgr.toml) from orphan
        /// analysis. Files stay in the graph; only the summary is filtered.
        #[arg(long)]
        first_party: bool,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Write output to file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Include symbol definitions in JSON output
        #[arg(long)]
        symbols: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Check for dependency issues (cycles, orphans, rule violations)
    Check {
        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: text, json [default: text]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Exclude vendored paths (vendor/, third_party/, external/ by
        /// default; override with `vendor_globs` in .kgr.toml) from orphan
        /// analysis. Files stay in the graph; only the summary is filtered.
        #[arg(long)]
        first_party: bool,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Record current violations as the new baseline (exits 0)
        #[arg(long)]
        update_baseline: bool,

        /// Path to baseline file [default: <root>/.kgr-baseline.json]
        #[arg(long)]
        baseline: Option<PathBuf>,

        /// Also report tree-sitter parse errors (ERROR/MISSING nodes)
        #[arg(long)]
        syntax: bool,

        /// Report-only mode: emit the same diagnostics but exit 0 even when
        /// cycles or error-severity rule violations are found. Operational
        /// failures (bad path, invalid format, broken config) still exit
        /// nonzero.
        #[arg(long)]
        exit_zero: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Query the dependency graph
    #[command(group(
        ArgGroup::new("query_selector")
            .required(true)
            .multiple(false)
            .args([
                "who_imports",
                "deps_of",
                "path_between",
                "cycles",
                "orphans",
                "heaviest",
                "largest_cycle",
            ])
    ))]
    Query {
        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Show files that import the given file
        #[arg(long)]
        who_imports: Option<PathBuf>,

        /// Show all transitive dependencies of a file
        #[arg(long)]
        deps_of: Option<PathBuf>,

        /// Show shortest path between two files
        #[arg(long, num_args = 2)]
        path_between: Option<Vec<PathBuf>>,

        /// List all cycles
        #[arg(long)]
        cycles: bool,

        /// List orphaned files
        #[arg(long)]
        orphans: bool,

        /// List files by number of dependents (descending)
        #[arg(long)]
        heaviest: bool,

        /// Show top N files for --heaviest (default: 20)
        #[arg(short, long, requires = "heaviest")]
        top: Option<usize>,

        /// List the largest cycle
        #[arg(long)]
        largest_cycle: bool,

        /// Exclude vendored paths (vendor/, third_party/, external/ by
        /// default; override with `vendor_globs` in .kgr.toml) from
        /// --heaviest and --orphans results
        #[arg(long)]
        first_party: bool,

        /// Output format [default: table]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// List all symbol definitions (functions, classes, methods)
    Symbols {
        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: table, json [default: table]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Find all references to a symbol (definitions + call sites)
    Refs {
        /// Symbol name to search for
        name: String,

        /// Scope the query to the definition in this file path
        #[arg(long, value_name = "PATH")]
        defined_in: Option<PathBuf>,

        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: table, json [default: table]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Check if a symbol is safe to remove (no references found = dead)
    Dead {
        /// Symbol name to check
        name: String,

        /// Scope the query to the definition in this file path
        #[arg(long, value_name = "PATH")]
        defined_in: Option<PathBuf>,

        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: table, json [default: table]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Emit a token-minimal skeleton of each file (signatures only, bodies replaced with ...)
    Skeleton {
        /// Root directory or file to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: text, json, table [default: text]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// One-shot codebase overview: file counts, languages, entry points, heaviest files
    Orient {
        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: text, json [default: text]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Show the transitive blast radius of a symbol change
    Impact {
        /// Symbol name to analyze
        name: String,

        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: text, json [default: text]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Maximum depth to traverse (default: unlimited)
        #[arg(short, long)]
        depth: Option<usize>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Rank files by complexity (function count, average length)
    Hotspots {
        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: text, json, table [default: table]
        #[arg(short, long)]
        format: Option<String>,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Show top N files (default: 20)
        #[arg(short, long)]
        top: Option<usize>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Print the definition body of a symbol, straight from source
    Show {
        /// Symbol name to print
        name: String,

        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Include n lines of context before/after the definition
        #[arg(short, long, default_value_t = 0)]
        context: usize,

        /// Print every match (default: first match + one-line pointers to others)
        #[arg(long)]
        all: bool,

        /// Disambiguate same-named symbols: fn, class, method
        #[arg(short, long)]
        kind: Option<String>,

        /// Output format: text, json [default: text]
        #[arg(short, long)]
        format: Option<String>,

        /// Raw body without line numbers (pipe-friendly)
        #[arg(long)]
        no_linenos: bool,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Print a numbered line window from a file (index-free)
    Slice {
        /// Target: <file>:<start>, <file>:<start>-<end>, or <file> with positional start/end
        target: String,

        /// Start line (positional fallback for `kgr slice file start end`)
        start: Option<usize>,

        /// End line (positional fallback)
        end: Option<usize>,

        /// Expand a single-line target both ways (default 10 when no end given)
        #[arg(short, long)]
        context: Option<usize>,

        /// Raw text without line numbers
        #[arg(long)]
        no_linenos: bool,

        /// Output format: text, json
        #[arg(short, long, default_value = "text")]
        format: String,

        /// Maximum lines to print
        #[arg(long, default_value_t = 500)]
        max: usize,
    },

    /// Generate a .kgr.toml configuration file
    Init {
        /// Directory to initialize
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Overwrite an existing .kgr.toml
        #[arg(long)]
        force: bool,
    },

    /// Rebuild kgr from source and replace the running binary
    Upgrade,

    /// Print a machine-readable guide for AI agents
    AgentInfo {
        /// Output format: text, json
        #[arg(short, long, default_value = "text")]
        format: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Graph {
            path,
            format,
            lang,
            no_external,
            show_external,
            first_party,
            no_progress,
            symbols,
            output,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_graph(
                &path,
                format.as_deref(),
                &lang,
                no_external,
                show_external,
                first_party,
                no_progress,
                symbols,
                output.as_deref(),
            );
        }
        Some(Commands::Check {
            path,
            format,
            lang,
            first_party,
            no_progress,
            update_baseline,
            baseline,
            syntax,
            exit_zero,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_check(
                &path,
                format.as_deref(),
                &lang,
                first_party,
                no_progress,
                update_baseline,
                baseline.as_deref(),
                syntax,
                exit_zero,
            );
        }
        Some(Commands::Query {
            path,
            who_imports,
            deps_of,
            path_between,
            cycles,
            orphans,
            heaviest,
            top,
            largest_cycle,
            first_party,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_query(
                &path,
                who_imports.as_deref(),
                deps_of.as_deref(),
                path_between.as_deref(),
                cycles,
                orphans,
                heaviest,
                top,
                largest_cycle,
                first_party,
                format.as_deref(),
                &lang,
                no_progress,
            );
        }
        Some(Commands::Symbols {
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_symbols(&path, format.as_deref(), &lang, no_progress);
        }
        Some(Commands::Refs {
            name,
            defined_in,
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_refs(
                &name,
                defined_in.as_deref(),
                &path,
                format.as_deref(),
                &lang,
                no_progress,
            );
        }
        Some(Commands::Dead {
            name,
            defined_in,
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_dead(
                &name,
                defined_in.as_deref(),
                &path,
                format.as_deref(),
                &lang,
                no_progress,
            );
        }
        Some(Commands::Skeleton {
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_skeleton(&path, format.as_deref(), &lang, no_progress);
        }
        Some(Commands::Orient {
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_orient(&path, format.as_deref(), &lang, no_progress);
        }
        Some(Commands::Impact {
            name,
            path,
            format,
            lang,
            depth,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_impact(&name, &path, format.as_deref(), &lang, depth, no_progress);
        }
        Some(Commands::Hotspots {
            path,
            format,
            lang,
            top,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_hotspots(&path, format.as_deref(), &lang, top, no_progress);
        }
        Some(Commands::Show {
            name,
            path,
            context,
            all,
            kind,
            format,
            no_linenos,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_show(
                &name,
                &path,
                context,
                all,
                kind.as_deref(),
                format.as_deref(),
                no_linenos,
                &lang,
                no_progress,
            );
        }
        Some(Commands::Slice {
            target,
            start,
            end,
            context,
            no_linenos,
            format,
            max,
        }) => {
            setup_tracing(0);
            run_slice(&target, start, end, context, no_linenos, &format, max);
        }
        Some(Commands::Init { path, force }) => {
            run_init(&path, force);
        }
        Some(Commands::Upgrade) => {
            run_upgrade();
        }
        Some(Commands::AgentInfo { format }) => {
            run_agent_info(&format);
        }
        None => {
            // Default: run graph on the current directory (format resolves
            // to config `format` if set, otherwise tree)
            setup_tracing(0);
            run_graph(
                &PathBuf::from("."),
                None,
                &None,
                false,
                false,
                false,
                false,
                false,
                None,
            );
        }
    }
}

fn setup_tracing(verbosity: u8) {
    let filter = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .init();
}

/// Canonicalize the user-supplied PATH and resolve the scan target.
///
/// When PATH is a directory, it becomes the scan root. When PATH is a file,
/// the scan root becomes its parent directory (so discovered paths stay
/// root-relative and import resolution works as usual) and the file itself
/// is returned for single-file discovery.
fn resolve_scan_target(path: &Path) -> (PathBuf, Option<PathBuf>) {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|e| {
        eprintln!("Error: cannot access '{}': {}", path.display(), e);
        process::exit(2);
    });
    if canon.is_file() {
        let root = canon
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"));
        (root, Some(canon))
    } else {
        (canon, None)
    }
}

/// Discover source files for the scan target. For a single explicitly-named
/// file, a clear error is printed and the process exits non-zero when the
/// file cannot be analyzed (unsupported language, --lang mismatch, too big).
///
/// Returns the analyzable files plus the skipped-unsupported summary from
/// the walk (always empty in single-file mode — the unsupported case exits
/// with an explicit error there).
fn discover_or_exit(
    root: &Path,
    single_file: Option<&Path>,
    lang: &Option<Vec<String>>,
    cfg: &config::Config,
) -> (Vec<walk::DiscoveredFile>, Vec<walk::SkippedGroup>) {
    match single_file {
        Some(file) => {
            match walk::discover_single_file(root, file, lang, cfg.max_file_size_bytes()) {
                Ok(f) => (vec![f], Vec::new()),
                Err(reason) => {
                    eprintln!("Error: cannot analyze '{}': {}", file.display(), reason);
                    process::exit(2);
                }
            }
        }
        None => {
            let discovery = walk::discover(root, lang, &cfg.exclude, cfg.max_file_size_bytes());
            (discovery.files, discovery.skipped_unsupported)
        }
    }
}

fn load_config_or_exit(root: &Path) -> config::Config {
    config::load_config(root).unwrap_or_else(|e| {
        eprintln!(
            "Error: failed to load config '{}': {}",
            root.join(".kgr.toml").display(),
            e
        );
        process::exit(2);
    })
}

/// Build the compiled first-party filter when filtering is enabled (CLI
/// `--first-party` OR config `first_party = true`); `None` keeps the
/// default, unfiltered behavior. Invalid vendor globs warn and are
/// skipped, mirroring exclude-glob handling in walk.rs.
fn build_first_party_filter(
    cli_first_party: bool,
    cfg: &config::Config,
) -> Option<config::FirstPartyFilter> {
    if !config::resolve_first_party(cli_first_party, cfg.first_party) {
        return None;
    }
    let (filter, warnings) =
        config::FirstPartyFilter::compile(config::effective_vendor_globs(&cfg.vendor_globs));
    for warning in warnings {
        eprintln!("{warning}");
    }
    Some(filter)
}

/// Drop vendored paths from an orphan list, returning how many were
/// excluded. First-party filtering composes with the test-entry and
/// structural-entry suppression already applied during graph construction
/// — it removes vendored paths from whatever orphans remain.
fn filter_vendored_orphans(orphans: &mut Vec<PathBuf>, filter: &config::FirstPartyFilter) -> usize {
    let before = orphans.len();
    orphans.retain(|path| !filter.is_vendored(path));
    before - orphans.len()
}

/// JSON object describing the applied first-party filtering. Attached to
/// outputs only when the filter is active, so default output shapes stay
/// byte-for-byte backwards compatible.
fn first_party_filter_json(
    filter: &config::FirstPartyFilter,
    excluded_key: &str,
    excluded: usize,
) -> serde_json::Value {
    serde_json::json!({
        "vendor_globs": filter.globs(),
        (excluded_key): excluded,
    })
}

/// Reject a resolved output format that the subcommand does not support.
///
/// Runs after `config::resolve_format`, so it covers bad values from the
/// CLI flag, the config `format` field, and the `KGR_FORMAT` env var alike.
/// Exits 2 with an error naming the valid formats, matching `kgr graph`'s
/// rejection behavior, instead of silently falling through to the default
/// text/table branch.
fn validate_format_or_exit(format: &str, valid: &[&str]) {
    if !valid.contains(&format) {
        eprintln!("Unknown format: {format} (expected: {})", valid.join(", "));
        process::exit(2);
    }
}

fn no_supported_files_message(root: &Path) -> String {
    format!(
        "No supported source files found in {}. Check the path, --lang filter, and exclude settings.",
        root.display()
    )
}

fn exit_no_supported_files(root: &Path, format: &str, skipped: &[walk::SkippedGroup]) -> ! {
    let message = no_supported_files_message(root);
    eprintln!("{message}");
    if format == "json" {
        let mut stdout = std::io::stdout().lock();
        let mut payload = serde_json::json!({
            "ok": false,
            "error": "no supported source files found",
            "root": root.to_string_lossy(),
            "hint": "Check the path, --lang filter, and exclude settings.",
        });
        if !skipped.is_empty() {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "skipped_unsupported".to_string(),
                    skipped_unsupported_json(skipped),
                );
            }
        }
        write_json_line(&mut stdout, &payload);
    }
    process::exit(2);
}

/// JSON value for the skipped-unsupported summary: an array of
/// `{group, count, sample}` objects, grouped by extension with a bounded
/// path sample. Attached to graph/check/orient JSON ONLY when at least one
/// unsupported file was skipped, so fully-supported repos keep their
/// existing output shapes byte-for-byte.
fn skipped_unsupported_json(skipped: &[walk::SkippedGroup]) -> serde_json::Value {
    let empty = serde_json::Value::Array(Vec::new());
    serde_json::to_value(skipped).unwrap_or(empty)
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch passes through all flags"
)]
fn run_graph(
    path: &Path,
    format: Option<&str>,
    lang: &Option<Vec<String>>,
    no_external: bool,
    show_external: bool,
    first_party: bool,
    no_progress: bool,
    include_symbols: bool,
    output: Option<&std::path::Path>,
) {
    let (root, single_file) = resolve_scan_target(path);

    let cfg = load_config_or_exit(&root);
    let format = config::resolve_format(format, cfg.format.as_deref(), "tree");
    validate_format_or_exit(format, &["tree", "json", "table", "dot", "mermaid"]);
    let lang = config::resolve_langs(lang, &cfg.languages);
    let no_progress = config::resolve_no_progress(no_progress, cfg.no_progress);
    let fp_filter = build_first_party_filter(first_party, &cfg);
    let registry = ParserRegistry::new();
    let (files, skipped_unsupported) = discover_or_exit(&root, single_file.as_deref(), &lang, &cfg);

    if files.is_empty() {
        exit_no_supported_files(&root, format, &skipped_unsupported);
    }

    tracing::info!("Discovered {} files", files.len());

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let mut file_nodes =
        pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    let resolver = Resolver::new(&root, &file_nodes);
    resolver.resolve_all(&mut file_nodes);

    // Keep a copy of file_nodes for --symbols enrichment
    let symbols_data: Option<Vec<_>> = if include_symbols {
        Some(
            file_nodes
                .iter()
                .map(|f| (f.path.clone(), f.symbols.clone()))
                .collect(),
        )
    } else {
        None
    };

    let kgraph = KGraph::from_files(&file_nodes);
    let mut dep_graph = kgraph.to_dep_graph(root, file_nodes);

    // First-party filtering: drop vendored paths from the orphan summary
    // (the graph itself — files and edges — is never filtered; that is
    // what config `exclude` is for).
    let excluded_orphans = fp_filter
        .as_ref()
        .map(|filter| filter_vendored_orphans(&mut dep_graph.orphans, filter))
        .unwrap_or(0);

    let mut writer: Box<dyn std::io::Write> = if let Some(out_path) = output {
        Box::new(std::fs::File::create(out_path).unwrap_or_else(|e| {
            eprintln!("Error: cannot create output file: {}", e);
            process::exit(2);
        }))
    } else {
        Box::new(std::io::stdout().lock())
    };

    // JSON output needs post-processing when --symbols injects per-file
    // symbols, an active first-party filter must be made explicit, and/or
    // the walk skipped unsupported files that must be reported.
    if format == "json"
        && (include_symbols || fp_filter.is_some() || !skipped_unsupported.is_empty())
    {
        let mut json = render::json::graph_value(&dep_graph).unwrap_or_else(|e| {
            eprintln!("Error rendering output: {}", e);
            process::exit(2);
        });
        if include_symbols {
            let Some(data) = symbols_data else { return };
            let symbols_map: std::collections::HashMap<_, _> = data.into_iter().collect();
            if let Some(files) = json.get_mut("files").and_then(|f| f.as_array_mut()) {
                for file in files {
                    let path_str = file["path"].as_str().unwrap_or_default();
                    let path = std::path::PathBuf::from(path_str);
                    if let Some(syms) = symbols_map.get(&path) {
                        file["symbols"] = serde_json::json!(syms
                            .iter()
                            .map(|s| serde_json::json!({
                                "name": s.name,
                                "kind": s.kind.to_string(),
                                "line": s.span.start_line,
                                "end_line": s.span.end_line,
                                "exported": s.exported,
                            }))
                            .collect::<Vec<_>>());
                    } else {
                        file["symbols"] = serde_json::json!([]);
                    }
                }
            }
        }
        if let Some(filter) = &fp_filter {
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "first_party_filter".to_string(),
                    first_party_filter_json(filter, "excluded_orphans", excluded_orphans),
                );
            }
        }
        if !skipped_unsupported.is_empty() {
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "skipped_unsupported".to_string(),
                    skipped_unsupported_json(&skipped_unsupported),
                );
            }
        }
        serde_json::to_writer_pretty(&mut writer, &json).unwrap_or_else(|e| {
            eprintln!("Error rendering output: {}", e);
            process::exit(2);
        });
        writeln!(writer).unwrap_or_else(|e| {
            eprintln!("Error rendering output: {}", e);
            process::exit(2);
        });
        return;
    }

    render::render(
        &dep_graph,
        &kgraph,
        format,
        no_external,
        show_external,
        &mut writer,
    )
    .unwrap_or_else(|e| {
        eprintln!("Error rendering output: {}", e);
        process::exit(2);
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch passes through all flags"
)]
fn run_check(
    path: &Path,
    format: Option<&str>,
    lang: &Option<Vec<String>>,
    first_party: bool,
    no_progress: bool,
    update_baseline: bool,
    baseline_path: Option<&Path>,
    syntax: bool,
    exit_zero: bool,
) {
    let (root, single_file) = resolve_scan_target(path);

    let cfg = load_config_or_exit(&root);
    let format = config::resolve_format(format, cfg.format.as_deref(), "text");
    validate_format_or_exit(format, &["text", "json"]);
    let lang = config::resolve_langs(lang, &cfg.languages);
    let no_progress = config::resolve_no_progress(no_progress, cfg.no_progress);
    let fp_filter = build_first_party_filter(first_party, &cfg);
    let registry = ParserRegistry::new();
    let (files, skipped_unsupported) = discover_or_exit(&root, single_file.as_deref(), &lang, &cfg);

    if files.is_empty() {
        exit_no_supported_files(&root, format, &skipped_unsupported);
    }

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let mut file_nodes =
        pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    let resolver = Resolver::new(&root, &file_nodes);
    resolver.resolve_all(&mut file_nodes);

    let kgraph = KGraph::from_files(&file_nodes);
    let mut dep_graph = kgraph.to_dep_graph(root.clone(), file_nodes);

    // First-party filtering applies to the orphan summary only; cycles and
    // rule violations are always reported in full.
    let excluded_orphans = fp_filter
        .as_ref()
        .map(|filter| filter_vendored_orphans(&mut dep_graph.orphans, filter))
        .unwrap_or(0);

    let all_rule_violations = match rules::check_rules(&dep_graph, &cfg.rules) {
        Ok(violations) => violations,
        Err(errors) => {
            for error in errors {
                eprintln!(
                    "warning[kgr::rule-config]: rule '{}' has invalid {} glob '{}': {}",
                    error.rule_name, error.field, error.pattern, error.message
                );
            }
            process::exit(1);
        }
    };
    let resolved_baseline_path = baseline_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".kgr-baseline.json"));

    // --update-baseline: record current state and exit 0
    if update_baseline {
        baseline::Baseline::load(&resolved_baseline_path).unwrap_or_else(|e| {
            eprintln!(
                "Error: failed to load baseline '{}': {}",
                resolved_baseline_path.display(),
                e
            );
            process::exit(2);
        });
        let bl = baseline::Baseline::new(&dep_graph.cycles, &all_rule_violations);
        bl.save(&resolved_baseline_path).unwrap_or_else(|e| {
            eprintln!("Error writing baseline: {}", e);
            process::exit(2);
        });
        eprintln!(
            "Baseline updated: {} cycle(s), {} rule violation(s) recorded in {}",
            bl.cycles.len(),
            bl.rule_violations.len(),
            resolved_baseline_path.display()
        );
        return;
    }

    // Load baseline if it exists
    let bl = baseline::Baseline::load(&resolved_baseline_path).unwrap_or_else(|e| {
        eprintln!(
            "Error: failed to load baseline '{}': {}",
            resolved_baseline_path.display(),
            e
        );
        process::exit(2);
    });
    let suppressed = bl.as_ref().map(|b| b.total()).unwrap_or(0);

    let active_cycles: Vec<&Vec<std::path::PathBuf>> = match &bl {
        Some(b) => b.new_cycles(&dep_graph.cycles),
        None => dep_graph.cycles.iter().collect(),
    };
    let active_rule_violations: Vec<&rules::RuleViolation> = match &bl {
        Some(b) => b.new_rule_violations(&all_rule_violations),
        None => all_rule_violations.iter().collect(),
    };

    let has_errors = !active_cycles.is_empty()
        || active_rule_violations
            .iter()
            .any(|v| matches!(v.severity, config::Severity::Error));

    // Collect syntax errors when --syntax is enabled
    let syntax_errors: Vec<(std::path::PathBuf, Vec<kgr_core::types::ParseError>)> = if syntax {
        let mut errs = Vec::new();
        for f in &dep_graph.files {
            let full_path = root.join(&f.path);
            let source = match std::fs::read(&full_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if let Some(parser) = registry.get(f.lang) {
                let file_errors = parser.parse_errors(&source, &f.path);
                if !file_errors.is_empty() {
                    errs.push((f.path.clone(), file_errors));
                }
            }
        }
        errs
    } else {
        Vec::new()
    };

    if format == "json" {
        let mut json = serde_json::json!({
            "ok": !has_errors,
            "cycles": active_cycles.iter().map(|cycle| {
                cycle.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>()
            }).collect::<Vec<_>>(),
            "orphans": dep_graph.orphans.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "rule_violations": active_rule_violations.iter().map(|v| serde_json::json!({
                "rule": v.rule_name,
                "from": v.from.to_string_lossy(),
                "to": v.to.to_string_lossy(),
                "severity": match v.severity {
                    config::Severity::Error => "error",
                    config::Severity::Warn => "warn",
                },
            })).collect::<Vec<_>>(),
            "suppressed": suppressed,
        });
        if let Some(filter) = &fp_filter {
            json.as_object_mut().unwrap().insert(
                "first_party_filter".to_string(),
                first_party_filter_json(filter, "excluded_orphans", excluded_orphans),
            );
        }
        if syntax {
            json.as_object_mut().unwrap().insert(
                "syntax_errors".to_string(),
                serde_json::json!(syntax_errors
                    .iter()
                    .flat_map(|(path, errors)| {
                        errors.iter().map(move |e| {
                            serde_json::json!({
                                "file": path.to_string_lossy(),
                                "message": e.message,
                                "line": e.span.start_line,
                                "column": e.span.start_col,
                            })
                        })
                    })
                    .collect::<Vec<_>>()),
            );
        }
        // Reported only when non-empty so existing JSON consumers and
        // snapshots of fully-supported repos are unaffected.
        if !skipped_unsupported.is_empty() {
            json.as_object_mut().unwrap().insert(
                "skipped_unsupported".to_string(),
                skipped_unsupported_json(&skipped_unsupported),
            );
        }
        println!("{}", serde_json::to_string_pretty(&json).unwrap());
    } else {
        // Report cycles
        if !active_cycles.is_empty() {
            eprintln!("error[kgr::cycle]: Circular dependency detected");
            for cycle in &active_cycles {
                eprint!("  ");
                for (i, p) in cycle.iter().enumerate() {
                    if i > 0 {
                        eprint!(" -> ");
                    }
                    eprint!("{}", p.display());
                }
                eprintln!(" -> {} (cycle)", cycle[0].display());
            }
            eprintln!();
        }

        // Report orphans (always warn, never baselined)
        if !dep_graph.orphans.is_empty() {
            eprintln!("warning[kgr::orphan]: Orphaned files (no imports, not imported):");
            for orphan in &dep_graph.orphans {
                eprintln!("  {}", orphan.display());
            }
            eprintln!();
        }

        // Report rule violations
        for v in &active_rule_violations {
            match v.severity {
                config::Severity::Error => {
                    eprintln!(
                        "error[kgr::rule]: rule '{}' violated: {} -> {}",
                        v.rule_name,
                        v.from.display(),
                        v.to.display()
                    );
                }
                config::Severity::Warn => {
                    eprintln!(
                        "warning[kgr::rule]: rule '{}' violated: {} -> {}",
                        v.rule_name,
                        v.from.display(),
                        v.to.display()
                    );
                }
            }
        }
        if !active_rule_violations.is_empty() {
            eprintln!();
        }

        // Report syntax errors as warnings
        if syntax {
            for (path, errors) in &syntax_errors {
                for err in errors {
                    eprintln!(
                        "warning[kgr::syntax]: {} at {}:{}:{}",
                        err.message,
                        path.display(),
                        err.span.start_line,
                        err.span.start_col
                    );
                }
            }
            let total: usize = syntax_errors.iter().map(|(_, e)| e.len()).sum();
            if total > 0 {
                eprintln!("{} syntax error(s) found", total);
                eprintln!();
            }
        }

        if suppressed > 0 {
            eprintln!("note: {} violation(s) suppressed by baseline", suppressed);
        }

        if fp_filter.is_some() {
            eprintln!(
                "note: first-party filter excluded {} vendored file(s) from orphan analysis",
                excluded_orphans
            );
        }

        if has_errors {
            // error messages already printed above
        } else {
            eprintln!("All checks passed.");
        }
    }

    // --exit-zero is report-only for FINDINGS: identical diagnostics and JSON
    // (including "ok": false), but the process exits 0 so the command can be
    // batched in shell pipelines without `|| true`. Operational failures
    // (unreadable target, invalid format, broken config/baseline) bypass this
    // — they exit nonzero above before reaching here.
    if has_errors && !exit_zero {
        process::exit(1);
    }
}

fn write_json_line<T: serde::Serialize + ?Sized>(stdout: &mut impl Write, value: &T) {
    serde_json::to_writer_pretty(&mut *stdout, value).ok();
    writeln!(stdout).ok();
}

fn normalize_query_target(
    root: &Path,
    target: &Path,
    files: &[kgr_core::types::FileNode],
) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if target.is_absolute() {
        if let Ok(rel) = target.strip_prefix(root) {
            push_query_target_candidate(&mut candidates, rel);
        }
    } else {
        push_query_target_candidate(&mut candidates, target);
    }

    let from_root = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    };
    if let Ok(canon) = std::fs::canonicalize(&from_root) {
        if let Ok(rel) = canon.strip_prefix(root) {
            push_query_target_candidate(&mut candidates, rel);
        }
    }

    if let Ok(canon) = std::fs::canonicalize(target) {
        if let Ok(rel) = canon.strip_prefix(root) {
            push_query_target_candidate(&mut candidates, rel);
        }
    }

    candidates.into_iter().find(|candidate| {
        files
            .iter()
            .any(|file| file.path.as_path() == candidate.as_path())
    })
}

fn push_query_target_candidate(candidates: &mut Vec<PathBuf>, candidate: &Path) {
    let candidate = normalize_query_target_path(candidate);
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn normalize_query_target_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir => normalized.push(".."),
            std::path::Component::Prefix(prefix) => {
                normalized.push(Path::new(prefix.as_os_str()));
            }
            std::path::Component::RootDir => {}
        }
    }
    normalized
}

fn exit_unknown_query_target(
    format: &str,
    selector: &str,
    target: &Path,
    root: &Path,
    stdout: &mut impl Write,
) -> ! {
    eprintln!(
        "Error: unknown query target for --{}: {} (not found in scanned files under {})",
        selector,
        target.display(),
        root.display()
    );
    if format == "json" {
        let payload = serde_json::json!({
            "found": false,
            "selector": selector,
            "target": target.to_string_lossy(),
            "root": root.to_string_lossy(),
            "error": "unknown query target",
        });
        write_json_line(stdout, &payload);
    }
    process::exit(2);
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch passes through all flags"
)]
fn run_query(
    path: &Path,
    who_imports: Option<&Path>,
    deps_of: Option<&Path>,
    path_between: Option<&[PathBuf]>,
    cycles: bool,
    orphans: bool,
    heaviest: bool,
    top: Option<usize>,
    largest_cycle: bool,
    first_party: bool,
    format: Option<&str>,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) {
    let (root, single_file) = resolve_scan_target(path);

    let cfg = load_config_or_exit(&root);
    let format = config::resolve_format(format, cfg.format.as_deref(), "table");
    validate_format_or_exit(format, &["table", "json"]);
    let lang = config::resolve_langs(lang, &cfg.languages);
    let no_progress = config::resolve_no_progress(no_progress, cfg.no_progress);
    let fp_filter = build_first_party_filter(first_party, &cfg);
    let registry = ParserRegistry::new();
    let (files, skipped_unsupported) = discover_or_exit(&root, single_file.as_deref(), &lang, &cfg);

    if files.is_empty() {
        exit_no_supported_files(&root, format, &skipped_unsupported);
    }

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let mut file_nodes =
        pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    let resolver = Resolver::new(&root, &file_nodes);
    resolver.resolve_all(&mut file_nodes);

    let kgraph = KGraph::from_files(&file_nodes);
    let dep_graph = kgraph.to_dep_graph(root.clone(), file_nodes);

    let mut stdout = std::io::stdout().lock();

    if let Some(target) = who_imports {
        let target = normalize_query_target(&root, target, &dep_graph.files).unwrap_or_else(|| {
            exit_unknown_query_target(format, "who-imports", target, &root, &mut stdout)
        });
        let mut dependents: Vec<PathBuf> = dep_graph
            .edges
            .iter()
            .filter(|edge| edge.to == target)
            .map(|edge| edge.from.clone())
            .collect();
        dependents.sort();
        if format == "json" {
            write_json_line(&mut stdout, &dependents);
        } else if dependents.is_empty() {
            eprintln!("No files import {}", target.display());
        } else {
            writeln!(stdout, "Files that import {}:", target.display()).ok();
            for dep in &dependents {
                writeln!(stdout, "  {}", dep.display()).ok();
            }
        }
    } else if let Some(target) = deps_of {
        let target = normalize_query_target(&root, target, &dep_graph.files).unwrap_or_else(|| {
            exit_unknown_query_target(format, "deps-of", target, &root, &mut stdout)
        });
        let deps = kgraph.transitive_deps(&target, None);
        if format == "json" {
            write_json_line(&mut stdout, &deps);
        } else if deps.is_empty() {
            eprintln!("{} has no dependencies", target.display());
        } else {
            writeln!(stdout, "Dependencies of {}:", target.display()).ok();
            for dep in &deps {
                writeln!(stdout, "  {}", dep.display()).ok();
            }
        }
    } else if let Some(endpoints) = path_between {
        if endpoints.len() == 2 {
            let from = normalize_query_target(&root, &endpoints[0], &dep_graph.files)
                .unwrap_or_else(|| {
                    exit_unknown_query_target(
                        format,
                        "path-between",
                        &endpoints[0],
                        &root,
                        &mut stdout,
                    )
                });
            let to = normalize_query_target(&root, &endpoints[1], &dep_graph.files).unwrap_or_else(
                || {
                    exit_unknown_query_target(
                        format,
                        "path-between",
                        &endpoints[1],
                        &root,
                        &mut stdout,
                    )
                },
            );
            let query_path = kgraph.shortest_path(&from, &to);
            if format == "json" {
                write_json_line(&mut stdout, &query_path);
            } else if let Some(path) = query_path {
                writeln!(
                    stdout,
                    "Shortest path from {} to {}:",
                    from.display(),
                    to.display()
                )
                .ok();
                for (i, node) in path.iter().enumerate() {
                    if i > 0 {
                        write!(stdout, " -> ").ok();
                    }
                    write!(stdout, "{}", node.display()).ok();
                }
                writeln!(stdout).ok();
            } else {
                eprintln!("No path found from {} to {}", from.display(), to.display());
            }
        }
    } else if cycles {
        if format == "json" {
            write_json_line(&mut stdout, &dep_graph.cycles);
        } else if dep_graph.cycles.is_empty() {
            eprintln!("No cycles found");
        } else {
            writeln!(stdout, "Cycles found: {}", dep_graph.cycles.len()).ok();
            for (i, cycle) in dep_graph.cycles.iter().enumerate() {
                write!(stdout, "  {}: ", i + 1).ok();
                for (j, node) in cycle.iter().enumerate() {
                    if j > 0 {
                        write!(stdout, " -> ").ok();
                    }
                    write!(stdout, "{}", node.display()).ok();
                }
                writeln!(stdout).ok();
            }
        }
    } else if orphans {
        let mut orphan_list = dep_graph.orphans.clone();
        let excluded = fp_filter
            .as_ref()
            .map(|filter| filter_vendored_orphans(&mut orphan_list, filter))
            .unwrap_or(0);
        if format == "json" {
            // With the filter active the bare array becomes an object so
            // the applied filtering is explicit; the default (unfiltered)
            // shape is unchanged.
            if let Some(filter) = &fp_filter {
                let payload = serde_json::json!({
                    "orphans": orphan_list,
                    "first_party_filter":
                        first_party_filter_json(filter, "excluded_orphans", excluded),
                });
                write_json_line(&mut stdout, &payload);
            } else {
                write_json_line(&mut stdout, &orphan_list);
            }
        } else {
            if fp_filter.is_some() {
                eprintln!(
                    "note: first-party filter excluded {} vendored file(s)",
                    excluded
                );
            }
            if orphan_list.is_empty() {
                eprintln!("No orphaned files found");
            } else {
                writeln!(stdout, "Orphaned files:").ok();
                for orphan in &orphan_list {
                    writeln!(stdout, "  {}", orphan.display()).ok();
                }
            }
        }
    } else if heaviest {
        let mut ranked = kgraph.heaviest();
        let excluded = fp_filter
            .as_ref()
            .map(|filter| {
                let before = ranked.len();
                ranked.retain(|(path, _)| !filter.is_vendored(path));
                before - ranked.len()
            })
            .unwrap_or(0);
        let limit = top.unwrap_or(20);
        if format == "json" {
            let items: Vec<serde_json::Value> = ranked
                .iter()
                .take(limit)
                .map(|(p, c)| {
                    serde_json::json!({
                        "path": p,
                        "dependents": c
                    })
                })
                .collect();
            // Same shape rule as --orphans: object only when filtering.
            if let Some(filter) = &fp_filter {
                let payload = serde_json::json!({
                    "heaviest": items,
                    "first_party_filter":
                        first_party_filter_json(filter, "excluded_files", excluded),
                });
                write_json_line(&mut stdout, &payload);
            } else {
                write_json_line(&mut stdout, &items);
            }
        } else {
            if fp_filter.is_some() {
                eprintln!(
                    "note: first-party filter excluded {} vendored file(s)",
                    excluded
                );
            }
            writeln!(stdout, "{:<50} {:>10}", "FILE", "DEPENDENTS").ok();
            writeln!(stdout, "{}", "-".repeat(62)).ok();
            for (path, count) in ranked.iter().take(limit) {
                writeln!(stdout, "{:<50} {:>10}", path.display(), count).ok();
            }
        }
    } else if largest_cycle {
        let cycle = dep_graph.cycles.iter().max_by_key(|c| c.len());
        if format == "json" {
            write_json_line(&mut stdout, &cycle);
        } else if let Some(cycle) = cycle {
            writeln!(stdout, "Largest cycle ({} files):", cycle.len()).ok();
            for node in cycle {
                writeln!(stdout, "  {}", node.display()).ok();
            }
        } else {
            eprintln!("No cycles found");
        }
    } else {
        eprintln!("Please specify a query flag. Run `kgr query --help` for options.");
        process::exit(2);
    }
}

fn run_init(path: &Path, force: bool) {
    match config::init_config(path, force) {
        Ok(config_path) => {
            println!("Created {}", config_path.display());
        }
        Err(e) => {
            eprintln!("Error creating config: {}", e);
            process::exit(2);
        }
    }
}

fn run_upgrade() {
    // The source directory is baked in at compile time by build.rs.
    // It points to the crates/kgr manifest dir, so the workspace root is two levels up.
    let source_dir = std::path::Path::new(env!("KGR_SOURCE_DIR"));
    let workspace_root = source_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(source_dir);

    // Where the running binary lives — this is where we'll overwrite.
    let dest = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("Error: cannot determine current executable path: {}", e);
        process::exit(2);
    });

    eprintln!("Upgrading kgr at {}", dest.display());
    eprintln!("Source: {}", workspace_root.display());

    // Resolve current branch name so pull works even without upstream tracking.
    let branch = std::process::Command::new("git")
        .args([
            "-C",
            &workspace_root.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to run git rev-parse: {}", e);
            process::exit(2);
        });
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();

    // git pull origin <branch>
    eprintln!("Running: git pull origin {}", branch);
    let status = std::process::Command::new("git")
        .args([
            "-C",
            &workspace_root.to_string_lossy(),
            "pull",
            "origin",
            &branch,
        ])
        .status()
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to run git pull: {}", e);
            process::exit(2);
        });
    if !status.success() {
        eprintln!("Error: git pull failed");
        process::exit(1);
    }

    // cargo build --release -p kgr
    eprintln!("Running: cargo build --release -p kgr");
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "-p", "kgr"])
        .current_dir(workspace_root)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to run cargo build: {}", e);
            process::exit(2);
        });
    if !status.success() {
        eprintln!("Error: cargo build failed");
        process::exit(1);
    }

    let new_bin = workspace_root.join("target/release/kgr");
    match replace_executable_atomically(&new_bin, &dest).unwrap_or_else(|e| {
        eprintln!(
            "Error: failed to replace {} with {}: {}",
            dest.display(),
            new_bin.display(),
            e
        );
        process::exit(2);
    }) {
        UpgradeReplacement::Replaced => {
            eprintln!("Installed {}", dest.display());
        }
        UpgradeReplacement::SkippedSameExecutable => {
            eprintln!("Built binary is already the running executable; skipping replacement.");
        }
    }

    eprintln!("kgr upgraded successfully.");
    eprintln!("Version: {}", env!("KGR_VERSION"));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpgradeReplacement {
    Replaced,
    SkippedSameExecutable,
}

fn replace_executable_atomically(
    new_bin: &Path,
    dest: &Path,
) -> std::io::Result<UpgradeReplacement> {
    if paths_refer_to_same_file(new_bin, dest) {
        return Ok(UpgradeReplacement::SkippedSameExecutable);
    }

    let dest_dir = dest.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("destination has no parent directory: {}", dest.display()),
        )
    })?;
    let dest_name = dest
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("kgr"))
        .to_string_lossy();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = dest_dir.join(format!(
        ".{dest_name}.upgrade-{}-{unique}.tmp",
        std::process::id()
    ));

    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    if let Err(e) = copy_executable_to_temp(new_bin, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    Ok(UpgradeReplacement::Replaced)
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn copy_executable_to_temp(src: &Path, tmp: &Path) -> std::io::Result<()> {
    std::fs::copy(src, tmp)?;
    let permissions = std::fs::metadata(src)?.permissions();
    std::fs::set_permissions(tmp, permissions)
}

/// Discover and parse all files for the scan target. Config-level defaults
/// (`format`, `languages`, `no_progress`) are resolved here so zero-file
/// errors can still respect JSON output.
fn build_file_nodes(
    path: &Path,
    format: Option<&str>,
    default_format: &str,
    valid_formats: &[&str],
    lang: &Option<Vec<String>>,
    no_progress: bool,
) -> (
    PathBuf,
    Vec<kgr_core::types::FileNode>,
    String,
    Vec<walk::SkippedGroup>,
) {
    let (root, single_file) = resolve_scan_target(path);

    let cfg = load_config_or_exit(&root);
    let format = config::resolve_format(format, cfg.format.as_deref(), default_format).to_string();
    validate_format_or_exit(&format, valid_formats);
    let lang = config::resolve_langs(lang, &cfg.languages);
    let no_progress = config::resolve_no_progress(no_progress, cfg.no_progress);
    let registry = ParserRegistry::new();
    let (files, skipped_unsupported) = discover_or_exit(&root, single_file.as_deref(), &lang, &cfg);

    if files.is_empty() {
        exit_no_supported_files(&root, &format, &skipped_unsupported);
    }

    tracing::info!("Discovered {} files", files.len());

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let file_nodes = pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    (root, file_nodes, format, skipped_unsupported)
}

fn run_symbols(path: &Path, format: Option<&str>, lang: &Option<Vec<String>>, no_progress: bool) {
    let (root, file_nodes, format, _skipped_unsupported) =
        build_file_nodes(path, format, "table", &["table", "json"], lang, no_progress);
    if file_nodes.is_empty() {
        return;
    }

    let mut stdout = std::io::stdout().lock();

    if format == "json" {
        let entries: Vec<serde_json::Value> = file_nodes
            .iter()
            .filter(|f| !f.symbols.is_empty())
            .map(|f| {
                serde_json::json!({
                    "file": f.path.to_string_lossy(),
                    "symbols": f.symbols.iter().map(|s| {
                        serde_json::json!({
                            "name": s.name,
                            "kind": s.kind.to_string(),
                            "line": s.span.start_line,
                            "end_line": s.span.end_line,
                            "exported": s.exported,
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        serde_json::to_writer_pretty(&mut stdout, &entries).ok();
        writeln!(stdout).ok();
    } else {
        writeln!(
            stdout,
            "{:<50} {:<20} {:<10} {:>5}  EXPORTED",
            "FILE", "SYMBOL", "KIND", "LINE"
        )
        .ok();
        writeln!(stdout, "{}", "-".repeat(95)).ok();
        for f in &file_nodes {
            let rel = f.path.strip_prefix(&root).unwrap_or(&f.path);
            for s in &f.symbols {
                writeln!(
                    stdout,
                    "{:<50} {:<20} {:<10} {:>5}  {}",
                    rel.display(),
                    s.name,
                    s.kind,
                    s.span.start_line,
                    if s.exported { "yes" } else { "no" }
                )
                .ok();
            }
        }
    }
}

/// True when a recorded callee refers to `name`, treating both `.` and `::`
/// as qualifier separators: matches `name`, `obj.name`, and `path::name`
/// (e.g. `util::helper`, `Foo::bar`, `tracing::warn`).
fn callee_matches(callee_raw: &str, name: &str) -> bool {
    callee_raw == name
        || callee_raw.ends_with(&format!(".{name}"))
        || callee_raw.ends_with(&format!("::{name}"))
}

#[derive(Clone, Copy)]
struct SymbolDefinitionMatch<'a> {
    file: &'a kgr_core::types::FileNode,
    symbol: &'a kgr_core::types::Symbol,
}

#[derive(Clone)]
struct SymbolReference {
    file: PathBuf,
    value: serde_json::Value,
}

fn resolve_defined_in_scope(
    format: &str,
    root: &Path,
    defined_in: Option<&Path>,
    file_nodes: &[kgr_core::types::FileNode],
    stdout: &mut impl Write,
) -> Option<PathBuf> {
    defined_in.map(|scope| {
        normalize_query_target(root, scope, file_nodes)
            .unwrap_or_else(|| exit_unknown_query_target(format, "defined-in", scope, root, stdout))
    })
}

fn find_symbol_definitions<'a>(
    file_nodes: &'a [kgr_core::types::FileNode],
    name: &str,
    defined_in: Option<&Path>,
) -> Vec<SymbolDefinitionMatch<'a>> {
    file_nodes
        .iter()
        .flat_map(|file| {
            file.symbols.iter().filter_map(move |symbol| {
                let matches_scope = match defined_in {
                    Some(scope) => file.path == scope,
                    None => true,
                };
                (symbol.name == name && matches_scope)
                    .then_some(SymbolDefinitionMatch { file, symbol })
            })
        })
        .collect()
}

fn symbol_definition_json(definition: SymbolDefinitionMatch<'_>) -> serde_json::Value {
    serde_json::json!({
        "id": definition.symbol.definition_id(&definition.file.path),
        "symbol": &definition.symbol.name,
        "file": definition.file.path.to_string_lossy(),
        "line": definition.symbol.span.start_line,
        "end_line": definition.symbol.span.end_line,
        "kind": definition.symbol.kind.to_string(),
        "exported": definition.symbol.exported,
    })
}

fn symbol_definition_jsons(definitions: &[SymbolDefinitionMatch<'_>]) -> Vec<serde_json::Value> {
    definitions
        .iter()
        .copied()
        .map(symbol_definition_json)
        .collect()
}

fn symbol_definition_ids(definitions: &[SymbolDefinitionMatch<'_>]) -> Vec<String> {
    definitions
        .iter()
        .map(|definition| definition.symbol.definition_id(&definition.file.path))
        .collect()
}

fn scope_hint() -> &'static str {
    "Use --defined-in <path> to select one definition."
}

fn write_ambiguous_symbol_text(
    stdout: &mut impl Write,
    name: &str,
    definitions: &[serde_json::Value],
) {
    writeln!(
        stdout,
        "Ambiguous symbol '{name}' matches {} definitions. {}",
        definitions.len(),
        scope_hint()
    )
    .ok();
    for definition in definitions {
        writeln!(
            stdout,
            "  {}:{} ({}) id={}",
            definition["file"].as_str().unwrap_or_default(),
            definition["line"],
            definition["kind"].as_str().unwrap_or_default(),
            definition["id"].as_str().unwrap_or_default()
        )
        .ok();
    }
}

fn reference_scope_files(
    definitions: &[SymbolDefinitionMatch<'_>],
    kgraph: &KGraph,
    scope_requested: bool,
) -> Option<std::collections::HashSet<PathBuf>> {
    if !scope_requested {
        return None;
    }

    if definitions.is_empty() {
        return Some(std::collections::HashSet::new());
    }

    let mut allowed = std::collections::HashSet::new();
    for definition in definitions {
        allowed.insert(definition.file.path.clone());
        for dependent in kgraph.transitive_dependents(&definition.file.path) {
            allowed.insert(dependent);
        }
    }
    Some(allowed)
}

fn source_context_line(
    root: &Path,
    file_path: &Path,
    line: usize,
    file_cache: &mut std::collections::HashMap<PathBuf, String>,
) -> String {
    if !file_cache.contains_key(file_path) {
        let full_path = root.join(file_path);
        if let Ok(content) = std::fs::read_to_string(&full_path) {
            file_cache.insert(file_path.to_path_buf(), content);
        }
    }

    file_cache
        .get(file_path)
        .and_then(|content| {
            content
                .lines()
                .nth(line.saturating_sub(1))
                .map(|l| l.trim().to_string())
        })
        .unwrap_or_default()
}

fn collect_call_references(
    root: &Path,
    file_nodes: &[kgr_core::types::FileNode],
    name: &str,
    allowed_files: Option<&std::collections::HashSet<PathBuf>>,
    definition_ids: &[String],
) -> Vec<SymbolReference> {
    let mut references = Vec::new();
    let mut file_cache: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();

    for file in file_nodes {
        if let Some(allowed) = allowed_files {
            if !allowed.contains(&file.path) {
                continue;
            }
        }

        for call in &file.calls {
            if callee_matches(&call.callee_raw, name) {
                let context =
                    source_context_line(root, &file.path, call.span.start_line, &mut file_cache);
                references.push(SymbolReference {
                    file: file.path.clone(),
                    value: serde_json::json!({
                        "file": file.path.to_string_lossy(),
                        "line": call.span.start_line,
                        "kind": "call",
                        "callee": &call.callee_raw,
                        "context": context,
                        "definition_ids": definition_ids,
                    }),
                });
            }
        }
    }

    references
}

fn reference_values(references: &[SymbolReference]) -> Vec<serde_json::Value> {
    references
        .iter()
        .map(|reference| reference.value.clone())
        .collect()
}

fn run_refs(
    name: &str,
    defined_in: Option<&Path>,
    path: &Path,
    format: Option<&str>,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) {
    let (root, mut file_nodes, format, _skipped_unsupported) =
        build_file_nodes(path, format, "table", &["table", "json"], lang, no_progress);

    let mut stdout = std::io::stdout().lock();
    let resolver = Resolver::new(&root, &file_nodes);
    resolver.resolve_all(&mut file_nodes);
    let kgraph = KGraph::from_files(&file_nodes);
    let defined_in = resolve_defined_in_scope(&format, &root, defined_in, &file_nodes, &mut stdout);
    let definitions = find_symbol_definitions(&file_nodes, name, defined_in.as_deref());
    let definition_jsons = symbol_definition_jsons(&definitions);
    let definition_ids = symbol_definition_ids(&definitions);
    let defined_in_json = defined_in
        .as_ref()
        .map(|scope| scope.to_string_lossy().to_string());

    if defined_in.is_none() && definitions.len() > 1 {
        if format == "json" {
            let result = serde_json::json!({
                "symbol": name,
                "defined_in": null,
                "ambiguous": true,
                "scope_hint": scope_hint(),
                "definitions": definition_jsons,
                "references": [],
            });
            serde_json::to_writer_pretty(&mut stdout, &result).ok();
            writeln!(stdout).ok();
        } else {
            write_ambiguous_symbol_text(&mut stdout, name, &definition_jsons);
        }
        return;
    }

    let allowed_files = reference_scope_files(&definitions, &kgraph, defined_in.is_some());
    let references = collect_call_references(
        &root,
        &file_nodes,
        name,
        allowed_files.as_ref(),
        &definition_ids,
    );
    let reference_values = reference_values(&references);

    if format == "json" {
        let result = serde_json::json!({
            "symbol": name,
            "defined_in": defined_in_json,
            "ambiguous": false,
            "definition_ids": definition_ids,
            "definitions": definition_jsons,
            "references": reference_values,
        });
        serde_json::to_writer_pretty(&mut stdout, &result).ok();
        writeln!(stdout).ok();
    } else {
        if definition_jsons.is_empty() && reference_values.is_empty() {
            eprintln!("No references found for '{name}'");
            return;
        }
        if !definition_jsons.is_empty() {
            writeln!(stdout, "Definitions of '{name}':").ok();
            for d in &definition_jsons {
                writeln!(
                    stdout,
                    "  {} ({}:{}) id={}",
                    d["file"].as_str().unwrap_or_default(),
                    d["kind"].as_str().unwrap_or_default(),
                    d["line"],
                    d["id"].as_str().unwrap_or_default()
                )
                .ok();
            }
        }
        if !reference_values.is_empty() {
            writeln!(stdout, "References to '{name}':").ok();
            for r in &reference_values {
                writeln!(
                    stdout,
                    "  {}:{} {}",
                    r["file"].as_str().unwrap_or_default(),
                    r["line"],
                    r["context"].as_str().unwrap_or_default()
                )
                .ok();
            }
        }
    }
}

fn run_dead(
    name: &str,
    defined_in: Option<&Path>,
    path: &Path,
    format: Option<&str>,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) {
    let (root, mut file_nodes, format, _skipped_unsupported) =
        build_file_nodes(path, format, "table", &["table", "json"], lang, no_progress);

    let mut stdout = std::io::stdout().lock();
    let resolver = Resolver::new(&root, &file_nodes);
    resolver.resolve_all(&mut file_nodes);
    let kgraph = KGraph::from_files(&file_nodes);
    let defined_in = resolve_defined_in_scope(&format, &root, defined_in, &file_nodes, &mut stdout);
    let definitions = find_symbol_definitions(&file_nodes, name, defined_in.as_deref());
    let definition_jsons = symbol_definition_jsons(&definitions);
    let definition_ids = symbol_definition_ids(&definitions);
    let defined_in_json = defined_in
        .as_ref()
        .map(|scope| scope.to_string_lossy().to_string());

    // Not found is a distinct verdict from dead: a typo'd or parser-missed
    // symbol must never read as a machine-removable `dead: true`.
    if definitions.is_empty() {
        if format == "json" {
            let result = serde_json::json!({
                "symbol": name,
                "defined_in": defined_in_json,
                "found": false,
                "dead": null,
                "ambiguous": false,
                "status": "not_found",
                "definitions": [],
                "references": [],
                "self_file_references": [],
                "cross_file_references": [],
                "caveat": null,
            });
            serde_json::to_writer_pretty(&mut stdout, &result).ok();
            writeln!(stdout).ok();
        } else {
            writeln!(stdout, "Symbol '{name}' not found in project.").ok();
        }
        return;
    }

    let allowed_files = reference_scope_files(&definitions, &kgraph, defined_in.is_some());
    let references = collect_call_references(
        &root,
        &file_nodes,
        name,
        allowed_files.as_ref(),
        &definition_ids,
    );
    let mut reference_values = Vec::new();
    let mut self_file_references = Vec::new();
    let mut cross_file_references = Vec::new();

    let definition_files: std::collections::HashSet<PathBuf> = definitions
        .iter()
        .map(|definition| definition.file.path.clone())
        .collect();

    for reference in references {
        if definition_files.contains(&reference.file) {
            self_file_references.push(reference.value.clone());
        } else {
            cross_file_references.push(reference.value.clone());
        }
        reference_values.push(reference.value);
    }

    let ambiguous = defined_in.is_none() && definitions.len() > 1;
    let precise_dead = cross_file_references.is_empty();
    let dead_json = if ambiguous && !cross_file_references.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Bool(precise_dead)
    };
    let (status, caveat) = if ambiguous && !cross_file_references.is_empty() {
        ("ambiguous", Some(scope_hint()))
    } else if reference_values.is_empty() {
        ("no_references", None)
    } else if cross_file_references.is_empty() {
        (
            "self_only_references",
            Some("Only self-file references found; no cross-file references were found."),
        )
    } else {
        ("live_cross_file_references", None)
    };

    if format == "json" {
        let result = serde_json::json!({
            "symbol": name,
            "defined_in": defined_in_json,
            "found": true,
            "dead": dead_json,
            "ambiguous": ambiguous,
            "status": status,
            "scope_hint": if ambiguous { Some(scope_hint()) } else { None },
            "definition_ids": definition_ids,
            "definitions": definition_jsons,
            "references": reference_values,
            "self_file_references": self_file_references,
            "cross_file_references": cross_file_references,
            "caveat": caveat,
        });
        serde_json::to_writer_pretty(&mut stdout, &result).ok();
        writeln!(stdout).ok();
    } else if reference_values.is_empty() {
        writeln!(stdout, "Dead — no references found.").ok();
        for def in &definition_jsons {
            writeln!(
                stdout,
                "  Defined at: {}:{} ({}) id={}",
                def["file"].as_str().unwrap_or_default(),
                def["line"],
                def["kind"].as_str().unwrap_or_default(),
                def["id"].as_str().unwrap_or_default()
            )
            .ok();
        }
    } else if cross_file_references.is_empty() {
        writeln!(
            stdout,
            "Dead — only self-file reference(s) found; no cross-file references."
        )
        .ok();
        for def in &definition_jsons {
            writeln!(
                stdout,
                "  Defined at: {}:{} ({}) id={}",
                def["file"].as_str().unwrap_or_default(),
                def["line"],
                def["kind"].as_str().unwrap_or_default(),
                def["id"].as_str().unwrap_or_default()
            )
            .ok();
        }
        writeln!(stdout, "Self-file references:").ok();
        for r in &self_file_references {
            writeln!(
                stdout,
                "  {}:{} {}",
                r["file"].as_str().unwrap_or_default(),
                r["line"],
                r["context"].as_str().unwrap_or_default()
            )
            .ok();
        }
    } else if ambiguous {
        write_ambiguous_symbol_text(&mut stdout, name, &definition_jsons);
        writeln!(stdout, "Potential cross-file references:").ok();
        for r in &cross_file_references {
            writeln!(
                stdout,
                "  {}:{} {}",
                r["file"].as_str().unwrap_or_default(),
                r["line"],
                r["context"].as_str().unwrap_or_default()
            )
            .ok();
        }
    } else {
        if definition_jsons.len() > 1 {
            writeln!(stdout, "Defined in {} locations:", definition_jsons.len()).ok();
            for def in &definition_jsons {
                writeln!(
                    stdout,
                    "  {}:{} ({}) id={}",
                    def["file"].as_str().unwrap_or_default(),
                    def["line"],
                    def["kind"].as_str().unwrap_or_default(),
                    def["id"].as_str().unwrap_or_default()
                )
                .ok();
            }
        }
        writeln!(
            stdout,
            "Not dead — {} cross-file reference(s) found:",
            cross_file_references.len()
        )
        .ok();
        for r in &cross_file_references {
            writeln!(
                stdout,
                "  {}:{} {}",
                r["file"].as_str().unwrap_or_default(),
                r["line"],
                r["context"].as_str().unwrap_or_default()
            )
            .ok();
        }
    }
}

fn run_agent_info(format: &str) {
    validate_format_or_exit(format, &["text", "json"]);
    if format == "json" {
        let json = serde_json::json!({ "guide": agent_docs::AGENT_DOCS });
        println!("{}", serde_json::to_string_pretty(&json).unwrap());
    } else {
        print!("{}", agent_docs::AGENT_DOCS);
    }
}

fn run_skeleton(path: &Path, format: Option<&str>, lang: &Option<Vec<String>>, no_progress: bool) {
    let (root, file_nodes, format, _skipped_unsupported) = build_file_nodes(
        path,
        format,
        "text",
        &["text", "json", "table"],
        lang,
        no_progress,
    );
    if file_nodes.is_empty() {
        return;
    }

    let mut stdout = std::io::stdout().lock();

    // Helper: extract signature from a source line
    fn extract_signature(line: &str) -> String {
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find('{') {
            let before = trimmed[..pos].trim_end();
            format!("{} {{ ... }}", before)
        } else {
            trimmed.to_string()
        }
    }

    if format == "json" {
        let entries: Vec<serde_json::Value> = file_nodes
            .iter()
            .filter(|f| !f.symbols.is_empty())
            .map(|f| {
                let rel = f.path.strip_prefix(&root).unwrap_or(&f.path);
                let source = std::fs::read_to_string(root.join(&f.path)).unwrap_or_default();
                let lines: Vec<&str> = source.lines().collect();

                let skeleton: Vec<serde_json::Value> = f
                    .symbols
                    .iter()
                    .filter_map(|s| {
                        let line_idx = s.span.start_line.checked_sub(1)?;
                        let src_line = lines.get(line_idx)?;
                        let sig = extract_signature(src_line);
                        Some(serde_json::json!({
                            "name": s.name,
                            "kind": s.kind.to_string(),
                            "line": s.span.start_line,
                            "exported": s.exported,
                            "signature": sig,
                        }))
                    })
                    .collect();

                serde_json::json!({
                    "file": rel.to_string_lossy(),
                    "skeleton": skeleton,
                })
            })
            .collect();
        serde_json::to_writer_pretty(&mut stdout, &entries).ok();
        writeln!(stdout).ok();
    } else if format == "table" {
        writeln!(
            stdout,
            "{:<50} {:<20} {:<10} {:>5}  SIGNATURE",
            "FILE", "SYMBOL", "KIND", "LINE"
        )
        .ok();
        writeln!(stdout, "{}", "-".repeat(120)).ok();
        for f in &file_nodes {
            let rel = f.path.strip_prefix(&root).unwrap_or(&f.path);
            let source = std::fs::read_to_string(root.join(&f.path)).unwrap_or_default();
            let lines: Vec<&str> = source.lines().collect();
            for s in &f.symbols {
                let sig = s
                    .span
                    .start_line
                    .checked_sub(1)
                    .and_then(|idx| lines.get(idx))
                    .map(|l| extract_signature(l))
                    .unwrap_or_default();
                writeln!(
                    stdout,
                    "{:<50} {:<20} {:<10} {:>5}  {}",
                    rel.display(),
                    s.name,
                    s.kind,
                    s.span.start_line,
                    sig
                )
                .ok();
            }
        }
    } else {
        // text format (default): emit source-like stubs
        for f in &file_nodes {
            if f.symbols.is_empty() {
                continue;
            }
            let rel = f.path.strip_prefix(&root).unwrap_or(&f.path);
            let source = std::fs::read_to_string(root.join(&f.path)).unwrap_or_default();
            let lines: Vec<&str> = source.lines().collect();

            writeln!(stdout, "// {}", rel.display()).ok();
            for s in &f.symbols {
                if let Some(line_idx) = s.span.start_line.checked_sub(1) {
                    if let Some(src_line) = lines.get(line_idx) {
                        let trimmed = src_line.trim();
                        // For functions/methods, ensure we have { ... }
                        match s.kind {
                            kgr_core::types::SymbolKind::Function
                            | kgr_core::types::SymbolKind::Method => {
                                if let Some(pos) = trimmed.find('{') {
                                    let before = trimmed[..pos].trim_end();
                                    writeln!(stdout, "{} {{ ... }}", before).ok();
                                } else if trimmed.ends_with(':') {
                                    // Python-style: def foo():
                                    writeln!(stdout, "{} ...", trimmed).ok();
                                } else {
                                    writeln!(stdout, "{} {{ ... }}", trimmed).ok();
                                }
                            }
                            kgr_core::types::SymbolKind::Class => {
                                if let Some(pos) = trimmed.find('{') {
                                    let before = trimmed[..pos].trim_end();
                                    writeln!(stdout, "{} {{ ... }}", before).ok();
                                } else if trimmed.ends_with(':') {
                                    writeln!(stdout, "{} ...", trimmed).ok();
                                } else {
                                    writeln!(stdout, "{} {{ ... }}", trimmed).ok();
                                }
                            }
                        }
                    }
                }
            }
            writeln!(stdout).ok();
        }
    }
}

fn run_orient(path: &Path, format: Option<&str>, lang: &Option<Vec<String>>, no_progress: bool) {
    use kgr_core::types::ImportKind;
    use std::collections::{HashMap, HashSet};

    let (root, mut file_nodes, format, skipped_unsupported) =
        build_file_nodes(path, format, "text", &["text", "json"], lang, no_progress);
    if file_nodes.is_empty() {
        return;
    }

    let resolver = Resolver::new(&root, &file_nodes);
    resolver.resolve_all(&mut file_nodes);
    let kgraph = KGraph::from_files(&file_nodes);
    let dep_graph = kgraph.to_dep_graph(root.clone(), file_nodes);

    // Language breakdown
    let mut lang_counts: HashMap<String, usize> = HashMap::new();
    for f in &dep_graph.files {
        *lang_counts.entry(f.lang.to_string()).or_insert(0) += 1;
    }

    // External packages
    let external_packages: HashSet<&str> = dep_graph
        .files
        .iter()
        .flat_map(|f| {
            f.imports
                .iter()
                .filter(|i| i.kind == ImportKind::External)
                .filter_map(|i| i.raw.split("::").next().filter(|pkg| !pkg.is_empty()))
        })
        .collect();

    // Entry points (roots) — relative paths
    let entry_points: Vec<String> = dep_graph
        .roots
        .iter()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .collect();

    // Heaviest files
    let heaviest = kgraph.heaviest();
    let heaviest_display: Vec<(String, usize)> = heaviest
        .iter()
        .take(3)
        .map(|(p, count)| {
            let rel = p
                .strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string();
            (rel, *count)
        })
        .collect();

    // Largest cycle size
    let largest_cycle_size = dep_graph.cycles.iter().map(|c| c.len()).max().unwrap_or(0);

    let mut stdout = std::io::stdout().lock();

    if format == "json" {
        // Sort languages for deterministic output
        let mut sorted_langs: Vec<_> = lang_counts.iter().collect();
        sorted_langs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let languages: serde_json::Map<String, serde_json::Value> = sorted_langs
            .into_iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
            .collect();

        let mut ext_sorted: Vec<&str> = external_packages.into_iter().collect();
        ext_sorted.sort();

        let heaviest_json: Vec<serde_json::Value> = heaviest_display
            .iter()
            .map(|(f, d)| serde_json::json!({"file": f, "dependents": d}))
            .collect();

        let mut json = serde_json::json!({
            "files": dep_graph.files.len(),
            "languages": languages,
            "edges": dep_graph.edges.len(),
            "entry_points": entry_points,
            "cycles": dep_graph.cycles.len(),
            "largest_cycle_size": largest_cycle_size,
            "orphans": dep_graph.orphans.len(),
            "external_packages": ext_sorted,
            "heaviest": heaviest_json,
        });

        // Reported only when non-empty so existing JSON consumers and
        // snapshots of fully-supported repos are unaffected.
        if !skipped_unsupported.is_empty() {
            json.as_object_mut().unwrap().insert(
                "skipped_unsupported".to_string(),
                skipped_unsupported_json(&skipped_unsupported),
            );
        }

        writeln!(stdout, "{}", serde_json::to_string_pretty(&json).unwrap()).ok();
    } else {
        // Text output — compact one-glance summary

        // Line 1: files (lang breakdown) | edges | cycles
        let mut lang_parts: Vec<_> = lang_counts.iter().collect();
        lang_parts.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let lang_str: Vec<String> = lang_parts
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();
        writeln!(
            stdout,
            "{} files ({}) | {} edges | {} cycles",
            dep_graph.files.len(),
            lang_str.join(", "),
            dep_graph.edges.len(),
            dep_graph.cycles.len(),
        )
        .ok();

        // Line 2: entry points
        if !entry_points.is_empty() {
            writeln!(stdout, "Entry points: {}", entry_points.join(", ")).ok();
        }

        // Line 3: heaviest
        if !heaviest_display.is_empty() {
            let parts: Vec<String> = heaviest_display
                .iter()
                .map(|(f, d)| format!("{} ({} deps)", f, d))
                .collect();
            writeln!(stdout, "Heaviest: {}", parts.join(", ")).ok();
        }

        // Line 4: external + orphans
        let mut ext_sorted: Vec<&str> = external_packages.into_iter().collect();
        ext_sorted.sort();
        let ext_str = if ext_sorted.is_empty() {
            "External: 0 packages".to_string()
        } else {
            format!(
                "External: {} packages ({})",
                ext_sorted.len(),
                ext_sorted.join(", ")
            )
        };
        writeln!(stdout, "{} | Orphans: {}", ext_str, dep_graph.orphans.len()).ok();

        // Line 5 (only when relevant): unsupported files the walk skipped.
        // Counts per extension, capped at the 5 largest groups, so even a
        // large repo full of unparseable assets stays one line.
        if !skipped_unsupported.is_empty() {
            let total: usize = skipped_unsupported.iter().map(|g| g.count).sum();
            let mut parts: Vec<String> = skipped_unsupported
                .iter()
                .take(5)
                .map(|g| format!("{}: {}", g.group, g.count))
                .collect();
            if skipped_unsupported.len() > 5 {
                parts.push(format!("+{} more", skipped_unsupported.len() - 5));
            }
            writeln!(
                stdout,
                "Skipped unsupported: {} file(s) ({})",
                total,
                parts.join(", ")
            )
            .ok();
        }
    }
}

fn run_impact(
    name: &str,
    path: &Path,
    format: Option<&str>,
    lang: &Option<Vec<String>>,
    depth: Option<usize>,
    no_progress: bool,
) {
    let (root, mut file_nodes, format, _skipped_unsupported) =
        build_file_nodes(path, format, "text", &["text", "json"], lang, no_progress);

    let resolver = Resolver::new(&root, &file_nodes);
    resolver.resolve_all(&mut file_nodes);

    let kgraph = KGraph::from_files(&file_nodes);

    // Find every file that defines the named symbol — common names (init,
    // main, helper) are often defined in several files, and the blast radius
    // must cover all of them.
    let mut definitions = Vec::new();
    let mut defining_files: Vec<PathBuf> = Vec::new();
    for f in &file_nodes {
        for s in &f.symbols {
            if s.name == name {
                definitions.push(serde_json::json!({
                    "file": f.path.to_string_lossy(),
                    "line": s.span.start_line,
                    "kind": s.kind.to_string(),
                }));
                if !defining_files.contains(&f.path) {
                    defining_files.push(f.path.clone());
                }
            }
        }
    }

    if definitions.is_empty() {
        if format == "json" {
            let result = serde_json::json!({
                "symbol": name,
                "found": false,
                "definitions": [],
                "impact": [],
                "error": format!("Symbol '{name}' not found"),
            });
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &result).ok();
            writeln!(stdout).ok();
        } else {
            eprintln!("Symbol '{name}' not found");
        }
        return;
    }

    // Union of transitive dependents across every defining file; when a
    // dependent is reachable from several definitions, the minimum depth wins.
    let mut depth_by_file: std::collections::HashMap<PathBuf, usize> =
        std::collections::HashMap::new();
    for def_file in &defining_files {
        for (p, d) in kgraph.transitive_dependents_with_depth(def_file, depth) {
            depth_by_file
                .entry(p)
                .and_modify(|existing| *existing = (*existing).min(d))
                .or_insert(d);
        }
    }
    let mut dependents: Vec<(PathBuf, usize)> = depth_by_file.into_iter().collect();
    dependents.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    // Cross-reference: for each dependent, check if it calls the symbol
    let calls_symbol: std::collections::HashMap<PathBuf, bool> = dependents
        .iter()
        .map(|(dep_path, _)| {
            let has_call = file_nodes.iter().any(|f| {
                f.path == *dep_path && f.calls.iter().any(|c| callee_matches(&c.callee_raw, name))
            });
            (dep_path.clone(), has_call)
        })
        .collect();

    let mut stdout = std::io::stdout().lock();

    if format == "json" {
        let impact: Vec<serde_json::Value> = dependents
            .iter()
            .map(|(p, d)| {
                serde_json::json!({
                    "file": p.to_string_lossy(),
                    "depth": d,
                    "calls_symbol": calls_symbol.get(p).copied().unwrap_or(false),
                })
            })
            .collect();

        let result = serde_json::json!({
            "symbol": name,
            "found": true,
            "definitions": definitions,
            "impact": impact,
        });
        serde_json::to_writer_pretty(&mut stdout, &result).ok();
        writeln!(stdout).ok();
    } else {
        if definitions.len() == 1 {
            writeln!(
                stdout,
                "Symbol: {name}\nDefined in: {}:{} ({})",
                definitions[0]["file"].as_str().unwrap_or_default(),
                definitions[0]["line"],
                definitions[0]["kind"].as_str().unwrap_or_default(),
            )
            .ok();
        } else {
            writeln!(
                stdout,
                "Symbol: {name}\nDefined in {} locations:",
                definitions.len()
            )
            .ok();
            for def in &definitions {
                writeln!(
                    stdout,
                    "  {}:{} ({})",
                    def["file"].as_str().unwrap_or_default(),
                    def["line"],
                    def["kind"].as_str().unwrap_or_default(),
                )
                .ok();
            }
        }
        writeln!(stdout).ok();

        if dependents.is_empty() {
            writeln!(stdout, "Impact: 0 files affected").ok();
        } else {
            writeln!(stdout, "Impact: {} files affected", dependents.len()).ok();

            // Group by depth
            let mut by_depth: std::collections::BTreeMap<usize, Vec<&PathBuf>> =
                std::collections::BTreeMap::new();
            for (p, d) in &dependents {
                by_depth.entry(*d).or_default().push(p);
            }

            for (d, files) in &by_depth {
                let label = if *d == 1 {
                    format!("  depth {d} (direct):")
                } else {
                    format!("  depth {d}:")
                };
                writeln!(stdout, "{label}").ok();
                for f in files {
                    let tag = if calls_symbol.get(*f).copied().unwrap_or(false) {
                        "  [calls symbol]"
                    } else {
                        ""
                    };
                    writeln!(stdout, "    {}{tag}", f.display()).ok();
                }
            }
        }
    }
}

fn run_hotspots(
    path: &Path,
    format: Option<&str>,
    lang: &Option<Vec<String>>,
    top: Option<usize>,
    no_progress: bool,
) {
    use kgr_core::types::SymbolKind;

    let (root, file_nodes, format, _skipped_unsupported) = build_file_nodes(
        path,
        format,
        "table",
        &["table", "json", "text"],
        lang,
        no_progress,
    );
    let limit = top.unwrap_or(20);

    #[derive(serde::Serialize)]
    struct HotspotEntry {
        file: String,
        functions: usize,
        avg_length: usize,
        max_length: usize,
        score: usize,
    }

    let mut entries: Vec<HotspotEntry> = file_nodes
        .iter()
        .filter_map(|node| {
            let fn_symbols: Vec<_> = node
                .symbols
                .iter()
                .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
                .collect();

            let function_count = fn_symbols.len();
            if function_count == 0 {
                return None;
            }

            let lengths: Vec<usize> = fn_symbols
                .iter()
                .map(|s| s.span.end_line - s.span.start_line + 1)
                .collect();

            let total_length: usize = lengths.iter().sum();
            let avg_length = total_length / function_count;
            let max_length = *lengths.iter().max().unwrap();
            let score = function_count * avg_length;

            let rel_path = node
                .path
                .strip_prefix(&root)
                .unwrap_or(&node.path)
                .to_string_lossy()
                .to_string();

            Some(HotspotEntry {
                file: rel_path,
                functions: function_count,
                avg_length,
                max_length,
                score,
            })
        })
        .collect();

    entries.sort_by_key(|b| std::cmp::Reverse(b.score));
    entries.truncate(limit);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match format.as_str() {
        "json" => {
            let _ = serde_json::to_writer_pretty(&mut out, &entries);
            let _ = writeln!(out);
        }
        "text" => {
            for (i, entry) in entries.iter().enumerate() {
                writeln!(
                    out,
                    "{}. {} ({} functions, avg {} lines, score {})",
                    i + 1,
                    entry.file,
                    entry.functions,
                    entry.avg_length,
                    entry.score,
                )
                .ok();
            }
        }
        _ => {
            // table (default)
            let _ = writeln!(
                out,
                "{:<55} {:>9}  {:>7}  {:>7}  {:>5}",
                "FILE", "FUNCTIONS", "AVG_LEN", "MAX_LEN", "SCORE"
            );
            let _ = writeln!(out, "{}", "-".repeat(89));
            for entry in &entries {
                let _ = writeln!(
                    out,
                    "{:<55} {:>9}  {:>7}  {:>7}  {:>5}",
                    entry.file, entry.functions, entry.avg_length, entry.max_length, entry.score,
                );
            }
        }
    }
}

/// Byte range of lines `start_line..=end_line` (1-based, inclusive) in `content`,
/// including the trailing newline of `end_line` when present. Used so
/// `--no-linenos` output round-trips byte-identical to the source window.
fn line_byte_range(content: &str, start_line: usize, end_line: usize) -> (usize, usize) {
    let mut line_starts = vec![0usize];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let start = line_starts
        .get(start_line - 1)
        .copied()
        .unwrap_or(content.len());
    let end = line_starts.get(end_line).copied().unwrap_or(content.len());
    (start, end)
}

fn write_numbered_window(out: &mut impl Write, lines: &[&str], start_line: usize, end_line: usize) {
    let width = end_line.to_string().len().max(4);
    for n in start_line..=end_line {
        if let Some(line) = lines.get(n - 1) {
            writeln!(out, "{:>width$}  {}", n, line, width = width).ok();
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch passes through all flags"
)]
fn run_show(
    name: &str,
    path: &Path,
    context: usize,
    all: bool,
    kind: Option<&str>,
    format: Option<&str>,
    no_linenos: bool,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) {
    use kgr_core::types::SymbolKind;

    let kind_filter = kind.map(|k| match k {
        "fn" | "function" => SymbolKind::Function,
        "class" => SymbolKind::Class,
        "method" => SymbolKind::Method,
        other => {
            eprintln!("Error: unknown kind '{other}' (expected fn, class, or method)");
            process::exit(2);
        }
    });

    let (root, file_nodes, format, _skipped_unsupported) =
        build_file_nodes(path, format, "text", &["text", "json"], lang, no_progress);

    let mut matches: Vec<(&PathBuf, &kgr_core::types::Symbol)> = file_nodes
        .iter()
        .flat_map(|f| f.symbols.iter().map(move |s| (&f.path, s)))
        .filter(|(_, s)| s.name == name && kind_filter.map_or(true, |k| s.kind == k))
        .collect();
    matches.sort_by(|a, b| (a.0, a.1.span.start_line).cmp(&(b.0, b.1.span.start_line)));

    if matches.is_empty() {
        eprintln!("Symbol '{name}' not found");
        let mut suggestions: Vec<&str> = file_nodes
            .iter()
            .flat_map(|f| f.symbols.iter())
            .filter(|s| {
                s.name.eq_ignore_ascii_case(name)
                    || s.name.to_lowercase().contains(&name.to_lowercase())
            })
            .map(|s| s.name.as_str())
            .collect();
        suggestions.sort_unstable();
        suggestions.dedup();
        if !suggestions.is_empty() {
            suggestions.truncate(5);
            eprintln!("Did you mean: {}?", suggestions.join(", "));
        }
        process::exit(1);
    }

    // Read each matched file once and compute the printed window per match
    let mut file_cache: std::collections::HashMap<&PathBuf, String> =
        std::collections::HashMap::new();
    let mut stdout = std::io::stdout().lock();
    let printed = if all { matches.len() } else { 1 };

    if format == "json" {
        let entries: Vec<serde_json::Value> = matches
            .iter()
            .enumerate()
            .map(|(i, (fpath, s))| {
                let body = (i < printed).then(|| {
                    let content = file_cache
                        .entry(fpath)
                        .or_insert_with(|| {
                            std::fs::read_to_string(root.join(fpath)).unwrap_or_default()
                        })
                        .as_str();
                    let total = content.lines().count();
                    let start = s.span.start_line.saturating_sub(context).max(1);
                    let end = (s.span.end_line + context).min(total.max(1));
                    let (b0, b1) = line_byte_range(content, start, end);
                    content[b0..b1].to_string()
                });
                serde_json::json!({
                    "name": s.name,
                    "kind": s.kind.to_string(),
                    "path": fpath.to_string_lossy(),
                    "start_line": s.span.start_line,
                    "end_line": s.span.end_line,
                    "exported": s.exported,
                    "body": body,
                })
            })
            .collect();
        serde_json::to_writer_pretty(&mut stdout, &entries).ok();
        writeln!(stdout).ok();
        return;
    }

    for (i, (fpath, s)) in matches.iter().enumerate() {
        if i >= printed {
            writeln!(
                stdout,
                "also: {}:{} ({})",
                fpath.display(),
                s.span.start_line,
                s.kind
            )
            .ok();
            continue;
        }
        let content = file_cache
            .entry(fpath)
            .or_insert_with(|| std::fs::read_to_string(root.join(fpath)).unwrap_or_default())
            .as_str();
        let total = content.lines().count();
        let start = s.span.start_line.saturating_sub(context).max(1);
        let end = (s.span.end_line + context).min(total.max(1));

        if no_linenos {
            let (b0, b1) = line_byte_range(content, start, end);
            write!(stdout, "{}", &content[b0..b1]).ok();
        } else {
            writeln!(
                stdout,
                "── {}:{}-{} ({} {}) ──",
                fpath.display(),
                start,
                end,
                s.kind,
                s.name
            )
            .ok();
            let lines: Vec<&str> = content.lines().collect();
            write_numbered_window(&mut stdout, &lines, start, end);
            if i + 1 < printed {
                writeln!(stdout).ok();
            }
        }
    }
}

fn run_slice(
    target: &str,
    start_pos: Option<usize>,
    end_pos: Option<usize>,
    context: Option<usize>,
    no_linenos: bool,
    format: &str,
    max: usize,
) {
    // Accept `file:start`, `file:start-end`, and `file start end` (positional)
    let (file, start, end) = if let Some(start) = start_pos {
        (target.to_string(), start, end_pos)
    } else {
        let parsed = target.rsplit_once(':').and_then(|(file, loc)| {
            let (s, e) = match loc.split_once('-') {
                Some((s, e)) => (s.parse().ok()?, Some(e.parse().ok()?)),
                None => (loc.parse().ok()?, None),
            };
            Some((file.to_string(), s, e))
        });
        match parsed {
            Some(p) => p,
            None => {
                eprintln!("Error: expected <file>:<start>[-<end>] or <file> <start> [<end>]");
                process::exit(2);
            }
        }
    };

    if start == 0 || end.is_some_and(|e| e < start) {
        eprintln!("Error: lines are 1-based and end must be >= start");
        process::exit(2);
    }

    let content = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        eprintln!("Error: cannot read '{file}': {e}");
        process::exit(2);
    });
    let total = content.lines().count().max(1);

    let (mut start, mut end) = match end {
        Some(e) => (start, e),
        None => {
            let ctx = context.unwrap_or(10);
            (start.saturating_sub(ctx).max(1), start + ctx)
        }
    };
    if start > total {
        eprintln!("note: start {start} is beyond EOF (file has {total} lines)");
        start = total;
    }
    if end > total {
        if format != "json" && !no_linenos {
            eprintln!("note: end clamped to EOF ({total} lines)");
        }
        end = total;
    }
    if end - start + 1 > max {
        end = start + max - 1;
        if format != "json" && !no_linenos {
            eprintln!("note: output capped at {max} lines (use --max to raise)");
        }
    }

    let mut stdout = std::io::stdout().lock();

    if format == "json" {
        let lines: Vec<&str> = content
            .lines()
            .skip(start - 1)
            .take(end - start + 1)
            .collect();
        let json = serde_json::json!({
            "path": file,
            "start_line": start,
            "end_line": end,
            "lines": lines,
        });
        serde_json::to_writer_pretty(&mut stdout, &json).ok();
        writeln!(stdout).ok();
    } else if no_linenos {
        let (b0, b1) = line_byte_range(&content, start, end);
        write!(stdout, "{}", &content[b0..b1]).ok();
    } else {
        let lines: Vec<&str> = content.lines().collect();
        write_numbered_window(&mut stdout, &lines, start, end);
    }
}

#[cfg(test)]
mod tests {
    use crate::test_env::{CleanKgrEnv, KGR_ENV_LOCK};

    use super::{callee_matches, replace_executable_atomically, UpgradeReplacement};

    #[test]
    fn callee_matches_bare_name() {
        assert!(callee_matches("helper", "helper"));
    }

    #[test]
    fn callee_matches_dot_qualified() {
        assert!(callee_matches("obj.helper", "helper"));
    }

    #[test]
    fn callee_matches_scoped_path() {
        assert!(callee_matches("util::helper", "helper"));
        assert!(callee_matches("Foo::bar", "bar"));
        assert!(callee_matches("crate::util::helper", "helper"));
        assert!(callee_matches("tracing::warn", "warn"));
    }

    #[test]
    fn callee_matches_rejects_suffix_overlap() {
        // Name must follow a separator (or match whole) — not a substring tail.
        assert!(!callee_matches("unhelper", "helper"));
        assert!(!callee_matches("util::unhelper", "helper"));
        assert!(!callee_matches("obj.unhelper", "helper"));
    }

    #[test]
    fn upgrade_replacement_skips_same_executable_without_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("kgr");
        std::fs::write(&bin, "fresh binary").unwrap();

        let result = replace_executable_atomically(&bin, &bin).unwrap();

        assert_eq!(result, UpgradeReplacement::SkippedSameExecutable);
        assert_eq!(std::fs::read_to_string(&bin).unwrap(), "fresh binary");
    }

    #[test]
    fn upgrade_replacement_renames_temp_over_destination() {
        let dir = tempfile::tempdir().unwrap();
        let new_bin = dir.path().join("target-kgr");
        let dest = dir.path().join("installed-kgr");
        std::fs::write(&new_bin, "new binary").unwrap();
        std::fs::write(&dest, "old binary").unwrap();

        let result = replace_executable_atomically(&new_bin, &dest).unwrap();

        assert_eq!(result, UpgradeReplacement::Replaced);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new binary");
        assert_eq!(std::fs::read_to_string(&new_bin).unwrap(), "new binary");
    }

    /// End-to-end through `run_graph`: config `languages` acts as the
    /// default --lang filter and config `format` as the default --format
    /// when the CLI flags are absent.
    /// (Config writes "json" — same value the lone env-manipulating config
    /// test sets for KGR_FORMAT, so even a parallel-test overlap cannot
    /// change the outcome.)
    #[test]
    fn config_defaults_drive_run_graph_when_flags_absent() {
        let _env_lock = KGR_ENV_LOCK.lock().unwrap();
        let _env = CleanKgrEnv::new();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.py"), "import helper\n").unwrap();
        std::fs::write(dir.path().join("helper.py"), "x = 1\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            dir.path().join(".kgr.toml"),
            "languages = [\"py\"]\nformat = \"json\"\nno_progress = true\n",
        )
        .unwrap();

        let out = dir.path().join("out.json");
        super::run_graph(
            dir.path(),
            None,
            &None,
            false,
            false,
            false,
            false,
            false,
            Some(&out),
        );

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        let files: Vec<&str> = json["files"]
            .as_array()
            .expect("config format=json must produce JSON output")
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(files.iter().any(|p| p.ends_with("app.py")));
        // languages = ["py"] excludes the Rust file when --lang is absent.
        assert!(!files.iter().any(|p| p.ends_with("main.rs")));
    }

    /// CLI flags win over config defaults: --format dot and --lang rs
    /// override config format/languages.
    #[test]
    fn cli_flags_beat_config_defaults_in_run_graph() {
        let _env_lock = KGR_ENV_LOCK.lock().unwrap();
        let _env = CleanKgrEnv::new();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.py"), "import helper\n").unwrap();
        std::fs::write(dir.path().join("helper.py"), "x = 1\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            dir.path().join(".kgr.toml"),
            "languages = [\"py\"]\nformat = \"tree\"\nno_progress = true\n",
        )
        .unwrap();

        let out = dir.path().join("out.dot");
        super::run_graph(
            dir.path(),
            Some("dot"),
            &Some(vec!["rs".to_string()]),
            false,
            false,
            false,
            false,
            false,
            Some(&out),
        );

        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("digraph"));
        assert!(content.contains("main.rs"));
        assert!(!content.contains("app.py"));
    }

    /// End-to-end through walk -> parse -> extract_calls -> callee_matches:
    /// a function invoked only as `util::helper()` must not look dead.
    /// This is exactly the liveness predicate `run_dead` applies.
    #[test]
    fn scoped_rust_call_is_visible_to_dead_check() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(
            src.join("main.rs"),
            "mod util;\n\nfn main() {\n    util::helper();\n    tracing::warn!(\"x\");\n}\n",
        )
        .unwrap();
        std::fs::write(src.join("util.rs"), "pub fn helper() {}\n").unwrap();

        let (_root, file_nodes, _format, _skipped_unsupported) =
            super::build_file_nodes(dir.path(), None, "table", &["table", "json"], &None, true);

        // The definition is found...
        assert!(file_nodes
            .iter()
            .any(|f| f.symbols.iter().any(|s| s.name == "helper")));
        // ...and the scoped call site keeps it alive.
        assert!(file_nodes.iter().any(|f| f
            .calls
            .iter()
            .any(|c| callee_matches(&c.callee_raw, "helper"))));
        // Scoped macros are captured as call refs too.
        assert!(file_nodes
            .iter()
            .any(|f| f.calls.iter().any(|c| c.callee_raw == "tracing::warn")));
    }
}

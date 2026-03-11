mod agent_docs;
mod baseline;
mod cache;
mod config;
mod pipeline;
mod render;
mod rules;
mod walk;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};

use kgr_core::graph::KGraph;
use kgr_core::parse::ParserRegistry;
use kgr_core::resolve::Resolver;

#[derive(Parser)]
#[command(
    name = "kgr",
    version,
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

        /// Output format: json, tree, dot
        #[arg(short, long, default_value = "tree")]
        format: String,

        /// Filter by language: py, ts, js, rs, java, c, cpp, go, zig, cs, objc, swift, rb, php, scala, lua, ex, hs, sh
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Hide external dependencies
        #[arg(long)]
        no_external: bool,

        /// Show external package names in tree/table output
        #[arg(long)]
        show_external: bool,

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

        /// Output format: text, json
        #[arg(short, long, default_value = "text")]
        format: String,

        /// Filter by language
        #[arg(short, long)]
        lang: Option<Vec<String>>,

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

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Query the dependency graph
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

        /// List the largest cycle
        #[arg(long)]
        largest_cycle: bool,

        /// Output format
        #[arg(short, long, default_value = "table")]
        format: String,

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

        /// Output format: table, json
        #[arg(short, long, default_value = "table")]
        format: String,

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

        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: table, json
        #[arg(short, long, default_value = "table")]
        format: String,

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

        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: table, json
        #[arg(short, long, default_value = "table")]
        format: String,

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

        /// Output format: text, json, table
        #[arg(short, long, default_value = "text")]
        format: String,

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

        /// Output format: text, json
        #[arg(short, long, default_value = "text")]
        format: String,

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

        /// Output format: text, json
        #[arg(short, long, default_value = "text")]
        format: String,

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

        /// Output format: text, json, table
        #[arg(short, long, default_value = "table")]
        format: String,

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

    /// Generate a .kgr.toml configuration file
    Init {
        /// Directory to initialize
        #[arg(default_value = ".")]
        path: PathBuf,
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
            no_progress,
            symbols,
            output,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_graph(
                &path,
                &format,
                &lang,
                no_external,
                show_external,
                no_progress,
                symbols,
                output.as_deref(),
            );
        }
        Some(Commands::Check {
            path,
            format,
            lang,
            no_progress,
            update_baseline,
            baseline,
            syntax,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_check(
                &path,
                &format,
                &lang,
                no_progress,
                update_baseline,
                baseline.as_deref(),
                syntax,
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
            largest_cycle,
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
                largest_cycle,
                &format,
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
            run_symbols(&path, &format, &lang, no_progress);
        }
        Some(Commands::Refs {
            name,
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_refs(&name, &path, &format, &lang, no_progress);
        }
        Some(Commands::Dead {
            name,
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_dead(&name, &path, &format, &lang, no_progress);
        }
        Some(Commands::Skeleton {
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_skeleton(&path, &format, &lang, no_progress);
        }
        Some(Commands::Orient {
            path,
            format,
            lang,
            no_progress,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_orient(&path, &format, &lang, no_progress);
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
            run_impact(&name, &path, &format, &lang, depth, no_progress);
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
            run_hotspots(&path, &format, &lang, top, no_progress);
        }
        Some(Commands::Init { path }) => {
            run_init(&path);
        }
        Some(Commands::Upgrade) => {
            run_upgrade();
        }
        Some(Commands::AgentInfo { format }) => {
            run_agent_info(&format);
        }
        None => {
            // Default: run graph with tree format on current directory
            setup_tracing(0);
            run_graph(
                &PathBuf::from("."),
                "tree",
                &None,
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

#[allow(clippy::too_many_arguments)]
fn run_graph(
    path: &PathBuf,
    format: &str,
    lang: &Option<Vec<String>>,
    no_external: bool,
    show_external: bool,
    no_progress: bool,
    include_symbols: bool,
    output: Option<&std::path::Path>,
) {
    let root = std::fs::canonicalize(path).unwrap_or_else(|e| {
        eprintln!("Error: cannot access '{}': {}", path.display(), e);
        process::exit(2);
    });

    let cfg = config::load_config(&root);
    let registry = ParserRegistry::new();
    let files = walk::discover(&root, lang, &cfg.exclude, cfg.max_file_size_bytes());

    if files.is_empty() {
        eprintln!("No supported source files found in {}", root.display());
        return;
    }

    tracing::info!("Discovered {} files", files.len());

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let mut file_nodes =
        pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    let resolver = Resolver::new(PathBuf::new(), &file_nodes);
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
    let dep_graph = kgraph.to_dep_graph(root, file_nodes);

    let mut writer: Box<dyn std::io::Write> = if let Some(out_path) = output {
        Box::new(std::fs::File::create(out_path).unwrap_or_else(|e| {
            eprintln!("Error: cannot create output file: {}", e);
            process::exit(2);
        }))
    } else {
        Box::new(std::io::stdout().lock())
    };

    // When --symbols is passed with JSON format, inject symbols into the output
    if include_symbols && format == "json" {
        let symbols_map: std::collections::HashMap<_, _> =
            symbols_data.unwrap().into_iter().collect();
        let mut json: serde_json::Value = serde_json::to_value(&dep_graph).unwrap();
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
                            "exported": s.exported,
                        }))
                        .collect::<Vec<_>>());
                } else {
                    file["symbols"] = serde_json::json!([]);
                }
            }
        }
        serde_json::to_writer_pretty(&mut writer, &json).ok();
        writeln!(writer).ok();
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

#[allow(clippy::too_many_arguments)]
fn run_check(
    path: &PathBuf,
    format: &str,
    lang: &Option<Vec<String>>,
    no_progress: bool,
    update_baseline: bool,
    baseline_path: Option<&Path>,
    syntax: bool,
) {
    let root = std::fs::canonicalize(path).unwrap_or_else(|e| {
        eprintln!("Error: cannot access '{}': {}", path.display(), e);
        process::exit(2);
    });

    let cfg = config::load_config(&root);
    let registry = ParserRegistry::new();
    let files = walk::discover(&root, lang, &cfg.exclude, cfg.max_file_size_bytes());

    if files.is_empty() {
        eprintln!("No supported source files found in {}", root.display());
        return;
    }

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let mut file_nodes =
        pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    let resolver = Resolver::new(PathBuf::new(), &file_nodes);
    resolver.resolve_all(&mut file_nodes);

    let kgraph = KGraph::from_files(&file_nodes);
    let dep_graph = kgraph.to_dep_graph(root.clone(), file_nodes);

    let all_rule_violations = rules::check_rules(&dep_graph, &cfg.rules);
    let resolved_baseline_path = baseline_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".kgr-baseline.json"));

    // --update-baseline: record current state and exit 0
    if update_baseline {
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
    let bl = baseline::Baseline::load(&resolved_baseline_path);
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

        if has_errors {
            // error messages already printed above
        } else {
            eprintln!("All checks passed.");
        }
    }

    if has_errors {
        process::exit(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_query(
    path: &PathBuf,
    who_imports: Option<&Path>,
    deps_of: Option<&Path>,
    path_between: Option<&[PathBuf]>,
    cycles: bool,
    orphans: bool,
    heaviest: bool,
    largest_cycle: bool,
    format: &str,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) {
    let root = std::fs::canonicalize(path).unwrap_or_else(|e| {
        eprintln!("Error: cannot access '{}': {}", path.display(), e);
        process::exit(2);
    });

    let cfg = config::load_config(&root);
    let registry = ParserRegistry::new();
    let files = walk::discover(&root, lang, &cfg.exclude, cfg.max_file_size_bytes());

    if files.is_empty() {
        eprintln!("No supported source files found in {}", root.display());
        return;
    }

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let mut file_nodes =
        pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    let resolver = Resolver::new(PathBuf::new(), &file_nodes);
    resolver.resolve_all(&mut file_nodes);

    let kgraph = KGraph::from_files(&file_nodes);
    let dep_graph = kgraph.to_dep_graph(root, file_nodes);

    let mut stdout = std::io::stdout().lock();

    if let Some(target) = who_imports {
        let dependents = kgraph.transitive_dependents(target);
        if dependents.is_empty() {
            eprintln!("No files import {}", target.display());
        } else if format == "json" {
            serde_json::to_writer_pretty(&mut stdout, &dependents).ok();
            writeln!(stdout).ok();
        } else {
            writeln!(stdout, "Files that import {}:", target.display()).ok();
            for dep in &dependents {
                writeln!(stdout, "  {}", dep.display()).ok();
            }
        }
    } else if let Some(target) = deps_of {
        let deps = kgraph.transitive_deps(target, None);
        if deps.is_empty() {
            eprintln!("{} has no dependencies", target.display());
        } else if format == "json" {
            serde_json::to_writer_pretty(&mut stdout, &deps).ok();
            writeln!(stdout).ok();
        } else {
            writeln!(stdout, "Dependencies of {}:", target.display()).ok();
            for dep in &deps {
                writeln!(stdout, "  {}", dep.display()).ok();
            }
        }
    } else if let Some(endpoints) = path_between {
        if endpoints.len() == 2 {
            if let Some(path) = kgraph.shortest_path(&endpoints[0], &endpoints[1]) {
                if format == "json" {
                    serde_json::to_writer_pretty(&mut stdout, &path).ok();
                    writeln!(stdout).ok();
                } else {
                    writeln!(
                        stdout,
                        "Shortest path from {} to {}:",
                        endpoints[0].display(),
                        endpoints[1].display()
                    )
                    .ok();
                    for (i, node) in path.iter().enumerate() {
                        if i > 0 {
                            write!(stdout, " -> ").ok();
                        }
                        write!(stdout, "{}", node.display()).ok();
                    }
                    writeln!(stdout).ok();
                }
            } else {
                eprintln!(
                    "No path found from {} to {}",
                    endpoints[0].display(),
                    endpoints[1].display()
                );
            }
        }
    } else if cycles {
        if dep_graph.cycles.is_empty() {
            eprintln!("No cycles found");
        } else if format == "json" {
            serde_json::to_writer_pretty(&mut stdout, &dep_graph.cycles).ok();
            writeln!(stdout).ok();
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
        if dep_graph.orphans.is_empty() {
            eprintln!("No orphaned files found");
        } else if format == "json" {
            serde_json::to_writer_pretty(&mut stdout, &dep_graph.orphans).ok();
            writeln!(stdout).ok();
        } else {
            writeln!(stdout, "Orphaned files:").ok();
            for orphan in &dep_graph.orphans {
                writeln!(stdout, "  {}", orphan.display()).ok();
            }
        }
    } else if heaviest {
        let ranked = kgraph.heaviest();
        if format == "json" {
            let items: Vec<serde_json::Value> = ranked
                .iter()
                .take(20)
                .map(|(p, c)| {
                    serde_json::json!({
                        "path": p,
                        "dependents": c
                    })
                })
                .collect();
            serde_json::to_writer_pretty(&mut stdout, &items).ok();
            writeln!(stdout).ok();
        } else {
            writeln!(stdout, "{:<50} {:>10}", "FILE", "DEPENDENTS").ok();
            writeln!(stdout, "{}", "-".repeat(62)).ok();
            for (path, count) in ranked.iter().take(20) {
                writeln!(stdout, "{:<50} {:>10}", path.display(), count).ok();
            }
        }
    } else if largest_cycle {
        if let Some(cycle) = dep_graph.cycles.iter().max_by_key(|c| c.len()) {
            if format == "json" {
                serde_json::to_writer_pretty(&mut stdout, cycle).ok();
                writeln!(stdout).ok();
            } else {
                writeln!(stdout, "Largest cycle ({} files):", cycle.len()).ok();
                for node in cycle {
                    writeln!(stdout, "  {}", node.display()).ok();
                }
            }
        } else {
            eprintln!("No cycles found");
        }
    } else {
        eprintln!("Please specify a query flag. Run `kgr query --help` for options.");
        process::exit(2);
    }
}

fn run_init(path: &Path) {
    match config::init_config(path) {
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

    // Copy the newly built binary over the current exe.
    let new_bin = workspace_root.join("target/release/kgr");
    std::fs::copy(&new_bin, &dest).unwrap_or_else(|e| {
        eprintln!(
            "Error: failed to copy {} to {}: {}",
            new_bin.display(),
            dest.display(),
            e
        );
        process::exit(2);
    });

    eprintln!("kgr upgraded successfully.");
    eprintln!("Version: {}", env!("CARGO_PKG_VERSION"));
}

fn build_file_nodes(
    path: &PathBuf,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) -> (PathBuf, Vec<kgr_core::types::FileNode>) {
    let root = std::fs::canonicalize(path).unwrap_or_else(|e| {
        eprintln!("Error: cannot access '{}': {}", path.display(), e);
        process::exit(2);
    });

    let cfg = config::load_config(&root);
    let registry = ParserRegistry::new();
    let files = walk::discover(&root, lang, &cfg.exclude, cfg.max_file_size_bytes());

    if files.is_empty() {
        eprintln!("No supported source files found in {}", root.display());
        return (root, Vec::new());
    }

    tracing::info!("Discovered {} files", files.len());

    let cache_path = root.join(".kgr-cache.json");
    let mut parse_cache = cache::ParseCache::load(&cache_path);
    let file_nodes = pipeline::parse_all(&root, files, &registry, &mut parse_cache, !no_progress);
    parse_cache.save(&cache_path);

    (root, file_nodes)
}

fn run_symbols(path: &PathBuf, format: &str, lang: &Option<Vec<String>>, no_progress: bool) {
    let (root, file_nodes) = build_file_nodes(path, lang, no_progress);
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

fn run_refs(
    name: &str,
    path: &PathBuf,
    format: &str,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) {
    let (root, file_nodes) = build_file_nodes(path, lang, no_progress);

    // Find definitions: symbols matching the name
    let mut definitions = Vec::new();
    for f in &file_nodes {
        for s in &f.symbols {
            if s.name == name {
                definitions.push(serde_json::json!({
                    "file": f.path.to_string_lossy(),
                    "line": s.span.start_line,
                    "kind": s.kind.to_string(),
                }));
            }
        }
    }

    // Find call references: calls where callee matches name
    let mut references = Vec::new();
    // Cache file reads for context extraction
    let mut file_cache: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();
    for f in &file_nodes {
        for c in &f.calls {
            let matches = c.callee_raw == name || c.callee_raw.ends_with(&format!(".{name}"));
            if matches {
                // Read source for context line
                let context = if !file_cache.contains_key(&f.path) {
                    let full_path = root.join(&f.path);
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        file_cache.insert(f.path.clone(), content);
                    }
                    file_cache
                        .get(&f.path)
                        .and_then(|content| {
                            content
                                .lines()
                                .nth(c.span.start_line - 1)
                                .map(|l| l.trim().to_string())
                        })
                        .unwrap_or_default()
                } else {
                    file_cache
                        .get(&f.path)
                        .and_then(|content| {
                            content
                                .lines()
                                .nth(c.span.start_line - 1)
                                .map(|l| l.trim().to_string())
                        })
                        .unwrap_or_default()
                };

                references.push(serde_json::json!({
                    "file": f.path.to_string_lossy(),
                    "line": c.span.start_line,
                    "kind": "call",
                    "context": context,
                }));
            }
        }
    }

    let mut stdout = std::io::stdout().lock();

    if format == "json" {
        let result = serde_json::json!({
            "symbol": name,
            "definitions": definitions,
            "references": references,
        });
        serde_json::to_writer_pretty(&mut stdout, &result).ok();
        writeln!(stdout).ok();
    } else {
        if definitions.is_empty() && references.is_empty() {
            eprintln!("No references found for '{name}'");
            return;
        }
        if !definitions.is_empty() {
            writeln!(stdout, "Definitions of '{name}':").ok();
            for d in &definitions {
                writeln!(
                    stdout,
                    "  {} ({}:{})",
                    d["file"].as_str().unwrap(),
                    d["kind"].as_str().unwrap(),
                    d["line"]
                )
                .ok();
            }
        }
        if !references.is_empty() {
            writeln!(stdout, "References to '{name}':").ok();
            for r in &references {
                writeln!(
                    stdout,
                    "  {}:{} {}",
                    r["file"].as_str().unwrap(),
                    r["line"],
                    r["context"].as_str().unwrap_or("")
                )
                .ok();
            }
        }
    }
}

fn run_dead(
    name: &str,
    path: &PathBuf,
    format: &str,
    lang: &Option<Vec<String>>,
    no_progress: bool,
) {
    let (root, file_nodes) = build_file_nodes(path, lang, no_progress);

    // Find definition
    let mut definition = None;
    for f in &file_nodes {
        for s in &f.symbols {
            if s.name == name {
                definition = Some(serde_json::json!({
                    "file": f.path.to_string_lossy(),
                    "line": s.span.start_line,
                    "kind": s.kind.to_string(),
                }));
                break;
            }
        }
        if definition.is_some() {
            break;
        }
    }

    // Find call references
    let mut references = Vec::new();
    let mut file_cache: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();
    for f in &file_nodes {
        for c in &f.calls {
            let matches = c.callee_raw == name || c.callee_raw.ends_with(&format!(".{name}"));
            if matches {
                if !file_cache.contains_key(&f.path) {
                    let full_path = root.join(&f.path);
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        file_cache.insert(f.path.clone(), content);
                    }
                }
                let context = file_cache
                    .get(&f.path)
                    .and_then(|content| {
                        content
                            .lines()
                            .nth(c.span.start_line - 1)
                            .map(|l| l.trim().to_string())
                    })
                    .unwrap_or_default();

                references.push(serde_json::json!({
                    "file": f.path.to_string_lossy(),
                    "line": c.span.start_line,
                    "kind": "call",
                    "context": context,
                }));
            }
        }
    }

    let dead = references.is_empty();
    let mut stdout = std::io::stdout().lock();

    if format == "json" {
        let result = if let Some(def) = &definition {
            serde_json::json!({
                "symbol": name,
                "dead": dead,
                "definition": def,
                "references": references,
            })
        } else {
            serde_json::json!({
                "symbol": name,
                "dead": true,
                "definition": null,
                "references": [],
            })
        };
        serde_json::to_writer_pretty(&mut stdout, &result).ok();
        writeln!(stdout).ok();
    } else if definition.is_none() {
        writeln!(stdout, "Symbol '{name}' not found in project.").ok();
    } else if dead {
        let def = definition.unwrap();
        writeln!(stdout, "Dead — no references found.").ok();
        writeln!(
            stdout,
            "  Defined at: {}:{} ({})",
            def["file"].as_str().unwrap(),
            def["line"],
            def["kind"].as_str().unwrap()
        )
        .ok();
    } else {
        writeln!(
            stdout,
            "Not dead — {} reference(s) found:",
            references.len()
        )
        .ok();
        for r in &references {
            writeln!(
                stdout,
                "  {}:{} {}",
                r["file"].as_str().unwrap(),
                r["line"],
                r["context"].as_str().unwrap_or("")
            )
            .ok();
        }
    }
}

fn run_agent_info(format: &str) {
    if format == "json" {
        let json = serde_json::json!({ "guide": agent_docs::AGENT_DOCS });
        println!("{}", serde_json::to_string_pretty(&json).unwrap());
    } else {
        print!("{}", agent_docs::AGENT_DOCS);
    }
}

fn run_skeleton(path: &PathBuf, format: &str, lang: &Option<Vec<String>>, no_progress: bool) {
    let (root, file_nodes) = build_file_nodes(path, lang, no_progress);
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

fn run_orient(path: &PathBuf, format: &str, lang: &Option<Vec<String>>, no_progress: bool) {
    use kgr_core::types::ImportKind;
    use std::collections::{HashMap, HashSet};

    let (root, mut file_nodes) = build_file_nodes(path, lang, no_progress);
    if file_nodes.is_empty() {
        return;
    }

    let resolver = Resolver::new(PathBuf::new(), &file_nodes);
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
                .map(|i| i.raw.as_str())
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

        let json = serde_json::json!({
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
    }
}

fn run_impact(
    name: &str,
    path: &PathBuf,
    format: &str,
    lang: &Option<Vec<String>>,
    depth: Option<usize>,
    no_progress: bool,
) {
    let (_root, mut file_nodes) = build_file_nodes(path, lang, no_progress);

    let resolver = Resolver::new(PathBuf::new(), &file_nodes);
    resolver.resolve_all(&mut file_nodes);

    let kgraph = KGraph::from_files(&file_nodes);

    // Find which file(s) define the named symbol
    let mut defining_file = None;
    let mut defining_symbol = None;
    for f in &file_nodes {
        for s in &f.symbols {
            if s.name == name {
                defining_file = Some(f.path.clone());
                defining_symbol = Some(s.clone());
                break;
            }
        }
        if defining_file.is_some() {
            break;
        }
    }

    let (defining_file, defining_symbol) = match (defining_file, defining_symbol) {
        (Some(f), Some(s)) => (f, s),
        _ => {
            if format == "json" {
                let result = serde_json::json!({
                    "symbol": name,
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
    };

    // Get transitive dependents with depth
    let dependents = kgraph.transitive_dependents_with_depth(&defining_file, depth);

    // Cross-reference: for each dependent, check if it calls the symbol
    let calls_symbol: std::collections::HashMap<PathBuf, bool> = dependents
        .iter()
        .map(|(dep_path, _)| {
            let has_call = file_nodes.iter().any(|f| {
                f.path == *dep_path
                    && f.calls.iter().any(|c| {
                        c.callee_raw == name || c.callee_raw.ends_with(&format!(".{name}"))
                    })
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
            "defined_in": {
                "file": defining_file.to_string_lossy(),
                "line": defining_symbol.span.start_line,
                "kind": defining_symbol.kind.to_string(),
            },
            "impact": impact,
        });
        serde_json::to_writer_pretty(&mut stdout, &result).ok();
        writeln!(stdout).ok();
    } else {
        writeln!(
            stdout,
            "Symbol: {name}\nDefined in: {}:{} ({})",
            defining_file.display(),
            defining_symbol.span.start_line,
            defining_symbol.kind,
        )
        .ok();
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
    path: &PathBuf,
    format: &str,
    lang: &Option<Vec<String>>,
    top: Option<usize>,
    no_progress: bool,
) {
    use kgr_core::types::SymbolKind;

    let (root, file_nodes) = build_file_nodes(path, lang, no_progress);
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

    entries.sort_by(|a, b| b.score.cmp(&a.score));
    entries.truncate(limit);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match format {
        "json" => {
            serde_json::to_writer_pretty(&mut out, &entries).unwrap();
            writeln!(out).unwrap();
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
                .unwrap();
            }
        }
        _ => {
            // table (default)
            writeln!(
                out,
                "{:<55} {:>9}  {:>7}  {:>7}  {:>5}",
                "FILE", "FUNCTIONS", "AVG_LEN", "MAX_LEN", "SCORE"
            )
            .unwrap();
            writeln!(out, "{}", "-".repeat(89)).unwrap();
            for entry in &entries {
                writeln!(
                    out,
                    "{:<55} {:>9}  {:>7}  {:>7}  {:>5}",
                    entry.file, entry.functions, entry.avg_length, entry.max_length, entry.score,
                )
                .unwrap();
            }
        }
    }
}

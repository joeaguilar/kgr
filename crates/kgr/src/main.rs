mod baseline;
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

        /// Filter by language (py, ts, js)
        #[arg(short, long)]
        lang: Option<Vec<String>>,

        /// Hide external dependencies
        #[arg(long)]
        no_external: bool,

        /// Disable progress bar
        #[arg(long)]
        no_progress: bool,

        /// Write output to file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Increase verbosity
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },

    /// Check for dependency issues (cycles, orphans, rule violations)
    Check {
        /// Root directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,

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

    /// Generate a .kgr.toml configuration file
    Init {
        /// Directory to initialize
        #[arg(default_value = ".")]
        path: PathBuf,
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
            no_progress,
            output,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_graph(
                &path,
                &format,
                &lang,
                no_external,
                no_progress,
                output.as_deref(),
            );
        }
        Some(Commands::Check {
            path,
            lang,
            no_progress,
            update_baseline,
            baseline,
            verbose,
        }) => {
            setup_tracing(verbose);
            run_check(
                &path,
                &lang,
                no_progress,
                update_baseline,
                baseline.as_deref(),
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
        Some(Commands::Init { path }) => {
            run_init(&path);
        }
        None => {
            // Default: run graph with tree format on current directory
            setup_tracing(0);
            run_graph(&PathBuf::from("."), "tree", &None, false, false, None);
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

fn run_graph(
    path: &PathBuf,
    format: &str,
    lang: &Option<Vec<String>>,
    no_external: bool,
    no_progress: bool,
    output: Option<&std::path::Path>,
) {
    let root = std::fs::canonicalize(path).unwrap_or_else(|e| {
        eprintln!("Error: cannot access '{}': {}", path.display(), e);
        process::exit(2);
    });

    let registry = ParserRegistry::new();
    let files = walk::discover(&root, lang);

    if files.is_empty() {
        eprintln!("No supported source files found in {}", root.display());
        return;
    }

    tracing::info!("Discovered {} files", files.len());

    let mut file_nodes = pipeline::parse_all(&root, files, &registry, !no_progress);
    let resolver = Resolver::new(PathBuf::new(), &file_nodes);
    resolver.resolve_all(&mut file_nodes);

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

    render::render(&dep_graph, &kgraph, format, no_external, &mut writer).unwrap_or_else(|e| {
        eprintln!("Error rendering output: {}", e);
        process::exit(2);
    });
}

fn run_check(
    path: &PathBuf,
    lang: &Option<Vec<String>>,
    no_progress: bool,
    update_baseline: bool,
    baseline_path: Option<&Path>,
) {
    let root = std::fs::canonicalize(path).unwrap_or_else(|e| {
        eprintln!("Error: cannot access '{}': {}", path.display(), e);
        process::exit(2);
    });

    let cfg = config::load_config(&root);
    let registry = ParserRegistry::new();
    let files = walk::discover(&root, lang);

    if files.is_empty() {
        eprintln!("No supported source files found in {}", root.display());
        return;
    }

    let mut file_nodes = pipeline::parse_all(&root, files, &registry, !no_progress);
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

    let mut has_errors = false;

    // Report cycles
    if !active_cycles.is_empty() {
        has_errors = true;
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
                has_errors = true;
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

    if suppressed > 0 {
        eprintln!("note: {} violation(s) suppressed by baseline", suppressed);
    }

    if has_errors {
        process::exit(1);
    } else {
        eprintln!("All checks passed.");
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

    let registry = ParserRegistry::new();
    let files = walk::discover(&root, lang);

    if files.is_empty() {
        eprintln!("No supported source files found in {}", root.display());
        return;
    }

    let mut file_nodes = pipeline::parse_all(&root, files, &registry, !no_progress);
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

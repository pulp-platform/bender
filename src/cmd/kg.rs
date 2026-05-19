// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! The `kg` subcommand: build, query, and serve the local knowledge graph.
//!
//! Reuses the existing Bender source resolution (the same path the `script`
//! and `sources` subcommands use), drives `bender-slang` to parse and walk
//! the design, persists the result into a single Grafeo file (graph +
//! HNSW vectors + BM25 text index), and exposes both a typed CLI and an
//! MCP stdio adapter.

#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use indexmap::{IndexMap, IndexSet};
use miette::{Context as _, IntoDiagnostic as _};
use owo_colors::{OwoColorize, Stream};
use tokio::runtime::Runtime;

use bender_kg_core::{CoreConfig, Engine};
use bender_kg_extract::{ExtractInputs, SourceGroupInput};

use crate::Result;
use crate::cmd::sources::get_passed_targets;
use crate::config::{Validate, ValidationContext};
use crate::sess::{Session, SessionIo};
use crate::src::{SourceFile, SourceType};
use crate::target::TargetSet;

#[derive(Args, Debug)]
pub struct KgArgs {
    #[command(subcommand)]
    pub command: KgCommands,
}

#[derive(Subcommand, Debug)]
pub enum KgCommands {
    /// Build the knowledge graph end-to-end (extract -> index).
    Build(BuildArgs),
    /// Run extraction only and write `kg.v3` IR to a JSONL file.
    Parse(ParseArgs),
    /// Index a previously-produced IR JSONL into the local graph store.
    Index(IndexArgs),
    /// Query the graph; mirrors the MCP tool surface.
    Query(QueryArgs),
    /// Drop a single design's data (or everything with `--all`).
    Clear(ClearArgs),
    /// Print summary statistics.
    Stats(StatsArgs),
    /// Run the stdio MCP server.
    Mcp(McpArgs),
}

/// Base arguments shared by all kg subcommands.
#[derive(Args, Debug, Clone)]
pub struct BaseKgArgs {
    /// Root directory for kg artifacts (defaults to `<workspace>/.bender-kg`).
    #[arg(long, env = "BENDER_KG_ROOT")]
    pub root: Option<PathBuf>,
    /// Output format (tree for human-readable, json for scripts/LLM).
    #[arg(long, value_enum, default_value_t = OutputFormat::Tree)]
    pub format: OutputFormat,
}

/// Build configuration arguments for commands that build/index the knowledge graph.
#[derive(Args, Debug, Clone)]
pub struct BuildConfigArgs {
    /// Skip the embedding step (faster builds, but disables `kg search`).
    #[arg(long)]
    pub no_embed: bool,
    /// Embedding dimensionality used by the deterministic-fallback embedder.
    #[arg(long, default_value_t = bender_kg_similarity::DEFAULT_DIM as u64)]
    pub embed_dim: u64,
    /// Maximum rows per UNWIND batch when upserting modules and edges into
    /// the Grafeo store. Larger = fewer Cypher round-trips, more memory
    /// per call. Default 4096 is tuned for ~1.7k-module designs; set
    /// lower on memory-tight hosts.
    #[arg(long, default_value_t = 4096, value_name = "N")]
    pub upsert_chunk_size: u32,
    /// Disable the parallel pipeline that overlaps slang's `walk_elaborated`
    /// with the base graph upsert when `--elab` is on. Falls back to running
    /// the two phases sequentially. Mostly useful for debugging; the parallel
    /// path is correctness-equivalent.
    #[arg(long)]
    pub no_pipeline_elab: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ResolutionArgs {
    /// Select specific target from Bender.yml (repeatable).
    #[arg(short, long, action = clap::ArgAction::Append)]
    pub target: Vec<String>,
    /// Select specific package (repeatable).
    #[arg(short = 'p', long, action = clap::ArgAction::Append)]
    pub package: Vec<String>,
    /// Exclude package from dependency resolution (repeatable).
    #[arg(long, action = clap::ArgAction::Append)]
    pub exclude: Vec<String>,
    /// Don't include dependencies, only direct sources.
    #[arg(long)]
    pub no_deps: bool,
    /// Include directory for SystemVerilog `include directives.
    #[arg(short = 'I', action = clap::ArgAction::Append)]
    pub include_dir: Vec<String>,
    /// Define macro for SystemVerilog preprocessing.
    #[arg(short = 'D', action = clap::ArgAction::Append)]
    pub define: Vec<String>,
    /// One or more elaboration roots. REQUIRED for `kg build` / `kg parse`
    /// / `kg index`. The graph is pruned to only the syntax trees reachable
    /// from these tops (via slang's symbol-reference graph) before the
    /// downstream walk, so the resulting graph captures exactly the modules
    /// used by the design. Repeatable; pass once per root.
    #[arg(
        long = "top",
        action = clap::ArgAction::Append,
        value_name = "MODULE",
        required = true
    )]
    pub top: Vec<String>,
    /// Run slang's elaboration pass from `--top` and enrich
    /// `InstantiationInfo` with `resolved_param_values` and
    /// `resolved_port_widths`. Off by default (skips a costly Compilation
    /// build). Orthogonal to `--top`: pruning still happens regardless.
    #[arg(long)]
    pub elab: bool,
    /// Design identifier for multi-design workspaces.
    #[arg(long)]
    pub design: Option<String>,
    /// Treat all source groups as one slang compilation unit (vcs / `vlog
    /// -mfcu` semantics). `\`define`s declared in earlier groups become
    /// visible to later groups, which lets cross-package macro use parse
    /// without per-file `\`include`s. Off by default; enable when a build
    /// fails on `unknown macro or compiler directive` errors that point to
    /// macros defined in another Bender package.
    #[arg(long)]
    pub single_unit: bool,
    /// Best-effort parsing: report parse-time errors but don't abort the
    /// build. The indexer ingests whichever modules survived parsing.
    /// Useful for repos with encrypted vendor IP (`\`protect`), unsatisfied
    /// `\`include`s, or other hostile inputs that still admit a partial
    /// graph. Off by default (strict).
    #[arg(long, alias = "keep-going")]
    pub lenient: bool,
}

#[derive(Args, Debug)]
pub struct BuildArgs {
    #[command(flatten)]
    pub base: BaseKgArgs,
    #[command(flatten)]
    pub build_config: BuildConfigArgs,
    #[command(flatten)]
    pub res: ResolutionArgs,
}

#[derive(Args, Debug)]
pub struct ParseArgs {
    #[command(flatten)]
    pub base: BaseKgArgs,
    /// Skip the embedding step (faster builds, but disables `kg search`).
    #[arg(long)]
    pub no_embed: bool,
    /// Embedding dimensionality used by the deterministic-fallback embedder.
    #[arg(long, default_value_t = bender_kg_similarity::DEFAULT_DIM as u64)]
    pub embed_dim: u64,
    /// Disable the parallel pipeline that overlaps slang's `walk_elaborated`
    /// with the base graph upsert when `--elab` is on.
    #[arg(long)]
    pub no_pipeline_elab: bool,
    #[command(flatten)]
    pub res: ResolutionArgs,
    /// Output file path for extracted JSONL.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct IndexArgs {
    #[command(flatten)]
    pub base: BaseKgArgs,
    /// Skip the embedding step (faster builds, but disables `kg search`).
    #[arg(long)]
    pub no_embed: bool,
    /// Embedding dimensionality used by the deterministic-fallback embedder.
    #[arg(long, default_value_t = bender_kg_similarity::DEFAULT_DIM as u64)]
    pub embed_dim: u64,
    /// Maximum rows per UNWIND batch when upserting modules and edges.
    #[arg(long, default_value_t = 4096, value_name = "N")]
    pub upsert_chunk_size: u32,
    /// Input JSONL file path to index.
    #[arg(short, long)]
    pub input: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ClearArgs {
    #[command(flatten)]
    pub base: BaseKgArgs,
    /// Design to clear from graph.
    #[arg(long)]
    pub design: Option<String>,
    /// Clear all designs from graph.
    #[arg(long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct StatsArgs {
    #[command(flatten)]
    pub base: BaseKgArgs,
    /// Show statistics for specific design.
    #[arg(long)]
    pub design: Option<String>,
}

#[derive(Args, Debug)]
pub struct McpArgs {
    #[command(flatten)]
    pub base: BaseKgArgs,
}

#[derive(Args, Debug)]
pub struct QueryArgs {
    #[command(flatten)]
    pub base: BaseKgArgs,
    #[command(subcommand)]
    pub op: QueryOp,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Compact single-line JSON; ideal for piping into `jq` or LLM consumption.
    Json,
    /// Human-friendly tree format (default). For pretty JSON, pipe json format to `jq`.
    Tree,
}

#[derive(Subcommand, Debug)]
pub enum QueryOp {
    SearchModules {
        query: String,
        #[arg(long, default_value_t = 15)]
        top_k: usize,
        #[arg(long)]
        design: Option<String>,
    },
    GetModule {
        name: String,
    },
    GetSubgraph {
        name: String,
        #[arg(long, default_value_t = 3)]
        depth: i32,
    },
    GetInstanceContext {
        parent: String,
        child: String,
    },
    GetParents {
        name: String,
    },
    GetChildren {
        name: String,
    },
    GetPorts {
        name: String,
    },
    FindByProtocol {
        protocol: String,
        #[arg(long)]
        design: Option<String>,
    },
    GetSourceSnippet {
        module_name: String,
        #[arg(long, default_value = "module")]
        element: String,
        #[arg(long, default_value = "")]
        instance_name: String,
    },
    TraceHierarchyPath {
        from_module: String,
        to_module: String,
    },
    CheckConnectivity {
        module_name: String,
        #[arg(long, default_value_t = 1)]
        depth: i32,
    },
    TraceParameter {
        module_name: String,
        param_name: String,
        /// Follow parameter propagation recursively through the hierarchy.
        #[arg(long)]
        recursive: bool,
        /// Maximum recursion depth (only active with --recursive).
        #[arg(long, default_value_t = 5, value_name = "N")]
        depth: i32,
    },
    TraceSignal {
        module_name: String,
        signal_name: String,
        /// Follow signal connections recursively through the hierarchy.
        #[arg(long)]
        recursive: bool,
        /// Maximum recursion depth (only active with --recursive).
        #[arg(long, default_value_t = 5, value_name = "N")]
        depth: i32,
    },
    MatchInterfaces {
        module_a: String,
        module_b: String,
        #[arg(long, default_value = "")]
        prefix_a: String,
        #[arg(long, default_value = "")]
        prefix_b: String,
    },
    FindStructurallySimilar {
        module_name: String,
        #[arg(long, default_value_t = 0.3)]
        min_overlap: f64,
        #[arg(long)]
        design: Option<String>,
    },
}

pub fn run(sess: &Session, args: KgArgs) -> Result<()> {
    let workspace = sess.root.to_path_buf();
    match args.command {
        KgCommands::Build(a) => run_build(sess, &workspace, a),
        KgCommands::Parse(a) => run_parse(sess, &workspace, a),
        KgCommands::Index(a) => run_index(&workspace, a),
        KgCommands::Query(a) => run_query(&workspace, a),
        KgCommands::Clear(a) => run_clear(&workspace, a),
        KgCommands::Stats(a) => run_stats(&workspace, a),
        KgCommands::Mcp(a) => run_mcp(&workspace, a),
    }
}


fn rt() -> Result<Runtime> {
    Runtime::new().into_diagnostic().wrap_err("tokio runtime")
}

fn open_engine(rt: &Runtime, cfg: CoreConfig) -> Result<Engine> {
    rt.block_on(Engine::open(cfg))
        .into_diagnostic()
        .wrap_err("open kg engine")
}

fn resolve_inputs(
    sess: &Session,
    workspace: &Path,
    res: &ResolutionArgs,
    rt: &Runtime,
) -> Result<ExtractInputs> {
    let io = SessionIo::new(sess);
    let srcs = rt.block_on(io.sources(false, &[]))?;

    let targets = TargetSet::new(res.target.iter().map(|s| s.as_str()));
    let package_set: IndexSet<String> = IndexSet::from_iter(res.package.iter().cloned());
    let exclude_set: IndexSet<String> = IndexSet::from_iter(res.exclude.iter().cloned());

    let packages = &srcs.get_package_list(
        sess.manifest.package.name.to_string(),
        &package_set,
        &exclude_set,
        res.no_deps,
    );

    let (targets, packages) = get_passed_targets(sess, rt, &io, &targets, packages, &package_set)?;

    let srcs = srcs
        .filter_targets(&targets)
        .unwrap_or_default()
        .filter_packages(&packages)
        .unwrap_or_default();

    let srcs_flat = srcs
        .flatten()
        .into_iter()
        .map(|f| f.validate(&ValidationContext::default()))
        .collect::<Result<Vec<_>>>()?;

    let active_targets: Vec<String> = res.target.iter().cloned().collect();
    let target_defs = bender_kg_extract::target_defines(&active_targets);

    let mut groups: IndexMap<String, SourceGroupInput> = IndexMap::new();
    for grp in srcs_flat {
        let key = grp.package.unwrap_or("").to_string();
        let entry = groups.entry(key).or_insert_with(|| {
            let mut defs: Vec<String> = target_defs.clone();
            defs.extend(res.define.iter().cloned());
            SourceGroupInput {
                files: Vec::new(),
                include_dirs: res.include_dir.clone(),
                defines: defs,
            }
        });
        for src in &grp.files {
            if let SourceFile::File(p, Some(SourceType::Verilog)) = src {
                entry.files.push(p.to_string_lossy().into_owned());
            }
        }
        for (_, p) in grp
            .include_dirs
            .iter()
            .chain(grp.export_incdirs.values().flatten())
        {
            let s = p.to_string_lossy().into_owned();
            if !entry.include_dirs.contains(&s) {
                entry.include_dirs.push(s);
            }
        }
        for (name, (_, val)) in &grp.defines {
            let entry_def = match val {
                Some(v) => format!("{name}={v}"),
                None => name.to_string(),
            };
            if !entry.defines.contains(&entry_def) {
                entry.defines.push(entry_def);
            }
        }
    }
    let groups: Vec<SourceGroupInput> = groups
        .into_values()
        .filter(|g| !g.files.is_empty())
        .collect();

    let tops: Vec<String> = res.top.iter().cloned().collect();

    Ok(ExtractInputs {
        workspace: workspace.to_string_lossy().into_owned(),
        targets: active_targets,
        tops,
        elab: res.elab,
        design_alias: res.design.clone(),
        groups,
        single_unit: res.single_unit,
        lenient: res.lenient,
        parse_jobs: 1,
    })
}

fn run_build(sess: &Session, workspace: &Path, args: BuildArgs) -> Result<()> {
    let rt = rt()?;
    let root = args.base.root.clone().unwrap_or_else(|| workspace.join(".bender-kg"));
    let mut cfg = CoreConfig::new(root);
    cfg.embed.dim = args.build_config.embed_dim as usize;
    cfg.skip_embeddings = args.build_config.no_embed;
    cfg.upsert_chunk_size = args.build_config.upsert_chunk_size.max(1) as usize;
    cfg.pipeline_elab = !args.build_config.no_pipeline_elab;

    let inputs = resolve_inputs(sess, workspace, &args.res, &rt)?;
    let mut engine = open_engine(&rt, cfg)?;
    let outcome = rt
        .block_on(engine.build(&inputs))
        .into_diagnostic()
        .wrap_err("kg build")?;
    let summary = serde_json::json!({
        "design": outcome.manifest.identity.alias,
        "id": outcome.manifest.identity.id,
        "modules": outcome.modules_indexed,
        "embeddings": outcome.embeddings_indexed,
        "ir_path": engine.config().ir_path(),
        "manifest_path": engine.config().manifest_path(),
        "db_path": engine.store().db_path().ok(),
        "phases_seconds": outcome.phases,
    });
    emit(&summary, args.base.format, None)
}

fn run_parse(sess: &Session, workspace: &Path, args: ParseArgs) -> Result<()> {
    let rt = rt()?;
    let root = args.base.root.clone().unwrap_or_else(|| workspace.join(".bender-kg"));
    let mut cfg = CoreConfig::new(root);
    cfg.embed.dim = args.embed_dim as usize;
    cfg.skip_embeddings = args.no_embed;
    cfg.pipeline_elab = !args.no_pipeline_elab;

    let inputs = resolve_inputs(sess, workspace, &args.res, &rt)?;
    let path = args.output.unwrap_or_else(|| cfg.ir_path());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).into_diagnostic()?;
    }
    let manifest = bender_kg_extract::extract_to_jsonl(&inputs, &path)
        .into_diagnostic()
        .wrap_err("kg parse")?;
    emit(
        &serde_json::json!({
            "design": manifest.identity.alias,
            "ir_path": path,
            "modules": manifest.module_count,
            "edges": manifest.edge_count,
            "warnings": manifest.extraction_warnings,
        }),
        args.base.format,
        None,
    )
}

fn run_index(workspace: &Path, args: IndexArgs) -> Result<()> {
    let rt = rt()?;
    let root = args.base.root.clone().unwrap_or_else(|| workspace.join(".bender-kg"));
    let mut cfg = CoreConfig::new(root);
    cfg.embed.dim = args.embed_dim as usize;
    cfg.skip_embeddings = args.no_embed;
    cfg.upsert_chunk_size = args.upsert_chunk_size.max(1) as usize;

    let path = args.input.unwrap_or_else(|| cfg.ir_path());
    let mut engine = open_engine(&rt, cfg)?;
    let count = rt
        .block_on(engine.index_from_jsonl(&path))
        .into_diagnostic()
        .wrap_err("kg index")?;
    emit(
        &serde_json::json!({"indexed": count, "from": path}),
        args.base.format,
        None,
    )
}

fn run_clear(workspace: &Path, args: ClearArgs) -> Result<()> {
    let rt = rt()?;
    let root = args.base.root.clone().unwrap_or_else(|| workspace.join(".bender-kg"));
    let cfg = CoreConfig::new(root);
    let mut engine = open_engine(&rt, cfg)?;
    let value = if args.all {
        rt.block_on(engine.clear_all()).into_diagnostic()?;
        serde_json::json!({"cleared": "all"})
    } else {
        let alias = args
            .design
            .ok_or_else(|| miette::miette!("--design <alias> is required (or pass --all)"))?;
        rt.block_on(engine.clear_design(&alias)).into_diagnostic()?;
        serde_json::json!({"cleared_design": alias})
    };
    emit(&value, args.base.format, None)
}

fn run_stats(workspace: &Path, args: StatsArgs) -> Result<()> {
    let rt = rt()?;
    let root = args.base.root.clone().unwrap_or_else(|| workspace.join(".bender-kg"));
    let cfg = CoreConfig::new(root);
    let engine = open_engine(&rt, cfg)?;
    let stats = engine
        .stats(args.design.as_deref())
        .into_diagnostic()
        .wrap_err("stats")?;
    emit(
        &serde_json::to_value(stats).into_diagnostic()?,
        args.base.format,
        None,
    )
}

fn run_mcp(workspace: &Path, args: McpArgs) -> Result<()> {
    let rt = rt()?;
    let root = args.base.root.clone().unwrap_or_else(|| workspace.join(".bender-kg"));
    let cfg = CoreConfig::new(root);
    rt.block_on(bender_kg_mcp::serve_stdio(cfg))
        .map_err(|e| miette::miette!("kg mcp: {e}"))?;
    Ok(())
}

fn run_query(workspace: &Path, args: QueryArgs) -> Result<()> {
    let rt = rt()?;
    let root = args.base.root.clone().unwrap_or_else(|| workspace.join(".bender-kg"));
    let cfg = CoreConfig::new(root);
    let engine = open_engine(&rt, cfg)?;
    let value = dispatch_query(&rt, &engine, &args.op)?;
    emit(&value, args.base.format, Some(&args.op))
}

fn dispatch_query(
    rt: &tokio::runtime::Runtime,
    engine: &Engine,
    op: &QueryOp,
) -> Result<serde_json::Value> {
    let v = match op {
        QueryOp::SearchModules { query, top_k, design } => {
            let hits = rt
                .block_on(engine.search_modules(query, *top_k, design.as_deref().filter(|s| !s.is_empty())))
                .into_diagnostic()?;
            serde_json::to_value(hits).into_diagnostic()?
        }
        QueryOp::GetModule { name } => match engine.get_module(name).into_diagnostic()? {
            Some(m) => serde_json::to_value(m).into_diagnostic()?,
            None => serde_json::json!({"error": format!("Module '{name}' not found")}),
        },
        QueryOp::GetSubgraph { name, depth } => {
            let sg = engine.get_subgraph(name, *depth).into_diagnostic()?;
            serde_json::json!({ "root": name, "nodes": sg.nodes, "edges": sg.edges })
        }
        QueryOp::GetInstanceContext { parent, child } => {
            serde_json::to_value(engine.get_instance_context(parent, child).into_diagnostic()?)
                .into_diagnostic()?
        }
        QueryOp::GetParents { name } => {
            serde_json::to_value(engine.get_parents(name).into_diagnostic()?).into_diagnostic()?
        }
        QueryOp::GetChildren { name } => {
            serde_json::to_value(engine.get_children(name).into_diagnostic()?).into_diagnostic()?
        }
        QueryOp::GetPorts { name } => match engine.get_module(name).into_diagnostic()? {
            Some(m) => serde_json::to_value(m.ports).into_diagnostic()?,
            None => serde_json::json!({"error": format!("Module '{name}' not found")}),
        },
        QueryOp::FindByProtocol { protocol, design } => {
            serde_json::to_value(
                engine.find_by_protocol(protocol, design.as_deref().filter(|s| !s.is_empty()))
                    .into_diagnostic()?,
            )
            .into_diagnostic()?
        }
        QueryOp::GetSourceSnippet { module_name, element, instance_name } => {
            engine.get_source_snippet(module_name, element, instance_name).into_diagnostic()?
        }
        QueryOp::TraceHierarchyPath { from_module, to_module } => {
            serde_json::to_value(engine.trace_hierarchy_path(from_module, to_module).into_diagnostic()?)
                .into_diagnostic()?
        }
        QueryOp::CheckConnectivity { module_name, depth } => {
            let findings = engine.check_connectivity(module_name, *depth).into_diagnostic()?;
            serde_json::json!({
                "module": module_name, "depth": depth,
                "issue_count": findings.len(), "findings": findings,
            })
        }
        QueryOp::TraceParameter { module_name, param_name, recursive, depth } => {
            if *recursive {
                let res = engine.trace_parameter_recursive(module_name, param_name, *depth).into_diagnostic()?;
                serde_json::json!({
                    "module": module_name, "parameter": param_name,
                    "recursive": true, "max_depth": depth,
                    "affected_instances": res.len(), "instances": res,
                })
            } else {
                let res = engine.trace_parameter(module_name, param_name).into_diagnostic()?;
                serde_json::json!({
                    "module": module_name, "parameter": param_name,
                    "affected_instances": res.len(), "instances": res,
                })
            }
        }
        QueryOp::TraceSignal { module_name, signal_name, recursive, depth } => {
            if *recursive {
                let res = engine.trace_signal_recursive(module_name, signal_name, *depth).into_diagnostic()?;
                serde_json::json!({
                    "module": module_name, "signal": signal_name,
                    "recursive": true, "max_depth": depth,
                    "connections": res.len(), "instances": res,
                })
            } else {
                let res = engine.trace_signal(module_name, signal_name).into_diagnostic()?;
                serde_json::json!({
                    "module": module_name, "signal": signal_name,
                    "connections": res.len(), "instances": res,
                })
            }
        }
        QueryOp::MatchInterfaces { module_a, module_b, prefix_a, prefix_b } => {
            engine.match_interfaces(module_a, module_b, prefix_a, prefix_b).into_diagnostic()?
        }
        QueryOp::FindStructurallySimilar { module_name, min_overlap, design } => {
            let res = engine
                .find_structurally_similar(module_name, *min_overlap, design.as_deref().filter(|s| !s.is_empty()))
                .into_diagnostic()?;
            serde_json::json!({"module": module_name, "candidates": res})
        }
    };
    Ok(v)
}

/// Single output dispatcher used by every `kg` subcommand. `op` is `Some`
/// only for `kg query` calls; for the summary outputs of the other
/// subcommands, `Tree` collapses to `Pretty`.
fn emit(value: &serde_json::Value, format: OutputFormat, op: Option<&QueryOp>) -> Result<()> {
    let s = match format {
        OutputFormat::Json => serde_json::to_string(value).into_diagnostic()?,
        OutputFormat::Tree => op
            .and_then(|op| format_tree(value, op))
            .map(|s| {
                if s.ends_with('\n') {
                    s
                } else {
                    format!("{s}\n")
                }
            })
            .unwrap_or_else(|| {
                // Fallback to pretty JSON for commands without tree renderer
                serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
            }),
    };
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(s.as_bytes()).into_diagnostic()?;
    if !s.ends_with('\n') {
        handle.write_all(b"\n").into_diagnostic()?;
    }
    Ok(())
}

/// Render a query op's JSON output as a human-readable indented tree.
/// Every `QueryOp` variant has a dedicated renderer; the function still
/// returns `Option<String>` so callers can defensively fall back to JSON
/// if a future variant is added without a renderer.
fn format_tree(value: &serde_json::Value, op: &QueryOp) -> Option<String> {
    let mut out = String::new();
    match op {
        QueryOp::GetInstanceContext { .. } | QueryOp::TraceHierarchyPath { .. } => {
            render_edges(&mut out, value)
        }
        QueryOp::GetSubgraph { .. } => render_subgraph(&mut out, value),
        QueryOp::GetParents { .. } => render_module_list(&mut out, value, "(no parents)"),
        QueryOp::GetChildren { .. } => render_module_list(&mut out, value, "(no children)"),
        QueryOp::SearchModules { .. } => render_module_list(&mut out, value, "(no hits)"),
        QueryOp::FindByProtocol { .. } => {
            render_module_list(&mut out, value, "(no matching modules)")
        }
        QueryOp::FindStructurallySimilar { .. } => render_similar(&mut out, value),
        QueryOp::GetModule { .. } => render_module(&mut out, value),
        QueryOp::GetPorts { .. } => render_ports(&mut out, value),
        QueryOp::GetSourceSnippet { .. } => render_snippet(&mut out, value),
        QueryOp::TraceParameter { .. } => render_trace_parameter(&mut out, value),
        QueryOp::TraceSignal { .. } => render_trace_signal(&mut out, value),
        QueryOp::CheckConnectivity { .. } => render_check_connectivity(&mut out, value),
        QueryOp::MatchInterfaces { .. } => render_match_interfaces(&mut out, value),
    }
    Some(out)
}

struct Indent(usize);
impl fmt::Display for Indent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for _ in 0..self.0 {
            f.write_str("  ")?;
        }
        Ok(())
    }
}

fn location_string(file: &str, lstart: Option<i64>, lend: Option<i64>) -> String {
    match (file.is_empty(), lstart, lend) {
        (false, Some(s), Some(e)) => format!(" [{file}:{s}-{e}]"),
        (false, Some(s), None) => format!(" [{file}:{s}]"),
        (false, _, _) => format!(" [{file}]"),
        _ => String::new(),
    }
}

/// Convert absolute path to relative path for better readability.
/// Strips common prefixes like workspace root or /proj_soc paths.
fn relative_path(path: &str) -> &str {
    // Try to find a good cut point - look for common patterns
    if let Some(pos) = path.rfind("/hw/") {
        return &path[pos + 1..]; // Return "hw/smu/rtl/smu.sv"
    }
    if let Some(pos) = path.rfind("/src/") {
        return &path[pos + 1..];
    }
    // If no pattern found, return the filename portion at minimum
    path.rsplit('/').next().unwrap_or(path)
}

fn render_edges(out: &mut String, value: &serde_json::Value) {
    let arr = value.as_array().filter(|a| !a.is_empty());
    let Some(arr) = arr else {
        out.push_str("(no edges)\n");
        return;
    };
    for edge in arr {
        render_edge(out, edge, 0);
    }
}

fn render_subgraph(out: &mut String, value: &serde_json::Value) {
    let root = value.get("root").and_then(|v| v.as_str()).unwrap_or("?");
    let node_count = value.get("nodes").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let edges = value.get("edges").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let edge_count = edges.len();

    let _ = writeln!(out, "{}  {} module(s), {} instantiation(s)\n",
        root.if_supports_color(Stream::Stdout, |t| t.yellow()),
        node_count, edge_count);

    // Build adjacency map: parent -> Vec<edge>
    let mut adj: BTreeMap<String, Vec<&serde_json::Value>> = BTreeMap::new();
    for edge in &edges {
        let parent = edge.get("parent").and_then(|v| v.as_str()).unwrap_or("");
        adj.entry(parent.to_string()).or_default().push(edge);
    }

    let mut on_path = std::collections::HashSet::new();
    render_subgraph_nodes(out, root, &adj, "", &mut on_path);
}

fn render_subgraph_nodes(
    out: &mut String,
    module: &str,
    adj: &BTreeMap<String, Vec<&serde_json::Value>>,
    prefix: &str,
    on_path: &mut std::collections::HashSet<String>,
) {
    let children = match adj.get(module) {
        Some(c) if !c.is_empty() => c,
        _ => return,
    };
    let total = children.len();
    on_path.insert(module.to_string());
    for (idx, edge) in children.iter().enumerate() {
        let is_last   = idx + 1 == total;
        let box_char  = if is_last { "└─" } else { "├─" };
        let child_pfx = if is_last { format!("{}    ", prefix) } else { format!("{}│   ", prefix) };
        let child_mod = edge.get("child").and_then(|v| v.as_str()).unwrap_or("?");
        let inst_name = edge.get("instance_name").and_then(|v| v.as_str()).unwrap_or("?");
        let file      = edge.get("parent_file_path").and_then(|v| v.as_str()).unwrap_or("");
        let lstart    = edge.get("line_start").and_then(|v| v.as_i64());
        let lend      = edge.get("line_end").and_then(|v| v.as_i64());
        let relpath   = relative_path(file);
        let loc = match (lstart, lend) {
            (Some(s), Some(e)) => format!("  {relpath}:{s}-{e}"),
            (Some(s), None)    => format!("  {relpath}:{s}"),
            _ if !file.is_empty() => format!("  {relpath}"),
            _                  => String::new(),
        };
        let cycle = on_path.contains(child_mod);
        let cycle_mark = if cycle { "  (↑ cycle)" } else { "" };

        let _ = writeln!(out, "{}{} {} → {}{}{}",
            prefix,
            box_char.if_supports_color(Stream::Stdout, |t| t.blue()),
            inst_name,
            child_mod.if_supports_color(Stream::Stdout, |t| t.yellow()),
            loc.if_supports_color(Stream::Stdout, |t| t.dimmed()),
            cycle_mark.if_supports_color(Stream::Stdout, |t| t.yellow()),
        );

        if !cycle {
            render_subgraph_nodes(out, child_mod, adj, &child_pfx, on_path);
        }
    }
    on_path.remove(module);
}

/// Shared module-list renderer: one line per module, pulling in only the
/// optional fields that exist in the JSON payload. Used by `GetParents`,
/// `SearchModules`, `FindByProtocol`, and (via `render_similar`) the
/// candidate list of `FindStructurallySimilar`.
fn render_module_list(out: &mut String, value: &serde_json::Value, empty_msg: &str) {
    let arr = value.as_array().filter(|a| !a.is_empty());
    let Some(arr) = arr else {
        let _ = writeln!(out, "{empty_msg}");
        return;
    };
    for m in arr {
        render_module_list_item(out, m);
    }
}

fn render_module_list_item(out: &mut String, m: &serde_json::Value) {
    let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("?");
    let design = m.get("design").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let score = m.get("score").and_then(|v| v.as_f64());
    let shared = m.get("shared_ports").and_then(|v| v.as_i64());
    let file = m.get("file_path").and_then(|v| v.as_str()).filter(|s| !s.is_empty());

    let mut parts = vec![name.if_supports_color(Stream::Stdout, |t| t.yellow()).to_string()];
    if let Some(d) = design {
        parts.push(format!("{}", format!("[{d}]").if_supports_color(Stream::Stdout, |t| t.dimmed())));
    }
    if let Some(s) = score {
        parts.push(format!("{}", format!("score={s:.3}").if_supports_color(Stream::Stdout, |t| t.green())));
    }
    if let Some(sp) = shared {
        parts.push(format!("shared_ports={sp}"));
    }
    if let Some(f) = file {
        parts.push(format!("{}", f.if_supports_color(Stream::Stdout, |t| t.dimmed())));
    }
    let _ = writeln!(out, "{}", parts.join(" "));
}

fn render_similar(out: &mut String, value: &serde_json::Value) {
    let module = value.get("module").and_then(|v| v.as_str()).unwrap_or("?");
    let candidates = value
        .get("candidates")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let _ = writeln!(out, "{module}: structurally-similar candidates");
    let arr = candidates.as_array().filter(|a| !a.is_empty());
    let Some(arr) = arr else {
        out.push_str("  (none)\n");
        return;
    };
    for m in arr {
        out.push_str("  ");
        render_module_list_item(out, m);
    }
}

fn render_module(out: &mut String, value: &serde_json::Value) {
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        let _ = writeln!(out, "{err}");
        return;
    }
    let name   = value.get("name").and_then(|v| v.as_str()).unwrap_or("?");
    let design = value.get("design").and_then(|v| v.as_str()).unwrap_or("?");
    let file   = value.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
    let lstart = value.get("line_start").and_then(|v| v.as_i64());
    let lend   = value.get("line_end").and_then(|v| v.as_i64());

    let relpath = relative_path(file);
    let loc = match (lstart, lend) {
        (Some(s), Some(e)) if !file.is_empty() => format!("  {relpath}:{s}-{e}"),
        _ if !file.is_empty()                  => format!("  {relpath}"),
        _                                       => String::new(),
    };

    let _ = writeln!(out, "{} {}{}",
        name.if_supports_color(Stream::Stdout, |t| t.yellow()),
        format!("[{design}]").if_supports_color(Stream::Stdout, |t| t.dimmed()),
        loc.if_supports_color(Stream::Stdout, |t| t.dimmed()),
    );

    if let Some(desc) = value.get("description").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        let _ = writeln!(out, "  description: {desc}");
    }

    if let Some(params) = value.get("parameters").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
        let _ = writeln!(out, "  {} ({}):",
            "parameters".if_supports_color(Stream::Stdout, |t| t.bold()),
            params.len());
        for p in params {
            let pname = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let kind  = p.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            let dv    = p.get("default_value").and_then(|v| v.as_str()).unwrap_or("");
            if dv.is_empty() {
                let _ = writeln!(out, "    {}: {}",
                    pname.if_supports_color(Stream::Stdout, |t| t.magenta()),
                    kind.if_supports_color(Stream::Stdout, |t| t.dimmed()));
            } else {
                let _ = writeln!(out, "    {}: {} = {}",
                    pname.if_supports_color(Stream::Stdout, |t| t.magenta()),
                    kind.if_supports_color(Stream::Stdout, |t| t.dimmed()),
                    dv.if_supports_color(Stream::Stdout, |t| t.green()));
            }
        }
    }

    if let Some(ports) = value.get("ports").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
        let _ = writeln!(out, "  {} ({}):",
            "ports".if_supports_color(Stream::Stdout, |t| t.bold()),
            ports.len());
        for p in ports {
            render_port(out, p, 2);
        }
    }

    if let Some(insts) = value.get("instantiations").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
        let _ = writeln!(out, "  {} ({}):",
            "instantiations".if_supports_color(Stream::Stdout, |t| t.bold()),
            insts.len());
        for inst in insts {
            let mn  = inst.get("module_name").and_then(|v| v.as_str()).unwrap_or("?");
            let inm = inst.get("instance_name").and_then(|v| v.as_str()).unwrap_or("?");
            let _ = writeln!(out, "    {} ({})",
                mn.if_supports_color(Stream::Stdout, |t| t.yellow()),
                inm.if_supports_color(Stream::Stdout, |t| t.dimmed()));
        }
    }

    if let Some(imps) = value.get("imports").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
        let names: Vec<&str> = imps
            .iter()
            .filter_map(|i| i.get("package_name").and_then(|v| v.as_str()))
            .collect();
        let _ = writeln!(out, "  {} ({}): {}",
            "imports".if_supports_color(Stream::Stdout, |t| t.bold()),
            names.len(),
            names.join(", ").if_supports_color(Stream::Stdout, |t| t.dimmed()));
    }
}

fn render_ports(out: &mut String, value: &serde_json::Value) {
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        let _ = writeln!(out, "{err}");
        return;
    }
    let Some(arr) = value.as_array().filter(|a| !a.is_empty()) else {
        out.push_str("(no ports)\n");
        return;
    };
    for p in arr {
        render_port(out, p, 0);
    }
}

fn render_port(out: &mut String, p: &serde_json::Value, depth: usize) {
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
    let dir  = p.get("direction").and_then(|v| v.as_str()).unwrap_or("?");
    let bw   = p.get("bit_width").and_then(|v| v.as_i64());
    let we   = p.get("width_expr").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let ts   = p.get("type_str").and_then(|v| v.as_str()).filter(|s| !s.is_empty());

    let width_s = match (bw, we) {
        (Some(b), _)    => format!("   width={b}"),
        (None, Some(e)) => format!("   width={e}"),
        _               => String::new(),
    };
    let type_s = ts.map(|t| format!("   type={t}")).unwrap_or_default();
    let dir_colored = match dir {
        "input"  => format!("{}", dir.if_supports_color(Stream::Stdout, |t| t.green())),
        "output" => format!("{}", dir.if_supports_color(Stream::Stdout, |t| t.bright_green())),
        "inout"  => format!("{}", dir.if_supports_color(Stream::Stdout, |t| t.blue())),
        _        => dir.to_string(),
    };
    let _ = writeln!(out, "{}{} ({}){}{}",
        Indent(depth),
        name.if_supports_color(Stream::Stdout, |t| t.cyan()),
        dir_colored,
        width_s.if_supports_color(Stream::Stdout, |t| t.dimmed()),
        type_s.if_supports_color(Stream::Stdout, |t| t.dimmed()),
    );
}

fn render_snippet(out: &mut String, value: &serde_json::Value) {
    if let Some(s) = value.as_str() {
        out.push_str(s);
        if !s.ends_with('\n') {
            out.push('\n');
        }
    } else if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        let _ = writeln!(out, "{err}");
    } else {
        let _ = writeln!(out, "(no snippet)");
    }
}

fn render_trace_parameter(out: &mut String, value: &serde_json::Value) {
    if value.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false) {
        render_trace_parameter_recursive(out, value);
        return;
    }

    let module = value.get("module").and_then(|v| v.as_str()).unwrap_or("?");
    let param  = value.get("parameter").and_then(|v| v.as_str()).unwrap_or("?");
    let n      = value.get("affected_instances").and_then(|v| v.as_u64()).unwrap_or(0);

    let Some(arr) = value
        .get("instances")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    else {
        let _ = writeln!(out, "{}.{} {} no propagations found",
            module.if_supports_color(Stream::Stdout, |t| t.yellow()),
            param.if_supports_color(Stream::Stdout, |t| t.magenta()),
            "→".if_supports_color(Stream::Stdout, |t| t.blue()),
        );
        return;
    };

    // Group parameters by instance
    let mut instances: BTreeMap<String, Vec<&serde_json::Value>> = BTreeMap::new();
    for inst in arr {
        let parent = inst.get("parent").and_then(|v| v.as_str()).unwrap_or("?");
        let child  = inst.get("child").and_then(|v| v.as_str()).unwrap_or("?");
        let inm    = inst.get("instance").and_then(|v| v.as_str()).unwrap_or("?");
        instances.entry(format!("{parent}::{child}::{inm}")).or_default().push(inst);
    }

    let inst_count = instances.len();
    let _ = writeln!(out, "{}.{} {} {n} propagations across {inst_count} instance(s)\n",
        module.if_supports_color(Stream::Stdout, |t| t.yellow()),
        param.if_supports_color(Stream::Stdout, |t| t.magenta()),
        "→".if_supports_color(Stream::Stdout, |t| t.blue()),
    );

    for (idx, (_key, params)) in instances.into_iter().enumerate() {
        let is_last    = idx + 1 == inst_count;
        let box_char   = if is_last { "└─" } else { "├─" };
        let indent_char = if is_last { "  " } else { "│ " };
        // All params in group share same instance metadata
        let first = params[0];
        let parent = first.get("parent").and_then(|v| v.as_str()).unwrap_or("?");
        let child  = first.get("child").and_then(|v| v.as_str()).unwrap_or("?");
        let inm    = first.get("instance").and_then(|v| v.as_str()).unwrap_or("?");
        let file   = first.get("parent_file_path").and_then(|v| v.as_str()).unwrap_or("");
        let lstart = first.get("line_start").and_then(|v| v.as_i64());
        let lend   = first.get("line_end").and_then(|v| v.as_i64());

        let relpath = relative_path(file);
        let loc = match (lstart, lend) {
            (Some(s), Some(e)) => format!("{relpath}:{s}-{e}"),
            (Some(s), None)    => format!("{relpath}:{s}"),
            _                  => relpath.to_string(),
        };

        let count = params.len();
        let plural = if count == 1 { "parameter" } else { "parameters" };
        let _ = writeln!(out, "{} {} ({} {} {})  {}",
            box_char.if_supports_color(Stream::Stdout, |t| t.blue()),
            inm,
            parent.if_supports_color(Stream::Stdout, |t| t.yellow()),
            "→".if_supports_color(Stream::Stdout, |t| t.blue()),
            child.if_supports_color(Stream::Stdout, |t| t.yellow()),
            loc.if_supports_color(Stream::Stdout, |t| t.dimmed()),
        );
        let _ = writeln!(out, "{}  {count} {plural}:", indent_char);

        let param_count = params.len();
        for (pidx, inst) in params.into_iter().enumerate() {
            let is_last_param = pidx + 1 == param_count;
            let param_box   = if is_last_param { "   └─" } else { "   ├─" };
            let child_param = inst.get("child_parameter").and_then(|v| v.as_str()).unwrap_or("?");
            let call        = inst.get("call_site_expression").and_then(|v| v.as_str()).unwrap_or("?");
            let default_val = inst.get("child_param_default").and_then(|v| v.as_str());

            let default_str = match default_val {
                Some(d) if !d.is_empty() =>
                    format!("  (default: {})", d.if_supports_color(Stream::Stdout, |t| t.dimmed())),
                _ => String::new(),
            };
            let _ = writeln!(out, "{}{} {:<22} {} {}{}",
                indent_char,
                param_box.if_supports_color(Stream::Stdout, |t| t.blue()),
                child_param.if_supports_color(Stream::Stdout, |t| t.magenta()),
                "←".if_supports_color(Stream::Stdout, |t| t.blue()),
                call.if_supports_color(Stream::Stdout, |t| t.green()),
                default_str,
            );

            // Show resolved value if available (elab mode)
            if let Some(rv) = inst.get("resolved_value").and_then(|v| v.as_str()) {
                let _ = writeln!(out, "{}      resolved: {}",
                    indent_char,
                    rv.if_supports_color(Stream::Stdout, |t| t.bright_green()),
                );
            }
        }

        out.push('\n'); // Blank line between instances
    }
}

fn render_trace_signal(out: &mut String, value: &serde_json::Value) {
    if value.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false) {
        render_trace_signal_recursive(out, value);
        return;
    }

    let module = value.get("module").and_then(|v| v.as_str()).unwrap_or("?");
    let signal = value.get("signal").and_then(|v| v.as_str()).unwrap_or("?");
    let n      = value.get("connections").and_then(|v| v.as_u64()).unwrap_or(0);

    let Some(arr) = value
        .get("instances")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    else {
        let _ = writeln!(out, "{}.{} {} no connections found",
            module.if_supports_color(Stream::Stdout, |t| t.yellow()),
            signal.if_supports_color(Stream::Stdout, |t| t.cyan()),
            "→".if_supports_color(Stream::Stdout, |t| t.blue()),
        );
        return;
    };

    // Group ports by instance (parent::child::instance_name)
    let mut instances: BTreeMap<String, Vec<&serde_json::Value>> = BTreeMap::new();
    for conn in arr {
        let parent = conn.get("parent").and_then(|v| v.as_str()).unwrap_or("?");
        let child  = conn.get("child").and_then(|v| v.as_str()).unwrap_or("?");
        let inm    = conn.get("instance").and_then(|v| v.as_str()).unwrap_or("?");
        instances.entry(format!("{parent}::{child}::{inm}")).or_default().push(conn);
    }

    let inst_count = instances.len();
    let _ = writeln!(out, "{}.{} {} {n} connection(s) across {inst_count} instance(s)\n",
        module.if_supports_color(Stream::Stdout, |t| t.yellow()),
        signal.if_supports_color(Stream::Stdout, |t| t.cyan()),
        "→".if_supports_color(Stream::Stdout, |t| t.blue()),
    );

    for (idx, (_key, ports)) in instances.into_iter().enumerate() {
        let is_last    = idx + 1 == inst_count;
        let box_char   = if is_last { "└─" } else { "├─" };
        let indent_char = if is_last { "  " } else { "│ " };
        let first  = ports[0];
        let parent = first.get("parent").and_then(|v| v.as_str()).unwrap_or("?");
        let child  = first.get("child").and_then(|v| v.as_str()).unwrap_or("?");
        let inm    = first.get("instance").and_then(|v| v.as_str()).unwrap_or("?");
        let file   = first.get("parent_file_path").and_then(|v| v.as_str()).unwrap_or("");
        let lstart = first.get("line_start").and_then(|v| v.as_i64());
        let lend   = first.get("line_end").and_then(|v| v.as_i64());

        let relpath = relative_path(file);
        let loc = match (lstart, lend) {
            (Some(s), Some(e)) => format!("{relpath}:{s}-{e}"),
            (Some(s), None)    => format!("{relpath}:{s}"),
            _                  => relpath.to_string(),
        };

        let count = ports.len();
        let plural = if count == 1 { "port" } else { "ports" };
        let _ = writeln!(out, "{} {} ({} {} {})  {}",
            box_char.if_supports_color(Stream::Stdout, |t| t.blue()),
            inm,
            parent.if_supports_color(Stream::Stdout, |t| t.yellow()),
            "→".if_supports_color(Stream::Stdout, |t| t.blue()),
            child.if_supports_color(Stream::Stdout, |t| t.yellow()),
            loc.if_supports_color(Stream::Stdout, |t| t.dimmed()),
        );
        let _ = writeln!(out, "{}  {count} {plural}:", indent_char);

        let port_count = ports.len();
        for (pidx, conn) in ports.into_iter().enumerate() {
            let is_last_port = pidx + 1 == port_count;
            let port_box   = if is_last_port { "   └─" } else { "   ├─" };
            let child_port = conn.get("child_port").and_then(|v| v.as_str()).unwrap_or("?");
            let expr       = conn.get("parent_expression").and_then(|v| v.as_str()).unwrap_or("?");
            let _ = writeln!(out, "{}{} {:<22} {} {}",
                indent_char,
                port_box.if_supports_color(Stream::Stdout, |t| t.blue()),
                child_port.if_supports_color(Stream::Stdout, |t| t.cyan()),
                "←".if_supports_color(Stream::Stdout, |t| t.blue()),
                expr.if_supports_color(Stream::Stdout, |t| t.green()),
            );
        }

        out.push('\n');
    }
}

fn render_trace_signal_recursive(out: &mut String, value: &serde_json::Value) {
    let module    = value.get("module").and_then(|v| v.as_str()).unwrap_or("?");
    let signal    = value.get("signal").and_then(|v| v.as_str()).unwrap_or("?");
    let max_depth = value.get("max_depth").and_then(|v| v.as_i64()).unwrap_or(5);
    let n         = value.get("connections").and_then(|v| v.as_u64()).unwrap_or(0);

    let Some(arr) = value
        .get("instances")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    else {
        let _ = writeln!(out, "{}.{} {} no connections found",
            module.if_supports_color(Stream::Stdout, |t| t.yellow()),
            signal.if_supports_color(Stream::Stdout, |t| t.cyan()),
            "→".if_supports_color(Stream::Stdout, |t| t.blue()),
        );
        return;
    };

    let _ = writeln!(out, "{}.{} {} {n} connection(s), depth ≤ {max_depth}\n",
        module.if_supports_color(Stream::Stdout, |t| t.yellow()),
        signal.if_supports_color(Stream::Stdout, |t| t.cyan()),
        "→".if_supports_color(Stream::Stdout, |t| t.blue()),
    );
    render_trace_tree_nodes(out, arr, "", &|inst| {
        let port = inst["child_port"].as_str().unwrap_or("?");
        format!("[→ {}]", port.if_supports_color(Stream::Stdout, |t| t.cyan()))
    });
}

/// Generic recursive tree renderer for `trace-signal --recursive` and
/// `trace-parameter --recursive`. `annotate(inst)` returns the trailing
/// `[→ ...]` annotation string that differs between the two commands.
fn render_trace_tree_nodes(
    out: &mut String,
    instances: &[serde_json::Value],
    prefix: &str,
    annotate: &dyn Fn(&serde_json::Value) -> String,
) {
    let total = instances.len();
    for (idx, inst) in instances.iter().enumerate() {
        let is_last   = idx + 1 == total;
        let box_char  = if is_last { "└─" } else { "├─" };
        let child_pfx = if is_last { format!("{}    ", prefix) } else { format!("{}│   ", prefix) };

        let inm    = inst["instance"].as_str().unwrap_or("?");
        let parent = inst["parent"].as_str().unwrap_or("?");
        let child  = inst["child"].as_str().unwrap_or("?");
        let file   = inst["parent_file_path"].as_str().unwrap_or("");
        let lstart = inst["line_start"].as_i64();
        let lend   = inst["line_end"].as_i64();
        let relpath = relative_path(file);
        let loc = match (lstart, lend) {
            (Some(s), Some(e)) => format!("{relpath}:{s}-{e}"),
            (Some(s), None)    => format!("{relpath}:{s}"),
            _                  => relpath.to_string(),
        };
        let children = inst["children"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        let leaf     = if children.is_empty() { "  (leaf)" } else { "" };
        let ann      = annotate(inst);

        let _ = writeln!(out, "{}{} {} ({} {} {})  {}  {}{}",
            prefix,
            box_char.if_supports_color(Stream::Stdout, |t| t.blue()),
            inm,
            parent.if_supports_color(Stream::Stdout, |t| t.yellow()),
            "→".if_supports_color(Stream::Stdout, |t| t.blue()),
            child.if_supports_color(Stream::Stdout, |t| t.yellow()),
            loc.if_supports_color(Stream::Stdout, |t| t.dimmed()),
            ann,
            leaf.if_supports_color(Stream::Stdout, |t| t.dimmed()),
        );

        if !children.is_empty() {
            render_trace_tree_nodes(out, children, &child_pfx, annotate);
        }
    }
}

fn render_trace_parameter_recursive(out: &mut String, value: &serde_json::Value) {
    let module    = value.get("module").and_then(|v| v.as_str()).unwrap_or("?");
    let param     = value.get("parameter").and_then(|v| v.as_str()).unwrap_or("?");
    let max_depth = value.get("max_depth").and_then(|v| v.as_i64()).unwrap_or(5);
    let n         = value.get("affected_instances").and_then(|v| v.as_u64()).unwrap_or(0);

    let Some(arr) = value
        .get("instances")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    else {
        let _ = writeln!(out, "{}.{} {} no propagations found",
            module.if_supports_color(Stream::Stdout, |t| t.yellow()),
            param.if_supports_color(Stream::Stdout, |t| t.magenta()),
            "→".if_supports_color(Stream::Stdout, |t| t.blue()),
        );
        return;
    };

    let _ = writeln!(out, "{}.{} {} {n} propagation(s), depth ≤ {max_depth}\n",
        module.if_supports_color(Stream::Stdout, |t| t.yellow()),
        param.if_supports_color(Stream::Stdout, |t| t.magenta()),
        "→".if_supports_color(Stream::Stdout, |t| t.blue()),
    );
    render_trace_tree_nodes(out, arr, "", &|inst| {
        let child_param = inst["child_parameter"].as_str().unwrap_or("?");
        let call        = inst["call_site_expression"].as_str().unwrap_or("?");
        format!("[→ {} ← {}]",
            child_param.if_supports_color(Stream::Stdout, |t| t.magenta()),
            call.if_supports_color(Stream::Stdout, |t| t.green()),
        )
    });
}

fn render_check_connectivity(out: &mut String, value: &serde_json::Value) {
    let module = value.get("module").and_then(|v| v.as_str()).unwrap_or("?");
    let depth = value.get("depth").and_then(|v| v.as_i64()).unwrap_or(0);
    let n = value
        .get("issue_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let _ = writeln!(out, "{module} @ depth={depth}: {n} issue(s)");
    let Some(arr) = value
        .get("findings")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    else {
        return;
    };
    for f in arr {
        let kind = f.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let parent = f.get("parent").and_then(|v| v.as_str()).unwrap_or("?");
        let child = f.get("child").and_then(|v| v.as_str()).unwrap_or("?");
        let inst = f.get("instance").and_then(|v| v.as_str()).unwrap_or("?");
        let port = f.get("port").and_then(|v| v.as_str()).unwrap_or("?");
        let iw = f.get("instance_width").and_then(|v| v.as_i64());
        let dw = f.get("declared_width").and_then(|v| v.as_i64());
        let mut line = format!("  - {kind}: {parent} -> {child} ({inst}).{port}");
        if let (Some(iw), Some(dw)) = (iw, dw) {
            line.push_str(&format!("   instance={iw} declared={dw}"));
        }
        let _ = writeln!(out, "{line}");
        if let Some(fields) = f
            .get("field_breakdown")
            .and_then(|v| v.as_object())
            .filter(|m| !m.is_empty())
        {
            for (fname, fw) in fields {
                let fv = fw.as_i64().unwrap_or(0);
                let _ = writeln!(out, "      {fname}   {fv}");
            }
        }
    }
}

fn render_match_interfaces(out: &mut String, value: &serde_json::Value) {
    let a = value
        .get("module_a")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let b = value
        .get("module_b")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let _ = writeln!(out, "{a} <-> {b}");
    let matched = value
        .get("matched")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let _ = writeln!(out, "  matched ({}):", matched.len());
    for m in matched {
        let port = m.get("port").and_then(|v| v.as_str()).unwrap_or("?");
        let ad = m.get("a_direction").and_then(|v| v.as_str()).unwrap_or("?");
        let bd = m.get("b_direction").and_then(|v| v.as_str()).unwrap_or("?");
        let aw = m.get("a_width").and_then(|v| v.as_i64());
        let bw = m.get("b_width").and_then(|v| v.as_i64());
        let dir_ok = m
            .get("direction_complementary")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dir_marker = if dir_ok { "" } else { " (NOT COMPLEMENTARY)" };
        let aw_s = aw.map(|w| w.to_string()).unwrap_or_else(|| "?".into());
        let bw_s = bw.map(|w| w.to_string()).unwrap_or_else(|| "?".into());
        let _ = writeln!(
            out,
            "    {port}   a:{ad}/{aw_s} <-> b:{bd}/{bw_s}{dir_marker}"
        );
    }
    if let Some(conflicts) = value
        .get("width_conflicts")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        let _ = writeln!(out, "  width_conflicts ({}):", conflicts.len());
        for c in conflicts {
            let port = c.get("port").and_then(|v| v.as_str()).unwrap_or("?");
            let aw = c.get("a_width").and_then(|v| v.as_i64());
            let bw = c.get("b_width").and_then(|v| v.as_i64());
            let _ = writeln!(
                out,
                "    {port}   a={}   b={}",
                aw.map(|w| w.to_string()).unwrap_or_else(|| "?".into()),
                bw.map(|w| w.to_string()).unwrap_or_else(|| "?".into()),
            );
        }
    }
    render_unmatched(out, value, "unmatched_a", a);
    render_unmatched(out, value, "unmatched_b", b);
}

fn render_unmatched(out: &mut String, value: &serde_json::Value, key: &str, label: &str) {
    let Some(arr) = value
        .get(key)
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    else {
        return;
    };
    let names: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
    let _ = writeln!(
        out,
        "  unmatched in {label} ({}): {}",
        names.len(),
        names.join(", ")
    );
}

fn render_edge(out: &mut String, edge: &serde_json::Value, base: usize) {
    let parent = edge.get("parent").and_then(|v| v.as_str()).unwrap_or("?");
    let child  = edge.get("child").and_then(|v| v.as_str()).unwrap_or("?");
    let inst   = edge.get("instance_name").and_then(|v| v.as_str()).unwrap_or("?");
    let file   = edge.get("parent_file_path").and_then(|v| v.as_str()).unwrap_or("");
    let lstart = edge.get("line_start").and_then(|v| v.as_i64());
    let lend   = edge.get("line_end").and_then(|v| v.as_i64());
    let loc    = location_string(file, lstart, lend);
    let _ = writeln!(out, "{}{} {} {} ({}){}",
        Indent(base),
        parent.if_supports_color(Stream::Stdout, |t| t.yellow()),
        "→".if_supports_color(Stream::Stdout, |t| t.blue()),
        child.if_supports_color(Stream::Stdout, |t| t.yellow()),
        inst,
        loc.if_supports_color(Stream::Stdout, |t| t.dimmed()),
    );

    let textual = edge.get("param_bindings").and_then(|v| v.as_object());
    let resolved = edge
        .get("resolved_param_values")
        .and_then(|v| v.as_object());
    let has_params =
        textual.is_some_and(|m| !m.is_empty()) || resolved.is_some_and(|m| !m.is_empty());
    if has_params {
        let _ = writeln!(out, "{}params:", Indent(base + 1));
        let mut keys: BTreeSet<&str> = BTreeSet::new();
        if let Some(t) = textual {
            keys.extend(t.keys().map(|k| k.as_str()));
        }
        if let Some(r) = resolved {
            keys.extend(r.keys().map(|k| k.as_str()));
        }
        for k in keys {
            let r = resolved.and_then(|m| m.get(k)).and_then(|v| v.as_str());
            let t = textual.and_then(|m| m.get(k)).and_then(|v| v.as_str());
            match (r, t) {
                (Some(rv), Some(tv)) => {
                    let _ = writeln!(out, "{}{k} = {rv}   (call site: {tv})", Indent(base + 2));
                }
                (Some(rv), None) => {
                    let _ = writeln!(out, "{}{k} = {rv}", Indent(base + 2));
                }
                (None, Some(tv)) => {
                    let _ = writeln!(out, "{}{k}   (call site: {tv})", Indent(base + 2));
                }
                (None, None) => {}
            }
        }
    }

    let pw = edge.get("resolved_port_widths").and_then(|v| v.as_object());
    if let Some(pw) = pw.filter(|m| !m.is_empty()) {
        let _ = writeln!(out, "{}ports:", Indent(base + 1));
        for (name, w) in pw {
            render_port_width(out, name, w, base + 2);
        }
    }
}

fn render_port_width(out: &mut String, name: &str, w: &serde_json::Value, depth: usize) {
    let total = w.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
    let count = w.get("element_count").and_then(|v| v.as_i64());
    match count {
        Some(c) => {
            let _ = writeln!(
                out,
                "{}{name}   total={total}   element_count={c}",
                Indent(depth)
            );
        }
        None => {
            let _ = writeln!(out, "{}{name}   total={total}", Indent(depth));
        }
    }
    if let Some(fields) = w
        .get("fields")
        .and_then(|v| v.as_object())
        .filter(|m| !m.is_empty())
    {
        for (fname, fw) in fields {
            let fv = fw.as_i64().unwrap_or(0);
            let _ = writeln!(out, "{}{fname}   {fv}", Indent(depth + 1));
        }
    }
    if let Some(elem) = w.get("element").filter(|v| !v.is_null()) {
        let _ = writeln!(out, "{}element:", Indent(depth + 1));
        let etot = elem.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
        let _ = writeln!(out, "{}total={etot}", Indent(depth + 2));
        if let Some(ef) = elem
            .get("fields")
            .and_then(|v| v.as_object())
            .filter(|m| !m.is_empty())
        {
            for (fname, fw) in ef {
                let fv = fw.as_i64().unwrap_or(0);
                let _ = writeln!(out, "{}{fname}   {fv}", Indent(depth + 2));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance_op() -> QueryOp {
        QueryOp::GetInstanceContext {
            parent: "p".into(),
            child: "c".into(),
        }
    }

    fn subgraph_op() -> QueryOp {
        QueryOp::GetSubgraph {
            name: "p".into(),
            depth: 2,
        }
    }

    fn parents_op() -> QueryOp {
        QueryOp::GetParents { name: "c".into() }
    }

    fn trace_path_op() -> QueryOp {
        QueryOp::TraceHierarchyPath {
            from_module: "p".into(),
            to_module: "c".into(),
        }
    }

    #[test]
    fn tree_renders_instance_edge_with_struct_and_array_ports() {
        let value = serde_json::json!([{
            "parent": "avsbus_controller",
            "child": "axi_lite_to_apb",
            "instance_name": "u_axi_lite_to_apb",
            "parent_file_path": "rtl/avsbus_controller.sv",
            "line_start": 334,
            "line_end": 342,
            "design": "test_design",
            "param_bindings": {
                "AddrWidth": "avsbus_controller_pkg::ADDR_WIDTH",
                "DataWidth": "32"
            },
            "resolved_param_values": {
                "AddrWidth": "32'd32",
                "DataWidth": "32'd32"
            },
            "port_bindings": {},
            "resolved_port_widths": {
                "apb_req_o": {
                    "total": 74,
                    "fields": { "paddr": 32, "pprot": 3, "psel": 1 }
                },
                "req_arr_i": {
                    "total": 144,
                    "fields": {},
                    "element_count": 4,
                    "element": {
                        "total": 36,
                        "fields": { "addr": 32, "valid": 1 }
                    }
                },
                "clk_i": { "total": 1, "fields": {} }
            }
        }]);
        let out = format_tree(&value, &instance_op()).expect("instance ctx renders");
        let expected = "\
avsbus_controller → axi_lite_to_apb (u_axi_lite_to_apb) [rtl/avsbus_controller.sv:334-342]
  params:
    AddrWidth = 32'd32   (call site: avsbus_controller_pkg::ADDR_WIDTH)
    DataWidth = 32'd32   (call site: 32)
  ports:
    apb_req_o   total=74
      paddr   32
      pprot   3
      psel   1
    clk_i   total=1
    req_arr_i   total=144   element_count=4
      element:
        total=36
        addr   32
        valid   1
";
        assert_eq!(out, expected);
    }

    #[test]
    fn tree_renders_empty_instance_context() {
        let out = format_tree(&serde_json::json!([]), &instance_op()).unwrap();
        assert_eq!(out, "(no edges)\n");
    }

    #[test]
    fn tree_renders_subgraph() {
        let value = serde_json::json!({
            "root": "top",
            "nodes": [{"name": "top"}, {"name": "mid"}, {"name": "leaf"}],
            "edges": [{
                "parent": "top",
                "child": "mid",
                "instance_name": "u_mid",
                "parent_file_path": "",
                "line_start": null,
                "line_end": null,
                "design": "d",
                "param_bindings": {},
                "resolved_param_values": {},
                "port_bindings": {},
                "resolved_port_widths": {}
            }]
        });
        let out = format_tree(&value, &subgraph_op()).unwrap();
        assert_eq!(
            out,
            "top  3 module(s), 1 instantiation(s)\n\n└─ u_mid → mid\n",
        );
    }

    #[test]
    fn tree_renders_parents() {
        let value = serde_json::json!([
            {"name": "top", "design": "d1", "file_path": "rtl/top.sv"},
            {"name": "wrapper", "design": "d1", "file_path": "rtl/wrap.sv"}
        ]);
        let out = format_tree(&value, &parents_op()).unwrap();
        assert_eq!(out, "top [d1] rtl/top.sv\nwrapper [d1] rtl/wrap.sv\n");
    }

    #[test]
    fn tree_renders_trace_hierarchy_path() {
        let value = serde_json::json!([
            {
                "parent": "top",
                "child": "mid",
                "instance_name": "u_mid",
                "parent_file_path": "rtl/top.sv",
                "line_start": 10,
                "line_end": 12,
                "design": "d",
                "param_bindings": {},
                "resolved_param_values": {},
                "port_bindings": {},
                "resolved_port_widths": {}
            }
        ]);
        let out = format_tree(&value, &trace_path_op()).unwrap();
        assert_eq!(out, "top → mid (u_mid) [rtl/top.sv:10-12]\n");
    }

    #[test]
    fn tree_renders_search_module_hits() {
        let value = serde_json::json!([
            {
                "name": "axi_lite_to_apb",
                "design": "test_design",
                "score": 0.828,
                "file_path": "rtl/axi_lite_to_apb.sv",
                "description": "",
                "num_ports": 4,
                "num_params": 5,
                "num_instantiations": 0,
            },
            {
                "name": "prim_axi_lite_to_apb",
                "design": "test_design",
                "score": 0.7195,
                "file_path": "",
                "description": "",
                "num_ports": 0,
                "num_params": 0,
                "num_instantiations": 0,
            }
        ]);
        let op = QueryOp::SearchModules {
            query: "axi".into(),
            top_k: 5,
            design: None,
        };
        let out = format_tree(&value, &op).unwrap();
        assert_eq!(
            out,
            "axi_lite_to_apb [test_design] score=0.828 rtl/axi_lite_to_apb.sv\n\
             prim_axi_lite_to_apb [test_design] score=0.720\n",
        );
    }

    #[test]
    fn tree_renders_get_module_with_params_and_ports() {
        let value = serde_json::json!({
            "name": "axi_lite_to_apb",
            "design": "test_design",
            "file_path": "rtl/axi_lite_to_apb.sv",
            "is_package": false,
            "line_start": 5,
            "line_end": 120,
            "description": "AXI-lite to APB bridge",
            "parameters": [
                {"name": "AddrWidth", "kind": "int", "default_value": "32", "is_type_param": false},
                {"name": "apb_req_t", "kind": "type", "default_value": "", "is_type_param": true}
            ],
            "ports": [
                {"name": "clk_i", "direction": "input", "type_str": "logic", "width_expr": "", "bit_width": 1},
                {"name": "req_i", "direction": "input", "type_str": "axi_lite_req_t", "width_expr": "", "bit_width": null}
            ],
            "instantiations": [
                {"module_name": "addr_decode", "instance_name": "i_dec"}
            ],
            "imports": [
                {"package_name": "avsbus_pkg", "is_wildcard": true, "specific_symbols": []}
            ]
        });
        let op = QueryOp::GetModule {
            name: "axi_lite_to_apb".into(),
        };
        let out = format_tree(&value, &op).unwrap();
        let expected = "\
axi_lite_to_apb [test_design]  axi_lite_to_apb.sv:5-120
  description: AXI-lite to APB bridge
  parameters (2):
    AddrWidth: int = 32
    apb_req_t: type
  ports (2):
    clk_i (input)   width=1   type=logic
    req_i (input)   type=axi_lite_req_t
  instantiations (1):
    addr_decode (i_dec)
  imports (1): avsbus_pkg
";
        assert_eq!(out, expected);
    }

    #[test]
    fn tree_renders_get_module_error_payload() {
        let value = serde_json::json!({"error": "Module 'foo' not found"});
        let op = QueryOp::GetModule { name: "foo".into() };
        let out = format_tree(&value, &op).unwrap();
        assert_eq!(out, "Module 'foo' not found\n");
    }

    #[test]
    fn tree_renders_get_ports_with_struct_breakdown() {
        let value = serde_json::json!([
            {"name": "clk_i", "direction": "input", "type_str": "logic", "bit_width": 1},
            {"name": "req_i", "direction": "input", "type_str": "axi_req_t", "bit_width": null, "width_expr": ""}
        ]);
        let op = QueryOp::GetPorts { name: "x".into() };
        let out = format_tree(&value, &op).unwrap();
        assert_eq!(
            out,
            "clk_i (input)   width=1   type=logic\n\
             req_i (input)   type=axi_req_t\n",
        );
    }

    #[test]
    fn tree_renders_trace_parameter_with_affected_widths() {
        let value = serde_json::json!({
            "module": "avsbus_controller",
            "parameter": "AddrWidth",
            "affected_instances": 1,
            "instances": [{
                "parent": "avsbus_controller",
                "child": "axi_lite_to_apb",
                "instance": "u_axi_lite_to_apb",
                "call_site_expression": "avsbus_controller_pkg::ADDR_WIDTH",
                "resolved_value": "32'd32",
                "affected_port_widths": {
                    "apb_req_o": {"total": 74, "fields": {"paddr": 32}}
                },
                "parent_file_path": "rtl/avsbus_controller.sv",
                "line_start": 334,
                "line_end": 342
            }]
        });
        let op = QueryOp::TraceParameter {
            module_name: "avsbus_controller".into(),
            param_name: "AddrWidth".into(),
            recursive: false,
            depth: 5,
        };
        let out = format_tree(&value, &op).unwrap();
        let expected = "\
avsbus_controller.AddrWidth → 1 propagations across 1 instance(s)

└─ u_axi_lite_to_apb (avsbus_controller → axi_lite_to_apb)  avsbus_controller.sv:334-342
    1 parameter:
     └─ ?                      ← avsbus_controller_pkg::ADDR_WIDTH
        resolved: 32'd32

";
        assert_eq!(out, expected);
    }

    #[test]
    fn tree_renders_check_connectivity_findings() {
        let value = serde_json::json!({
            "module": "top",
            "depth": 2,
            "issue_count": 1,
            "findings": [{
                "kind": "width_mismatch",
                "parent": "top",
                "child": "leaf",
                "instance": "u_leaf",
                "port": "data_i",
                "instance_width": 32,
                "declared_width": 64,
                "field_breakdown": {"hi": 16, "lo": 16}
            }]
        });
        let op = QueryOp::CheckConnectivity {
            module_name: "top".into(),
            depth: 2,
        };
        let out = format_tree(&value, &op).unwrap();
        let expected = "\
top @ depth=2: 1 issue(s)
  - width_mismatch: top -> leaf (u_leaf).data_i   instance=32 declared=64
      hi   16
      lo   16
";
        assert_eq!(out, expected);
    }

    #[test]
    fn tree_renders_match_interfaces_pairs() {
        let value = serde_json::json!({
            "module_a": "axi_master",
            "module_b": "axi_slave",
            "matched": [{
                "port": "valid",
                "a_direction": "output",
                "b_direction": "input",
                "direction_complementary": true,
                "a_width": 1,
                "b_width": 1
            }, {
                "port": "data",
                "a_direction": "output",
                "b_direction": "output",
                "direction_complementary": false,
                "a_width": 32,
                "b_width": 32
            }],
            "width_conflicts": [],
            "unmatched_a": ["dbg_o"],
            "unmatched_b": []
        });
        let op = QueryOp::MatchInterfaces {
            module_a: "axi_master".into(),
            module_b: "axi_slave".into(),
            prefix_a: "".into(),
            prefix_b: "".into(),
        };
        let out = format_tree(&value, &op).unwrap();
        let expected = "\
axi_master <-> axi_slave
  matched (2):
    valid   a:output/1 <-> b:input/1
    data   a:output/32 <-> b:output/32 (NOT COMPLEMENTARY)
  unmatched in axi_master (1): dbg_o
";
        assert_eq!(out, expected);
    }

    #[test]
    fn tree_renders_snippet_string() {
        let op = QueryOp::GetSourceSnippet {
            module_name: "x".into(),
            element: "module".into(),
            instance_name: "".into(),
        };
        let out = format_tree(&serde_json::json!("module x;\n"), &op).unwrap();
        assert_eq!(out, "module x;\n");
    }

    #[test]
    fn tree_renders_find_structurally_similar_candidates() {
        let value = serde_json::json!({
            "module": "axi_demux",
            "candidates": [
                {"name": "axi_mux", "score": 0.75, "shared_ports": 6},
                {"name": "axi_xbar", "score": 0.4, "shared_ports": 3}
            ]
        });
        let op = QueryOp::FindStructurallySimilar {
            module_name: "axi_demux".into(),
            min_overlap: 0.3,
            design: None,
        };
        let out = format_tree(&value, &op).unwrap();
        let expected = "\
axi_demux: structurally-similar candidates
  axi_mux score=0.750 shared_ports=6
  axi_xbar score=0.400 shared_ports=3
";
        assert_eq!(out, expected);
    }
}

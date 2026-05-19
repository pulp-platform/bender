// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Grafeo-backed knowledge-graph store for `bender kg`.
//!
//! Schema:
//!   Node labels: `Module`, `Design`
//!   Edge types : `INSTANTIATES` (Module -> Module),
//!                `IMPORTS`      (Module -> Module),
//!                `BELONGS_TO`   (Module -> Design)
//!
//! Each module's ports / parameters / imports are JSON strings on the
//! `Module` node so a single MATCH returns everything needed to
//! reconstruct a [`ModuleData`]. Embeddings live as a `Module.embedding`
//! `Value::Vector` property and are searched through Grafeo's HNSW index.
//!
//! ## Per-design identity
//!
//! Modules are scoped per design. A `Module` node's identity is a compound
//! key `<design>::<name>` stored as `m.key`; the user-visible name lives
//! on `m.name`. Two designs that both define `axi_pkg` produce two
//! independent nodes with identical `name` but distinct `key`. This lets
//! `clear_design(alias)` cleanly wipe everything that design owns without
//! touching modules from other designs that happen to share a name.
//!
//! ## Edges
//!
//! Grafeo allows parallel edges between the same `(src, dst, type)`, so
//! every instantiation is its own edge with a self-contained property set
//! (`instance_name`, `param_bindings_json`, `port_bindings_json`,
//! `resolved_param_values_json`, `resolved_port_widths_json`,
//! `line_start`, `line_end`). No more JSON-array packing.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use bender_kg_models::{
    ModuleData, PortInfo, ResolvedPortWidth,
};
use grafeo::{Error as GrafeoError, GrafeoDB, Session, Value};
use grafeo_common::types::PropertyKey;
use thiserror::Error;

// Modular query components
mod instance;
mod graph;
mod port;
mod param;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("grafeo error: {0}")]
    Db(#[from] GrafeoError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("schema mismatch: {0}")]
    Schema(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Default UNWIND batch size for `upsert_modules`. Tuned for ~1.7k-module
/// designs; override via [`StoreConfig::with_upsert_chunk_size`] when
/// memory pressure or unusually wide modules suggest a smaller batch.
pub const DEFAULT_UPSERT_CHUNK_SIZE: usize = 4096;

/// Configuration for opening a knowledge-graph database.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Directory holding the database files. `bender kg` defaults to
    /// `<workspace>/.bender-kg/`.
    pub root: PathBuf,
    /// Optional database directory name (default: `graph.db`).
    pub db_filename: Option<String>,
    /// Embedding dimensionality. Used when creating the `:Module(embedding)`
    /// vector index on first open. Caller must keep this stable across
    /// rebuilds; passing a different dim against an existing index will
    /// fail at insert time.
    pub embedding_dim: Option<usize>,
    /// Maximum rows per UNWIND batch in `upsert_modules`. Larger means
    /// fewer Cypher round-trips; smaller means lower per-call memory.
    pub upsert_chunk_size: usize,
}

impl StoreConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            db_filename: None,
            embedding_dim: None,
            upsert_chunk_size: DEFAULT_UPSERT_CHUNK_SIZE,
        }
    }
    pub fn with_embedding_dim(mut self, dim: usize) -> Self {
        self.embedding_dim = Some(dim);
        self
    }
    pub fn with_upsert_chunk_size(mut self, n: usize) -> Self {
        self.upsert_chunk_size = n.max(1);
        self
    }
    pub fn db_path(&self) -> PathBuf {
        self.root
            .join(self.db_filename.as_deref().unwrap_or("graph.db"))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstanceEdge {
    pub parent: String,
    pub child: String,
    pub instance_name: String,
    /// Textual call-site expressions, keyed by the *child*'s parameter
    /// name. Captured by the syntactic walk, never overwritten by elab.
    pub param_bindings: BTreeMap<String, String>,
    /// Folded literal values produced by elaboration (e.g. `"32'd32"`).
    /// Empty for builds that ran without `--elab`.
    #[serde(default)]
    pub resolved_param_values: BTreeMap<String, String>,
    pub port_bindings: BTreeMap<String, String>,
    /// Per-port resolved width with optional packed-struct breakdown.
    /// Empty for builds that ran without `--elab`.
    pub resolved_port_widths: BTreeMap<String, ResolvedPortWidth>,
    /// Source file of the *parent* module — i.e. the file where the
    /// instantiation statement lives. `line_start` / `line_end` are
    /// offsets into this file.
    pub parent_file_path: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub design: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Subgraph {
    pub nodes: Vec<ModuleData>,
    pub edges: Vec<InstanceEdge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphStats {
    pub modules: usize,
    pub packages: usize,
    pub instantiations: usize,
    pub imports: usize,
    pub designs: Vec<DesignStat>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DesignStat {
    pub design: String,
    pub modules: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VectorHit {
    pub module: String,
    pub design: String,
    pub score: f32,
}

/// Grafeo-backed knowledge graph + vector store.
pub struct Store {
    db: GrafeoDB,
    db_path: PathBuf,
    upsert_chunk_size: usize,
}

impl Store {
    pub fn open(cfg: &StoreConfig) -> Result<Self> {
        std::fs::create_dir_all(&cfg.root)?;
        let path = cfg.db_path();
        let db = GrafeoDB::open(&path)?;
        // Ensure schema (idempotent). Both indexes auto-maintain on
        // subsequent `set_node_property` / Cypher SET writes.
        if let Some(dim) = cfg.embedding_dim {
            // Empty vector index pre-allocated; Grafeo populates it as we
            // write `Module.embedding` properties.
            db.create_vector_index(
                "Module",
                "embedding",
                Some(dim),
                Some("cosine"),
                None,
                None,
                None,
            )?;
        }
        // BM25 inverted index for `find_by_protocol`. No-op if already present.
        db.create_text_index("Module", "ports_json")?;
        Ok(Self {
            db,
            db_path: path,
            upsert_chunk_size: cfg.upsert_chunk_size.max(1),
        })
    }

    /// Maximum number of rows per UNWIND batch in [`Self::upsert_modules`].
    pub fn upsert_chunk_size(&self) -> usize {
        self.upsert_chunk_size
    }

    pub fn db_path(&self) -> Result<String> {
        Ok(self.db_path.to_string_lossy().into_owned())
    }

    // ------------------------------------------------------------------
    // Mutations
    // ------------------------------------------------------------------

    /// Upsert a single module. Convenience wrapper around
    /// [`Self::upsert_modules`] for callers that have one module in hand.
    pub fn upsert_module(&self, m: &ModuleData) -> Result<()> {
        self.upsert_modules(std::iter::once(m))?;
        Ok(())
    }

    /// Upsert a batch of modules and all their outgoing edges atomically.
    ///
    /// Everything happens inside a single Grafeo transaction. Each phase
    /// (node merge, design stub, child stub, INSTANTIATES, IMPORTS,
    /// BELONGS_TO) drives one or more `UNWIND $rows AS r ...` Cypher
    /// statements. Rows are chunked at [`Store::upsert_chunk_size`] so
    /// very large batches stay within Grafeo's per-call memory budget.
    /// Parallel edges between the same `(src, dst, type)` are preserved
    /// by `CREATE` (one row -> one edge).
    ///
    /// Identity is compound `<design>::<name>` so two designs can each
    /// carry their own copy of `axi_pkg` without colliding on a shared
    /// node.
    pub fn upsert_modules<'a, I>(&self, modules: I) -> Result<usize>
    where
        I: IntoIterator<Item = &'a ModuleData>,
    {
        let modules: Vec<&ModuleData> = modules.into_iter().collect();
        if modules.is_empty() {
            return Ok(0);
        }

        let mut session = self.db.session();
        session.begin_transaction()?;

        let chunk = self.upsert_chunk_size.max(1);

        // Pass 1a: bulk MERGE of every module node in the batch (one
        // Cypher per chunk).
        merge_module_nodes_batch(&session, &modules, chunk)?;

        // Pass 1b: bulk MERGE of each touched design alias. Deduped via
        // a sorted set so we never MERGE the same alias twice.
        let design_aliases: BTreeSet<String> = modules
            .iter()
            .filter(|m| !m.design.is_empty())
            .map(|m| m.design.clone())
            .collect();
        merge_design_stubs_batch(&session, &design_aliases, chunk)?;

        // Pass 1c: bulk MERGE of every child stub (instantiation and
        // import targets). Deduped per `<design>::<name>` so external
        // children that appear in many parents are merged once.
        merge_child_stubs_batch(&session, &modules, chunk)?;

        // Pass 2: bulk CREATE of all outgoing edges, one Cypher per kind.
        create_instantiates_batch(&session, &modules, chunk)?;
        create_imports_batch(&session, &modules, chunk)?;
        create_belongs_to_batch(&session, &modules, chunk)?;
        session.commit()?;
        Ok(modules.len())
    }

    /// Apply a batch of resolved-edge patches produced by deferred
    /// elaboration ([`bender_kg_extract::ElabHandle::run`]).
    ///
    /// Each row identifies an existing `INSTANTIATES` edge by
    /// `(parent_name, child_name, instance_name, design)` and SETs
    /// `resolved_param_values_json` / `resolved_port_widths_json` on
    /// it. Rows whose triple does not match any edge silently no-op
    /// (mirrors the prior inline merge's `inst_miss` warning case).
    /// Runs inside one transaction; chunked at the same
    /// [`Self::upsert_chunk_size`] used by `upsert_modules`.
    pub fn update_resolved_edges(
        &self,
        updates: &[bender_kg_models::ResolvedEdgeUpdate],
    ) -> Result<usize> {
        if updates.is_empty() {
            return Ok(0);
        }
        let mut session = self.db.session();
        session.begin_transaction()?;

        let rows: Vec<Value> = updates
            .iter()
            .map(|u| {
                row([
                    ("pn", Value::from(u.parent_module.as_str())),
                    ("cn", Value::from(u.child_module.as_str())),
                    ("inn", Value::from(u.instance_name.as_str())),
                    ("d", Value::from(u.design.as_str())),
                    ("rpv", Value::from(u.resolved_param_values_json.as_str())),
                    ("rpw", Value::from(u.resolved_port_widths_json.as_str())),
                ])
            })
            .collect();
        unwind_in_chunks(
            &session,
            "UNWIND $rows AS r \
             MATCH (p:Module {name: r.pn, design: r.d}) \
                 -[e:INSTANTIATES {instance_name: r.inn, design: r.d}]-> \
                 (c:Module {name: r.cn, design: r.d}) \
             SET e.resolved_param_values_json = r.rpv, \
                 e.resolved_port_widths_json = r.rpw",
            rows,
            self.upsert_chunk_size.max(1),
        )?;

        session.commit()?;
        Ok(updates.len())
    }

    /// Register design metadata. Idempotent: re-running with the same
    /// alias updates the existing node's properties via MERGE.
    pub fn register_design(
        &self,
        alias: &str,
        identity_id: &str,
        workspace: Option<&str>,
        top: Option<&str>,
        targets: &[String],
        defines: &[String],
    ) -> Result<()> {
        let p = HashMap::from([
            ("a".into(),   Value::from(alias)),
            ("id".into(),  Value::from(identity_id)),
            ("ws".into(),  Value::from(workspace.unwrap_or(""))),
            ("top".into(), Value::from(top.unwrap_or(""))),
            ("tg".into(),  Value::from(serde_json::to_string(targets)?)),
            ("df".into(),  Value::from(serde_json::to_string(defines)?)),
            ("ca".into(),  Value::from(now_unix_ts())),
        ]);
        self.db.execute_cypher_with_params(
            "MERGE (d:Design {alias: $a}) \
             SET d.identity_id = $id, d.workspace = $ws, d.top = $top, \
                 d.targets = $tg, d.defines = $df, d.created_at = $ca",
            p,
        )?;
        Ok(())
    }

    pub fn clear_design(&self, alias: &str) -> Result<()> {
        // Drop every Module owned by this design (with edges), then the
        // Design node itself. Parameterised DETACH DELETE handles both.
        self.db.execute_cypher_with_params(
            "MATCH (m:Module) WHERE m.design = $a DETACH DELETE m",
            cparam("a", alias),
        )?;
        self.db.execute_cypher_with_params(
            "MATCH (d:Design {alias: $a}) DETACH DELETE d",
            cparam("a", alias),
        )?;
        Ok(())
    }

    pub fn clear_all(&self) -> Result<()> {
        self.db.execute_cypher("MATCH (m:Module) DETACH DELETE m")?;
        self.db.execute_cypher("MATCH (d:Design) DETACH DELETE d")?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Reads
    // ------------------------------------------------------------------

    pub fn get_module(&self, name: &str) -> Result<Option<ModuleData>> {
        let r = self.db.execute_cypher_with_params(
            "MATCH (m:Module {name: $n}) \
             RETURN m.name, m.file_path, m.design, m.is_package, \
                    m.line_start, m.line_end, \
                    m.param_block_start, m.param_block_end, \
                    m.port_block_start, m.port_block_end, \
                    m.description, m.ports_json, m.params_json, m.imports_json",
            cparam("n", name),
        )?;
        let Some(row) = r.rows().first() else {
            return Ok(None);
        };
        // A bare stub (no full upsert yet) has nulls past column 1.
        if row_is_stub(row) {
            return Ok(None);
        }
        Ok(Some(row_to_module(row)?))
    }

    pub fn get_parents(&self, module: &str) -> Result<Vec<ModuleData>> {
        let r = self.db.execute_cypher_with_params(
            "MATCH (p:Module)-[:INSTANTIATES]->(:Module {name: $n}) \
             RETURN DISTINCT p.name",
            cparam("n", module),
        )?;
        let mut out = Vec::new();
        for row in r.rows() {
            let n = as_string(&row[0]);
            if let Some(m) = self.get_module(&n)? {
                out.push(m);
            }
        }
        Ok(out)
    }

    pub fn get_children(&self, module: &str) -> Result<Vec<ModuleData>> {
        let r = self.db.execute_cypher_with_params(
            "MATCH (:Module {name: $n})-[:INSTANTIATES]->(c:Module) \
             RETURN DISTINCT c.name",
            cparam("n", module),
        )?;
        let mut out = Vec::new();
        for row in r.rows() {
            let n = as_string(&row[0]);
            if let Some(m) = self.get_module(&n)? {
                out.push(m);
            }
        }
        Ok(out)
    }

    pub fn get_subgraph(&self, root: &str, depth: i32) -> Result<Subgraph> {
        let mut nodes_set: BTreeSet<String> = BTreeSet::new();
        let mut edges = Vec::new();
        let mut frontier: Vec<String> = vec![root.to_string()];
        nodes_set.insert(root.to_string());
        let mut steps = 0;
        while !frontier.is_empty() && steps < depth.max(0) {
            let mut next: Vec<String> = Vec::new();
            for parent in &frontier {
                for edge in self.list_instance_edges_from(parent)? {
                    if nodes_set.insert(edge.child.clone()) {
                        next.push(edge.child.clone());
                    }
                    edges.push(edge);
                }
            }
            frontier = next;
            steps += 1;
        }
        let mut nodes = Vec::with_capacity(nodes_set.len());
        for name in nodes_set {
            if let Some(m) = self.get_module(&name)? {
                nodes.push(m);
            }
        }
        Ok(Subgraph { nodes, edges })
    }

    pub fn get_instance_context(&self, parent: &str, child: &str) -> Result<Vec<InstanceEdge>> {
        let parent_file = self.module_meta(parent)?.1;
        let cypher = if child.is_empty() {
            instance::INSTANCE_EDGE_QUERY
        } else {
            instance::INSTANCE_EDGE_QUERY_FILTERED
        };
        let mut p = std::collections::HashMap::new();
        p.insert("p".into(), Value::from(parent));
        if !child.is_empty() {
            p.insert("c".into(), Value::from(child));
        }
        let r = self.db.execute_cypher_with_params(cypher, p)?;
        let mut out: Vec<InstanceEdge> = Vec::new();
        for row in r.rows() {
            out.push(instance::row_to_instance_edge(row, &parent_file));
        }
        Ok(out)
    }

    /// Trace the BFS hierarchy path between two modules. Implemented in
    /// Rust because reconstructing a path from a Cypher `shortestPath`
    /// requires walking edge metadata anyway.
    pub fn trace_hierarchy_path(&self, from: &str, to: &str) -> Result<Vec<InstanceEdge>> {
        graph::trace_hierarchy_path(&self.db, &|name| self.module_meta(name), from, to)
    }

    pub fn check_connectivity(&self, module: &str, depth: i32) -> Result<Vec<serde_json::Value>> {
        let mut findings = Vec::new();
        let sub = self.get_subgraph(module, depth)?;
        for edge in &sub.edges {
            let Some(child) = self.get_module(&edge.child)? else {
                continue;
            };
            for port in &child.ports {
                let Some(entry) = edge.resolved_port_widths.get(&port.name) else {
                    continue;
                };
                if entry.total == 0 {
                    continue;
                }
                let Some(decl) = port.bit_width else { continue };
                if entry.total != decl {
                    findings.push(serde_json::json!({
                        "kind": "width_mismatch",
                        "parent": edge.parent,
                        "child": edge.child,
                        "instance": edge.instance_name,
                        "port": port.name,
                        "instance_width": entry.total,
                        "declared_width": decl,
                        "field_breakdown": entry.fields,
                    }));
                }
            }
        }
        Ok(findings)
    }

    /// Find every instantiation that binds `param` along the
    /// `INSTANTIATES` edges incident to `module` (either as parent or
    /// child). Each parallel edge is its own row, so we just iterate.
    pub fn trace_parameter(&self, module: &str, param: &str) -> Result<Vec<serde_json::Value>> {
        let r = self.db.execute_cypher_with_params(
            "MATCH (p:Module)-[r:INSTANTIATES]->(c:Module) \
             WHERE p.name = $n OR c.name = $n \
             RETURN p.name, c.name, r.instance_name, r.design, \
                    r.param_bindings_json, r.port_bindings_json, \
                    r.resolved_param_values_json, r.resolved_port_widths_json, \
                    r.line_start, r.line_end",
            cparam("n", module),
        )?;
        let mut file_cache: BTreeMap<String, String> = BTreeMap::new();
        let mut child_param_cache: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut out = Vec::new();
        for row in r.rows() {
            let parent_name = as_string(&row[0]);
            let parent_file = self.cached_file(&parent_name, &mut file_cache)?;
            let edge = instance::row_to_instance_edge(row, &parent_file);

            // Cache child module parameter defaults
            if !child_param_cache.contains_key(&edge.child) {
                let defaults = match self.get_module(&edge.child)? {
                    Some(m) => m.parameters.iter()
                        .map(|p| (p.name.clone(), p.default_value.clone()))
                        .collect(),
                    None => BTreeMap::new(),
                };
                child_param_cache.insert(edge.child.clone(), defaults);
            }

            // Check each parameter binding to see if it references our parameter
            // This now handles struct field accesses like "Cfg.JTAG_BSR_ENABLE"
            for (child_param_name, binding_value) in &edge.param_bindings {
                if param::value_references_param(param, binding_value) {
                    let child_param_default = child_param_cache
                        .get(&edge.child)
                        .and_then(|defaults| defaults.get(child_param_name).cloned());

                    out.push(serde_json::json!({
                        "parent": edge.parent,
                        "child": edge.child,
                        "instance": edge.instance_name,
                        "child_parameter": child_param_name,
                        "call_site_expression": binding_value,
                        "resolved_value": edge.resolved_param_values.get(child_param_name),
                        "child_param_default": child_param_default,
                        "affected_port_widths": edge.resolved_port_widths,
                        "parent_file_path": edge.parent_file_path,
                        "line_start": edge.line_start,
                        "line_end": edge.line_end,
                    }));
                }
            }
        }
        Ok(out)
    }

    /// Trace how a signal (port) on `module` propagates to child instances.
    ///
    /// Searches all instantiation edges where `module` is the parent and looks
    /// through `port_bindings` for any child port whose binding expression
    /// references `signal`.  Returns one entry per matching (instance, port) pair.
    pub fn trace_signal(&self, module: &str, signal: &str) -> Result<Vec<serde_json::Value>> {
        let r = self.db.execute_cypher_with_params(
            "MATCH (p:Module)-[r:INSTANTIATES]->(c:Module) \
             WHERE p.name = $n \
             RETURN p.name, c.name, r.instance_name, r.design, \
                    r.param_bindings_json, r.port_bindings_json, \
                    r.resolved_param_values_json, r.resolved_port_widths_json, \
                    r.line_start, r.line_end",
            cparam("n", module),
        )?;
        let mut file_cache: BTreeMap<String, String> = BTreeMap::new();
        let mut out = Vec::new();
        for row in r.rows() {
            let parent_name = as_string(&row[0]);
            let parent_file = self.cached_file(&parent_name, &mut file_cache)?;
            let edge = instance::row_to_instance_edge(row, &parent_file);

            for (child_port_name, binding_expr) in &edge.port_bindings {
                if param::value_references_signal(signal, binding_expr) {
                    out.push(serde_json::json!({
                        "parent": edge.parent,
                        "child": edge.child,
                        "instance": edge.instance_name,
                        "child_port": child_port_name,
                        "parent_expression": binding_expr,
                        "parent_file_path": edge.parent_file_path,
                        "line_start": edge.line_start,
                        "line_end": edge.line_end,
                    }));
                }
            }
        }
        Ok(out)
    }

    /// Find modules whose `ports_json` mentions the protocol keyword.
    /// Grafeo's planner pushes `CONTAINS` into the `:Module(ports_json)`
    /// inverted index when the BM25 token matches; otherwise it falls
    /// back to a property scan with the same correctness semantics.
    pub fn find_by_protocol(
        &self,
        protocol: &str,
        design: Option<&str>,
    ) -> Result<Vec<ModuleData>> {
        let kw = protocol.to_lowercase();
        let cypher = if design.is_some() {
            "MATCH (m:Module) \
             WHERE m.design = $d AND lower(m.ports_json) CONTAINS $kw \
             RETURN m.name"
        } else {
            "MATCH (m:Module) WHERE lower(m.ports_json) CONTAINS $kw RETURN m.name"
        };
        let r = self.db.execute_cypher_with_params(cypher, cparam_d("kw", kw.clone(), design))?;
        let mut out = Vec::new();
        for row in r.rows() {
            let n = as_string(&row[0]);
            // Re-confirm the match against the typed PortInfo to drop
            // false positives where the token appears outside type_str.
            if let Some(m) = self.get_module(&n)? {
                if m.ports
                    .iter()
                    .any(|p| p.type_str.to_lowercase().contains(&kw))
                {
                    out.push(m);
                }
            }
        }
        Ok(out)
    }

    pub fn match_interfaces(
        &self,
        a: &str,
        b: &str,
        prefix_a: &str,
        prefix_b: &str,
    ) -> Result<serde_json::Value> {
        let ma = self
            .get_module(a)?
            .ok_or_else(|| StoreError::NotFound(a.into()))?;
        let mb = self
            .get_module(b)?
            .ok_or_else(|| StoreError::NotFound(b.into()))?;

        let comparison = port::compare_ports(&ma.ports, &mb.ports, prefix_a, prefix_b);

        let matched: Vec<serde_json::Value> = comparison
            .matched
            .iter()
            .map(|m| {
                serde_json::json!({
                    "port": m.name,
                    "a_direction": port::direction_str(m.a_direction),
                    "b_direction": port::direction_str(m.b_direction),
                    "direction_complementary": m.direction_complementary,
                    "a_width": m.a_width,
                    "b_width": m.b_width,
                })
            })
            .collect();

        let width_conflicts: Vec<serde_json::Value> = comparison
            .width_conflicts
            .iter()
            .map(|c| {
                serde_json::json!({
                    "port": c.name,
                    "a_width": c.a_width,
                    "b_width": c.b_width,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "module_a": a,
            "module_b": b,
            "matched": matched,
            "width_conflicts": width_conflicts,
            "unmatched_a": comparison.unmatched_a,
            "unmatched_b": comparison.unmatched_b,
        }))
    }

    /// Find modules whose port set has Jaccard overlap >= `min_overlap`
    /// with `module`'s port set.
    ///
    /// One Cypher (`MATCH (m:Module) ... RETURN m.name, m.port_set_json,
    /// m.port_set_card`) feeds a linear two-pointer Jaccard against the
    /// target's already-sorted port set. Total cost is `O(N * P_avg)` for
    /// `N` candidates and average port-set cardinality `P_avg`, vs the
    /// previous `O(N * (Cypher round-trip + |A| + |B|))`. Port sets are
    /// pre-stamped on each Module node by the upsert path so we never
    /// reparse the full `ports_json`.
    pub fn find_structurally_similar(
        &self,
        module: &str,
        min_overlap: f64,
        design: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        // Look up target's pre-stamped port set in one round-trip.
        let cypher = match design {
            Some(_) => {
                "MATCH (m:Module {name: $n, design: $d}) \
                 RETURN m.port_set_json AS pj, m.port_set_card AS pc"
            }
            None => {
                "MATCH (m:Module {name: $n}) \
                 RETURN m.port_set_json AS pj, m.port_set_card AS pc"
            }
        };
        let r = self.db.execute_cypher_with_params(cypher, cparam_d("n", module, design))?;
        let rows = r.rows();
        let target_row = rows
            .first()
            .ok_or_else(|| StoreError::NotFound(module.into()))?;
        let target_sorted = port::parse_port_set_json(&as_string(&target_row[0]));
        let target_card = target_row[1].as_int64().unwrap_or(0);
        if target_sorted.is_empty() || target_card == 0 {
            return Ok(Vec::new());
        }

        // Pull every candidate's pre-stamped set in one round-trip (or
        // two if a design filter is in play). The vector index isn't
        // applicable here so we just scan every Module node in scope.
        let cypher = match design {
            Some(_) => {
                "MATCH (m:Module) WHERE m.design = $d AND m.name <> $n \
                   AND m.port_set_card > 0 \
                 RETURN m.name AS name, m.port_set_json AS pj, m.port_set_card AS pc"
            }
            None => {
                "MATCH (m:Module) WHERE m.name <> $n AND m.port_set_card > 0 \
                 RETURN m.name AS name, m.port_set_json AS pj, m.port_set_card AS pc"
            }
        };
        let r = self.db.execute_cypher_with_params(cypher, cparam_d("n", module, design))?;
        let mut out = Vec::new();
        for row_v in r.rows() {
            let name = as_string(&row_v[0]);
            let cand_sorted = port::parse_port_set_json(&as_string(&row_v[1]));
            let cand_card = row_v[2].as_int64().unwrap_or(0);
            if cand_card == 0 {
                continue;
            }
            // Cardinality-only Jaccard upper bound: the ratio of the
            // smaller card over the larger card is an upper bound on
            // any possible Jaccard between the two sets. Skip work
            // when even that upper bound can't clear `min_overlap`.
            let lo = target_card.min(cand_card) as f64;
            let hi = target_card.max(cand_card) as f64;
            if hi <= 0.0 || lo / hi < min_overlap {
                continue;
            }

            let score = port::compute_jaccard_similarity(
                &target_sorted,
                &cand_sorted,
                target_card,
                cand_card,
            );

            if score >= min_overlap {
                let inter = port::sorted_intersection_count(&target_sorted, &cand_sorted);
                out.push(serde_json::json!({
                    "name": name,
                    "score": score,
                    "shared_ports": inter as i64,
                }));
            }
        }
        out.sort_by(|a, b| {
            b["score"]
                .as_f64()
                .unwrap_or(0.0)
                .partial_cmp(&a["score"].as_f64().unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    }

    pub fn stats(&self, design: Option<&str>) -> Result<GraphStats> {
        let modules = self.count_modules(design, false)?;
        let packages = self.count_modules(design, true)?;
        let instantiations = match design {
            Some(d) => self.count_with_param(
                "MATCH ()-[r:INSTANTIATES]->() WHERE r.design = $d RETURN count(r)",
                "d",
                d,
            )?,
            None => self.count_simple("MATCH ()-[r:INSTANTIATES]->() RETURN count(r)")?,
        };
        let imports = self.count_simple("MATCH ()-[r:IMPORTS]->() RETURN count(r)")?;
        let r = self
            .db
            .execute_cypher("MATCH (m:Module) RETURN m.design AS d, count(m) AS n ORDER BY d")?;
        let designs = r
            .rows()
            .iter()
            .map(|row| DesignStat {
                design: as_string(&row[0]),
                modules: row[1].as_int64().unwrap_or(0) as usize,
            })
            .collect();
        Ok(GraphStats {
            modules,
            packages,
            instantiations,
            imports,
            designs,
        })
    }

    // ------------------------------------------------------------------
    // Vector API
    // ------------------------------------------------------------------

    /// Stamp an embedding vector + model name on a module node. Uses
    /// Grafeo's typed `set_node_property` so the `:Module(embedding)`
    /// HNSW index auto-syncs without a rebuild.
    pub fn upsert_embedding(
        &self,
        design: &str,
        name: &str,
        vector: &[f32],
        model: &str,
    ) -> Result<()> {
        let key = module_key(design, name);
        let key_value = Value::from(key.as_str());
        let nodes = self.db.find_nodes_by_property("key", &key_value);
        let Some(node_id) = nodes.into_iter().next() else {
            return Err(StoreError::NotFound(format!(
                "Module {key} not present; cannot stamp embedding"
            )));
        };
        self.db
            .set_node_property(node_id, "embedding", Value::Vector(vector.to_vec().into()));
        self.db
            .set_node_property(node_id, "embedding_model", Value::from(model));
        Ok(())
    }

    /// HNSW top-k vector search with optional design filter.
    pub fn search_modules_by_vector(
        &self,
        query: &[f32],
        top_k: usize,
        design: Option<&str>,
    ) -> Result<Vec<VectorHit>> {
        let filters = design.map(|d| cparam("design", d));
        let hits =
            self.db
                .vector_search("Module", "embedding", query, top_k, None, filters.as_ref())?;
        let mut out = Vec::with_capacity(hits.len());
        for (node_id, dist) in hits {
            let Some(node) = self.db.get_node(node_id) else {
                continue;
            };
            let module = node
                .get_property("name")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            let design = node
                .get_property("design")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            // Cosine distance is in [0, 2]; convert to a [-1, 1] similarity
            // so callers can sort descending. Unit-norm vectors keep this
            // stable, but we don't enforce normalisation here.
            let score = (1.0 - dist * 0.5).clamp(-1.0, 1.0);
            out.push(VectorHit {
                module,
                design,
                score,
            });
        }
        Ok(out)
    }

    /// Drop the embedding properties for every module of `alias`. Cheap;
    /// `clear_design` already covers this when wiping a whole design.
    pub fn clear_embeddings_for_design(&self, alias: &str) -> Result<()> {
        self.db.execute_cypher_with_params(
            "MATCH (m:Module {design: $a}) REMOVE m.embedding, m.embedding_model",
            cparam("a", alias),
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn list_instance_edges_from(&self, parent: &str) -> Result<Vec<InstanceEdge>> {
        let parent_file = self.module_meta(parent)?.1;
        graph::list_instance_edges_from(&self.db, parent, &parent_file)
    }

    /// Return the file path for `name`, consulting `cache` first to avoid
    /// repeated round-trips when the same parent appears on many edges.
    fn cached_file(&self, name: &str, cache: &mut BTreeMap<String, String>) -> Result<String> {
        if let Some(f) = cache.get(name) {
            return Ok(f.clone());
        }
        let f = self.module_meta(name)?.1;
        cache.insert(name.to_string(), f.clone());
        Ok(f)
    }

    /// Fetch the `(design, file_path)` pair for `name` in a single
    /// round-trip.
    fn module_meta(&self, name: &str) -> Result<(String, String)> {
        let r = self.db.execute_cypher_with_params(
            "MATCH (m:Module {name: $n}) RETURN m.design, m.file_path",
            cparam("n", name),
        )?;
        Ok(r.rows()
            .first()
            .map(|row| (as_string(&row[0]), as_string(&row[1])))
            .unwrap_or_default())
    }

    fn count_modules(&self, design: Option<&str>, is_package: bool) -> Result<usize> {
        let cypher = match design {
            Some(_) => "MATCH (m:Module) WHERE m.is_package = $ip AND m.design = $d RETURN count(m)",
            None    => "MATCH (m:Module) WHERE m.is_package = $ip RETURN count(m)",
        };
        let r = self.db.execute_cypher_with_params(cypher, cparam_d("ip", is_package, design))?;
        Ok(r.rows()[0][0].as_int64().unwrap_or(0) as usize)
    }

    fn count_simple(&self, cypher: &str) -> Result<usize> {
        let r = self.db.execute_cypher(cypher)?;
        Ok(r.rows()[0][0].as_int64().unwrap_or(0) as usize)
    }

    fn count_with_param(&self, cypher: &str, k: &str, v: &str) -> Result<usize> {
        let r = self.db.execute_cypher_with_params(cypher, cparam(k, v))?;
        Ok(r.rows()[0][0].as_int64().unwrap_or(0) as usize)
    }
}

// =====================================================================
// Cypher mutation helpers (in-session, parameterised)
// =====================================================================

/// Build a single-entry Cypher parameter map.
pub(crate) fn cparam(key: &str, val: impl Into<Value>) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    m.insert(key.to_string(), val.into());
    m
}

/// Build a parameter map with an optional design filter (`$d`).
/// Always inserts `key → val`; adds `"d" → design` when `design` is `Some`.
fn cparam_d(key: &str, val: impl Into<Value>, design: Option<&str>) -> HashMap<String, Value> {
    let mut m = cparam(key, val);
    if let Some(d) = design {
        m.insert("d".to_string(), Value::from(d));
    }
    m
}

/// Compose the per-design Module identity. Stored on `m.key` and used as
/// the MERGE match key.
fn module_key(design: &str, name: &str) -> String {
    format!("{design}::{name}")
}

// Build a single `Value::Map` row from `(key, value)` pairs. The map is
// `Arc`'d once at the end so it ships as one heap allocation per row.
fn row(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut map: BTreeMap<PropertyKey, Value> = BTreeMap::new();
    for (k, v) in entries {
        map.insert(PropertyKey::new(k), v);
    }
    Value::Map(std::sync::Arc::new(map))
}

// Run `UNWIND $rows AS r <body>` in chunks of `chunk` rows. Empty input is
// a no-op. Each chunk is one Cypher round-trip.
fn unwind_in_chunks(session: &Session, cypher: &str, rows: Vec<Value>, chunk: usize) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let chunk = chunk.max(1);
    for slice in rows.chunks(chunk) {
        let params = std::collections::HashMap::from([(
            "rows".to_string(),
            Value::List(slice.to_vec().into()),
        )]);
        session.execute_language(cypher, "cypher", Some(params))?;
    }
    Ok(())
}

// Pass 1a: MERGE every full module node.
fn merge_module_nodes_batch(
    session: &Session,
    modules: &[&ModuleData],
    chunk: usize,
) -> Result<()> {
    let mut rows: Vec<Value> = Vec::with_capacity(modules.len());
    for m in modules {
        let ports_json = serde_json::to_string(&m.ports)?;
        let params_json = serde_json::to_string(&m.parameters)?;
        let imports_json = serde_json::to_string(&m.imports)?;
        let (pbs, pbe) = m.param_block_lines.unwrap_or((-1, -1));
        let (pos_, poe) = m.port_block_lines.unwrap_or((-1, -1));
        let key = module_key(&m.design, &m.name);
        let (port_set_json, port_set_card) = build_port_set(&m.ports);
        rows.push(row([
            ("k", Value::from(key)),
            ("name", Value::from(m.name.as_str())),
            ("fp", Value::from(m.file_path.as_str())),
            ("d", Value::from(m.design.as_str())),
            ("ip", Value::from(m.is_package)),
            ("ls", Value::from(m.line_start.unwrap_or(-1))),
            ("le", Value::from(m.line_end.unwrap_or(-1))),
            ("pbs", Value::from(pbs)),
            ("pbe", Value::from(pbe)),
            ("ps", Value::from(pos_)),
            ("pe", Value::from(poe)),
            ("desc", Value::from(m.description.as_deref().unwrap_or(""))),
            ("pj", Value::from(ports_json)),
            ("paj", Value::from(params_json)),
            ("ij", Value::from(imports_json)),
            ("psj", Value::from(port_set_json)),
            ("psc", Value::from(port_set_card)),
        ]));
    }
    unwind_in_chunks(
        session,
        "UNWIND $rows AS r \
         MERGE (m:Module {key: r.k}) \
         SET m.name = r.name, m.file_path = r.fp, m.design = r.d, \
             m.is_package = r.ip, m.line_start = r.ls, m.line_end = r.le, \
             m.param_block_start = r.pbs, m.param_block_end = r.pbe, \
             m.port_block_start = r.ps, m.port_block_end = r.pe, \
             m.description = r.desc, \
             m.ports_json = r.pj, m.params_json = r.paj, m.imports_json = r.ij, \
             m.port_set_json = r.psj, m.port_set_card = r.psc",
        rows,
        chunk,
    )
}

/// Build a sorted-dedup-normalized port name set as a compact JSON array
/// string plus its cardinality. Stored at upsert time so
/// `find_structurally_similar` doesn't have to reparse the full
/// `ports_json` for every candidate.
fn build_port_set(ports: &[PortInfo]) -> (String, i64) {
    let set: BTreeSet<String> = ports.iter().map(|p| normalize_port_name(&p.name)).collect();
    let card = set.len() as i64;
    let json = serde_json::to_string(&set.into_iter().collect::<Vec<_>>()).unwrap_or_default();
    (json, card)
}

// Pass 1b: MERGE every touched Design alias as a stub.
fn merge_design_stubs_batch(
    session: &Session,
    aliases: &BTreeSet<String>,
    chunk: usize,
) -> Result<()> {
    if aliases.is_empty() {
        return Ok(());
    }
    let rows: Vec<Value> = aliases
        .iter()
        .map(|a| row([("a", Value::from(a.as_str()))]))
        .collect();
    unwind_in_chunks(
        session,
        "UNWIND $rows AS r MERGE (d:Design {alias: r.a})",
        rows,
        chunk,
    )
}

// Pass 1c: MERGE every child Module stub (instantiation + import targets)
// once per `<design>::<name>` key.
fn merge_child_stubs_batch(session: &Session, modules: &[&ModuleData], chunk: usize) -> Result<()> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut rows: Vec<Value> = Vec::new();
    for m in modules {
        for inst in &m.instantiations {
            let key = module_key(&m.design, &inst.module_name);
            if seen.insert(key.clone()) {
                rows.push(row([
                    ("k", Value::from(key)),
                    ("name", Value::from(inst.module_name.as_str())),
                    ("d", Value::from(m.design.as_str())),
                ]));
            }
        }
        for imp in &m.imports {
            let key = module_key(&m.design, &imp.package_name);
            if seen.insert(key.clone()) {
                rows.push(row([
                    ("k", Value::from(key)),
                    ("name", Value::from(imp.package_name.as_str())),
                    ("d", Value::from(m.design.as_str())),
                ]));
            }
        }
    }
    unwind_in_chunks(
        session,
        "UNWIND $rows AS r \
         MERGE (m:Module {key: r.k}) \
         ON CREATE SET m.name = r.name, m.design = r.d",
        rows,
        chunk,
    )
}

// Pass 2a: CREATE every INSTANTIATES edge (one row per call site;
// parallel edges preserved).
fn create_instantiates_batch(
    session: &Session,
    modules: &[&ModuleData],
    chunk: usize,
) -> Result<()> {
    let mut rows: Vec<Value> = Vec::new();
    for m in modules {
        let parent_key = module_key(&m.design, &m.name);
        for inst in &m.instantiations {
            let child_key = module_key(&m.design, &inst.module_name);
            rows.push(row([
                ("pk", Value::from(parent_key.as_str())),
                ("ck", Value::from(child_key)),
                ("inst", Value::from(inst.instance_name.as_str())),
                ("d", Value::from(m.design.as_str())),
                (
                    "pb",
                    Value::from(serde_json::to_string(&inst.param_bindings)?),
                ),
                (
                    "ob",
                    Value::from(serde_json::to_string(&inst.port_bindings)?),
                ),
                (
                    "rpv",
                    Value::from(serde_json::to_string(&inst.resolved_param_values)?),
                ),
                (
                    "rpw",
                    Value::from(serde_json::to_string(&inst.resolved_port_widths)?),
                ),
                ("ls", Value::from(inst.line_start.unwrap_or(-1))),
                ("le", Value::from(inst.line_end.unwrap_or(-1))),
            ]));
        }
    }
    unwind_in_chunks(
        session,
        "UNWIND $rows AS r \
         MATCH (p:Module {key: r.pk}), (c:Module {key: r.ck}) \
         CREATE (p)-[:INSTANTIATES { \
            instance_name: r.inst, design: r.d, \
            param_bindings_json: r.pb, port_bindings_json: r.ob, \
            resolved_param_values_json: r.rpv, \
            resolved_port_widths_json: r.rpw, \
            line_start: r.ls, line_end: r.le }]->(c)",
        rows,
        chunk,
    )
}

// Pass 2b: CREATE every IMPORTS edge.
fn create_imports_batch(session: &Session, modules: &[&ModuleData], chunk: usize) -> Result<()> {
    let mut rows: Vec<Value> = Vec::new();
    for m in modules {
        let parent_key = module_key(&m.design, &m.name);
        for imp in &m.imports {
            let pkg_key = module_key(&m.design, &imp.package_name);
            rows.push(row([
                ("pk", Value::from(parent_key.as_str())),
                ("ck", Value::from(pkg_key)),
                ("wc", Value::from(imp.is_wildcard)),
                (
                    "syms",
                    Value::from(serde_json::to_string(&imp.specific_symbols)?),
                ),
            ]));
        }
    }
    unwind_in_chunks(
        session,
        "UNWIND $rows AS r \
         MATCH (p:Module {key: r.pk}), (c:Module {key: r.ck}) \
         CREATE (p)-[:IMPORTS { is_wildcard: r.wc, specific_symbols_json: r.syms }]->(c)",
        rows,
        chunk,
    )
}

// Pass 2c: CREATE every BELONGS_TO edge.
fn create_belongs_to_batch(session: &Session, modules: &[&ModuleData], chunk: usize) -> Result<()> {
    let rows: Vec<Value> = modules
        .iter()
        .filter(|m| !m.design.is_empty())
        .map(|m| {
            row([
                ("pk", Value::from(module_key(&m.design, &m.name))),
                ("a", Value::from(m.design.as_str())),
            ])
        })
        .collect();
    unwind_in_chunks(
        session,
        "UNWIND $rows AS r \
         MATCH (p:Module {key: r.pk}), (d:Design {alias: r.a}) \
         CREATE (p)-[:BELONGS_TO]->(d)",
        rows,
        chunk,
    )
}

// =====================================================================
// Row parsing
// =====================================================================

pub(crate) fn as_string(v: &Value) -> String {
    v.as_str().map(|s| s.to_string()).unwrap_or_default()
}

fn as_bool_opt(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| v.as_int64().map(|i| i != 0))
}

pub(crate) fn as_i64_or_none(v: &Value) -> Option<i64> {
    match v {
        Value::Null => None,
        _ => v.as_int64(),
    }
}

fn row_is_stub(row: &[Value]) -> bool {
    // file_path / design / ports_json all empty/null => never given a full upsert.
    row.get(1).is_none_or(is_empty_str)
        && row.get(2).is_none_or(is_empty_str)
        && row.get(11).is_none_or(is_empty_str)
}

fn is_empty_str(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

fn row_to_module(row: &[Value]) -> Result<ModuleData> {
    let name = as_string(&row[0]);
    let file_path = as_string(&row[1]);
    let design = as_string(&row[2]);
    let is_package = as_bool_opt(&row[3]).unwrap_or(false);
    let line_start = as_i64_or_none(&row[4]);
    let line_end = as_i64_or_none(&row[5]);
    let pbs = row[6].as_int64().unwrap_or(-1);
    let pbe = row[7].as_int64().unwrap_or(-1);
    let pos_ = row[8].as_int64().unwrap_or(-1);
    let poe = row[9].as_int64().unwrap_or(-1);
    let description = {
        let s = as_string(&row[10]);
        if s.is_empty() { None } else { Some(s) }
    };
    Ok(ModuleData {
        name,
        file_path,
        design,
        is_package,
        line_start: line_start.filter(|v| *v >= 0),
        line_end: line_end.filter(|v| *v >= 0),
        param_block_lines: if pbs >= 0 && pbe >= 0 {
            Some((pbs, pbe))
        } else {
            None
        },
        port_block_lines: if pos_ >= 0 && poe >= 0 {
            Some((pos_, poe))
        } else {
            None
        },
        parameters: serde_json::from_str(&as_string(&row[12])).unwrap_or_default(),
        ports: serde_json::from_str(&as_string(&row[11])).unwrap_or_default(),
        instantiations: Vec::new(),
        imports: serde_json::from_str(&as_string(&row[13])).unwrap_or_default(),
        includes: Vec::new(),
        exported_typedefs: Vec::new(),
        description,
    })
}

pub(crate) fn decode_json<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    if s.trim().is_empty() {
        return None;
    }
    serde_json::from_str(s).ok()
}

fn normalize_port_name(name: &str) -> String {
    let mut s = name.to_lowercase();
    for prefix in &["i_", "o_", "io_", "in_", "out_", "inout_"] {
        if s.starts_with(prefix) {
            s = s[prefix.len()..].to_string();
            break;
        }
    }
    for suffix in &["_i", "_o", "_io", "_in", "_out"] {
        if s.ends_with(suffix) {
            s = s[..s.len() - suffix.len()].to_string();
            break;
        }
    }
    s
}

fn now_unix_ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bender_kg_models::{Direction, InstantiationInfo, ParamInfo, ParamKind};
    use tempfile::tempdir;

    fn make_module(name: &str, design: &str) -> ModuleData {
        let mut m = ModuleData::default();
        m.name = name.into();
        m.design = design.into();
        m.is_package = false;
        m.line_start = Some(1);
        m.line_end = Some(10);
        m
    }

    #[test]
    fn store_round_trip_module_and_instantiation() -> Result<()> {
        let tmp = tempdir().unwrap();
        let cfg = StoreConfig::new(tmp.path());
        let store = Store::open(&cfg)?;
        let mut parent = make_module("parent", "d1");
        parent.parameters.push(ParamInfo {
            name: "WIDTH".into(),
            kind: ParamKind::Int,
            default_value: "32".into(),
            is_type_param: false,
        });
        parent.ports.push(PortInfo {
            name: "clk".into(),
            direction: Direction::Input,
            type_str: "logic".into(),
            ..Default::default()
        });
        let mut inst0 = InstantiationInfo {
            module_name: "child".into(),
            instance_name: "u_child0".into(),
            ..Default::default()
        };
        inst0
            .param_bindings
            .insert("WIDTH".into(), "AddrWidth".into());
        inst0
            .resolved_param_values
            .insert("WIDTH".into(), "32'd32".into());
        inst0.resolved_port_widths.insert(
            "apb_req".into(),
            ResolvedPortWidth {
                total: 32,
                fields: BTreeMap::from([("foo".into(), 16i64), ("bar.baz".into(), 16i64)]),
                ..Default::default()
            },
        );
        inst0.resolved_port_widths.insert(
            "req_arr".into(),
            ResolvedPortWidth {
                total: 64,
                element_count: Some(4),
                element: Some(Box::new(ResolvedPortWidth {
                    total: 16,
                    fields: BTreeMap::from([("paddr".into(), 16i64)]),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        parent.instantiations.push(inst0);
        // Second parallel instantiation of `child` — Grafeo keeps it as a
        // distinct edge.
        parent.instantiations.push(InstantiationInfo {
            module_name: "child".into(),
            instance_name: "u_child1".into(),
            ..Default::default()
        });
        let child = make_module("child", "d1");
        store.register_design("d1", "ID1", None, None, &["rtl".to_string()], &[])?;
        store.upsert_modules([&parent, &child])?;
        let got = store.get_module("parent")?.unwrap();
        assert_eq!(got.parameters.len(), 1);
        assert_eq!(got.ports.len(), 1);
        let parents = store.get_parents("child")?;
        assert_eq!(parents.len(), 1);
        let stats = store.stats(None)?;
        assert_eq!(stats.modules, 2);
        let ctx = store.get_instance_context("parent", "child")?;
        let names: BTreeSet<&str> = ctx.iter().map(|e| e.instance_name.as_str()).collect();
        assert!(names.contains("u_child0") && names.contains("u_child1"));
        let edge0 = ctx.iter().find(|e| e.instance_name == "u_child0").unwrap();
        assert_eq!(
            edge0.resolved_param_values.get("WIDTH"),
            Some(&"32'd32".to_string())
        );
        let pw = edge0.resolved_port_widths.get("apb_req").unwrap();
        assert_eq!(pw.total, 32);
        assert_eq!(pw.fields.get("foo"), Some(&16));
        assert_eq!(pw.fields.get("bar.baz"), Some(&16));
        assert!(pw.element_count.is_none() && pw.element.is_none());
        let arr = edge0.resolved_port_widths.get("req_arr").unwrap();
        assert_eq!(arr.total, 64);
        assert_eq!(arr.element_count, Some(4));
        let elem = arr.element.as_deref().unwrap();
        assert_eq!(elem.total, 16);
        assert_eq!(elem.fields.get("paddr"), Some(&16));
        Ok(())
    }

    /// Two parallel `INSTANTIATES` edges from `parent` to `child` with
    /// different bindings must round-trip as two distinct edges.
    #[test]
    fn parallel_instantiations_round_trip_as_separate_edges() -> Result<()> {
        let tmp = tempdir().unwrap();
        let cfg = StoreConfig::new(tmp.path());
        let store = Store::open(&cfg)?;
        let mut parent = make_module("parent", "d1");
        let mut a = InstantiationInfo {
            module_name: "child".into(),
            instance_name: "u_lo".into(),
            ..Default::default()
        };
        a.param_bindings.insert("AW".into(), "32".into());
        let mut b = InstantiationInfo {
            module_name: "child".into(),
            instance_name: "u_hi".into(),
            ..Default::default()
        };
        b.param_bindings.insert("AW".into(), "64".into());
        parent.instantiations.extend([a, b]);
        let child = make_module("child", "d1");
        store.register_design("d1", "ID1", None, None, &[], &[])?;
        store.upsert_modules([&parent, &child])?;
        let ctx = store.get_instance_context("parent", "child")?;
        assert_eq!(ctx.len(), 2);
        let lo = ctx.iter().find(|e| e.instance_name == "u_lo").unwrap();
        let hi = ctx.iter().find(|e| e.instance_name == "u_hi").unwrap();
        assert_eq!(lo.param_bindings.get("AW"), Some(&"32".to_string()));
        assert_eq!(hi.param_bindings.get("AW"), Some(&"64".to_string()));
        Ok(())
    }

    #[test]
    fn trace_parameter_emits_call_site_expression() -> Result<()> {
        let tmp = tempdir().unwrap();
        let cfg = StoreConfig::new(tmp.path());
        let store = Store::open(&cfg)?;
        let mut parent = make_module("parent", "d1");
        parent.parameters.push(ParamInfo {
            name: "WIDTH".into(),
            kind: ParamKind::Int,
            default_value: "32".into(),
            is_type_param: false,
        });
        let mut inst = InstantiationInfo {
            module_name: "child".into(),
            instance_name: "u_child".into(),
            ..Default::default()
        };
        // child.AW gets parent.WIDTH (expression = "WIDTH", binds to parent's WIDTH param)
        inst.param_bindings
            .insert("AW".into(), "WIDTH".into());
        inst.resolved_param_values
            .insert("AW".into(), "32'd32".into());
        parent.instantiations.push(inst);
        let child = make_module("child", "d1");
        store.register_design("d1", "ID1", None, None, &["rtl".to_string()], &[])?;
        store.upsert_modules([&parent, &child])?;

        let rows = store.trace_parameter("parent", "WIDTH")?;
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["call_site_expression"], "WIDTH");
        assert_eq!(row["resolved_value"], "32'd32");
        // Old field name must not leak.
        assert!(row.get("bound_value").is_none());
        Ok(())
    }

    #[test]
    fn cross_design_modules_with_same_name_are_isolated() -> Result<()> {
        let tmp = tempdir().unwrap();
        let cfg = StoreConfig::new(tmp.path());
        let store = Store::open(&cfg)?;
        store.register_design("d1", "ID1", None, None, &[], &[])?;
        store.register_design("d2", "ID2", None, None, &[], &[])?;
        store.upsert_module(&make_module("axi_pkg", "d1"))?;
        store.upsert_module(&make_module("axi_pkg", "d2"))?;
        assert_eq!(store.stats(Some("d1"))?.modules, 1);
        assert_eq!(store.stats(Some("d2"))?.modules, 1);
        store.clear_design("d1")?;
        assert_eq!(store.stats(Some("d1"))?.modules, 0);
        assert_eq!(store.stats(Some("d2"))?.modules, 1);
        Ok(())
    }

    #[test]
    fn embedding_round_trip() -> Result<()> {
        let tmp = tempdir().unwrap();
        let cfg = StoreConfig::new(tmp.path()).with_embedding_dim(4);
        let store = Store::open(&cfg)?;
        store.register_design("d1", "ID1", None, None, &[], &[])?;
        let m1 = make_module("a", "d1");
        let m2 = make_module("b", "d1");
        store.upsert_modules([&m1, &m2])?;
        store.upsert_embedding("d1", "a", &[1.0, 0.0, 0.0, 0.0], "test-model")?;
        store.upsert_embedding("d1", "b", &[0.0, 1.0, 0.0, 0.0], "test-model")?;
        let hits = store.search_modules_by_vector(&[1.0, 0.0, 0.0, 0.0], 2, None)?;
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(hits[0].module, "a");
        Ok(())
    }
}

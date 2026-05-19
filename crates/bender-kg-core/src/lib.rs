// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Knowledge-graph orchestrator. Composes extraction with the Grafeo
//! store (graph + HNSW vectors + BM25 text index) into a typed API used
//! by both the `bender kg` CLI and the MCP adapter.
//!
//! The `Engine` surface area is split across focused modules:
//! * [`build`]   — extraction + ingest into the Grafeo store.
//! * [`query`]   — sync graph reads (modules, hierarchy, structural
//!   analysis).
//! * [`search`]  — vector-aware semantic search.
//! * [`snippet`] — file-system reads of source-line ranges.
//!
//! Public methods stay `async fn` for back-compat with the MCP adapter
//! and CLI runtime even though Grafeo's surface is synchronous; the
//! bodies are now non-awaiting.

mod build;
mod query;
mod search;
mod snippet;

use std::path::PathBuf;

use bender_kg_extract::SourceGroupInput;
use bender_kg_models::{BuildPhases, Manifest, ModuleData};
use bender_kg_similarity::{Embedder, build as build_embedder};
use bender_kg_store::{Store, StoreConfig};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use bender_kg_store::{DesignStat, GraphStats, InstanceEdge, Subgraph, VectorHit};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("extract error: {0}")]
    Extract(#[from] bender_kg_extract::ExtractError),
    #[error("store error: {0}")]
    Store(#[from] bender_kg_store::StoreError),
    #[error("embed error: {0}")]
    Embed(#[from] bender_kg_similarity::EmbedError),
    #[error("models error: {0}")]
    Models(#[from] bender_kg_models::ModelsError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// Directory holding all kg artifacts, typically `<workspace>/.bender-kg/`.
    pub root: PathBuf,
    pub embed: bender_kg_similarity::EmbedConfig,
    /// Skip the embedding/index step (faster builds, disables search).
    pub skip_embeddings: bool,
    /// Maximum rows per UNWIND batch in the store's `upsert_modules`. Larger
    /// = fewer Cypher round-trips, more memory per call. Defaults to
    /// [`bender_kg_store::DEFAULT_UPSERT_CHUNK_SIZE`].
    pub upsert_chunk_size: usize,
    /// Overlap slang's `walk_elaborated` with the base graph upsert when
    /// `inputs.elab` is on. Default `true`. Set to `false` to fall back
    /// to the simpler sequential path (mostly useful for debugging).
    pub pipeline_elab: bool,
}

impl CoreConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            embed: bender_kg_similarity::EmbedConfig::default(),
            skip_embeddings: false,
            upsert_chunk_size: bender_kg_store::DEFAULT_UPSERT_CHUNK_SIZE,
            pipeline_elab: true,
        }
    }
    pub fn ir_path(&self) -> PathBuf {
        self.root.join("ir.jsonl")
    }
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }
    pub fn store_config(&self, dim: usize) -> StoreConfig {
        StoreConfig::new(&self.root)
            .with_embedding_dim(dim)
            .with_upsert_chunk_size(self.upsert_chunk_size)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildOutcome {
    pub manifest: Manifest,
    pub modules_indexed: usize,
    pub embeddings_indexed: usize,
    /// Wall-clock breakdown of the build's major phases. Defaults to all
    /// zeros for callers (e.g. `index_from_jsonl`) that don't measure.
    /// Consumed by the `bender kg build` JSON summary; can be ignored.
    #[serde(default)]
    pub phases: BuildPhases,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSearchResult {
    pub name: String,
    pub score: f32,
    pub file_path: String,
    pub design: String,
    pub description: String,
    pub num_ports: usize,
    pub num_params: usize,
    pub num_instantiations: usize,
}

pub struct Engine {
    pub(crate) cfg: CoreConfig,
    pub(crate) store: Store,
    pub(crate) embedder: Box<dyn Embedder>,
}

impl Engine {
    /// Open or create the engine artifacts under `cfg.root`. The signature
    /// stays async for back-compat with the MCP adapter and CLI; the body
    /// is synchronous because Grafeo is sync end-to-end.
    pub async fn open(cfg: CoreConfig) -> Result<Self> {
        std::fs::create_dir_all(&cfg.root)?;
        let embedder = build_embedder(&cfg.embed)?;
        let store = Store::open(&cfg.store_config(embedder.dim()))?;
        Ok(Self {
            cfg,
            store,
            embedder,
        })
    }

    pub fn config(&self) -> &CoreConfig {
        &self.cfg
    }

    pub fn store(&self) -> &Store {
        &self.store
    }
}

/// Compose the document string used as the embedding input for a module.
pub(crate) fn module_document(m: &ModuleData) -> String {
    let mut parts = vec![format!("module {}", m.name)];
    if !m.file_path.is_empty() {
        parts.push(format!("path {}", m.file_path));
    }
    if !m.parameters.is_empty() {
        let plist: Vec<&str> = m.parameters.iter().map(|p| p.name.as_str()).collect();
        parts.push(format!("params {}", plist.join(" ")));
    }
    if !m.ports.is_empty() {
        let plist: Vec<&str> = m.ports.iter().map(|p| p.name.as_str()).collect();
        parts.push(format!("ports {}", plist.join(" ")));
    }
    if let Some(desc) = &m.description {
        if !desc.is_empty() {
            parts.push(desc.clone());
        }
    }
    parts.join(" ")
}

/// Convenience: build a [`SourceGroupInput`] from flat lists.
pub fn one_group(
    files: Vec<String>,
    includes: Vec<String>,
    defines: Vec<String>,
) -> SourceGroupInput {
    SourceGroupInput {
        files,
        include_dirs: includes,
        defines,
    }
}

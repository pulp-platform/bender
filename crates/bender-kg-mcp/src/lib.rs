// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Stdio MCP adapter for `bender kg`, built on the official Rust SDK
//! (`rmcp`). Exposes the kg query surface as MCP tools.

use std::sync::Arc;

use bender_kg_core::{CoreConfig, Engine};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

const SERVER_NAME: &str = "bender-kg";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTRUCTIONS: &str =
    "RTL knowledge graph: search, browse, and query SystemVerilog module data.";

// =====================================================================
// Param structs (one per tool, all derive serde + schemars)
// =====================================================================

fn d_5_i32() -> i32 {
    5
}
fn d_15() -> usize {
    15
}
fn d_10() -> usize {
    10
}
fn d_3_i32() -> i32 {
    3
}
fn d_1_i32() -> i32 {
    1
}
fn d_1_usize() -> usize {
    1
}
fn d_overlap() -> f64 {
    0.3
}
fn d_module() -> String {
    "module".into()
}
fn d_input() -> String {
    "input".into()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchModulesParams {
    /// Natural-language description.
    pub query: String,
    #[serde(default = "d_15")]
    pub top_k: usize,
    /// Restrict search to a design alias (empty for all).
    #[serde(default)]
    pub design: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchModulesBatchParams {
    pub queries: Vec<String>,
    #[serde(default = "d_10")]
    pub top_k: usize,
    #[serde(default)]
    pub design: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NameParams {
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSubgraphParams {
    pub name: String,
    #[serde(default = "d_3_i32")]
    pub depth: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetInstanceContextParams {
    pub parent: String,
    pub child: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindByProtocolParams {
    pub protocol: String,
    #[serde(default)]
    pub design: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSourceSnippetParams {
    pub module_name: String,
    #[serde(default = "d_module")]
    pub element: String,
    #[serde(default)]
    pub instance_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TraceHierarchyPathParams {
    pub from_module: String,
    pub to_module: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PropagatePortParams {
    pub from_module: String,
    pub to_module: String,
    pub signal_name: String,
    #[serde(default = "d_input")]
    pub direction: String,
    #[serde(default = "d_1_usize")]
    pub width: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckConnectivityParams {
    pub module_name: String,
    #[serde(default = "d_1_i32")]
    pub depth: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TraceParameterParams {
    pub module_name: String,
    pub param_name: String,
    /// Follow parameter propagation recursively through the hierarchy.
    #[serde(default)]
    pub recursive: bool,
    /// Maximum recursion depth (only active when recursive=true).
    #[serde(default = "d_5_i32")]
    pub max_depth: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TraceSignalParams {
    pub module_name: String,
    pub signal_name: String,
    /// Follow signal connections recursively through the hierarchy.
    #[serde(default)]
    pub recursive: bool,
    /// Maximum recursion depth (only active when recursive=true).
    #[serde(default = "d_5_i32")]
    pub max_depth: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MatchInterfacesParams {
    pub module_a: String,
    pub module_b: String,
    #[serde(default)]
    pub prefix_a: String,
    #[serde(default)]
    pub prefix_b: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindStructurallySimilarParams {
    pub module_name: String,
    #[serde(default = "d_overlap")]
    pub min_overlap: f64,
    #[serde(default)]
    pub design: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GraphStatsParams {
    #[serde(default)]
    pub design: String,
}

// =====================================================================
// Server impl
// =====================================================================

#[derive(Clone)]
pub struct BenderKg {
    engine: Arc<Engine>,
    // The `#[tool_router]` macro reads this field via generated code that
    // looks invisible to rustc's dead-code analysis.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl BenderKg {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Semantic search for RTL modules by natural-language description.")]
    async fn search_modules(
        &self,
        Parameters(p): Parameters<SearchModulesParams>,
    ) -> Result<String, McpError> {
        let hits = self
            .engine
            .search_modules(&p.query, p.top_k, opt(&p.design))
            .await
            .map_err(internal)?;
        as_json(&hits)
    }

    #[tool(description = "Run multiple semantic searches in one call (deduplicated by name).")]
    async fn search_modules_batch(
        &self,
        Parameters(p): Parameters<SearchModulesBatchParams>,
    ) -> Result<String, McpError> {
        let hits = self
            .engine
            .search_modules_batch(&p.queries, p.top_k, opt(&p.design))
            .await
            .map_err(internal)?;
        as_json(&hits)
    }

    #[tool(description = "Get full details for a single RTL module by exact name.")]
    async fn get_module(&self, Parameters(p): Parameters<NameParams>) -> Result<String, McpError> {
        match self.engine.get_module(&p.name).map_err(internal)? {
            Some(m) => as_json(&m),
            None => {
                as_json(&serde_json::json!({"error": format!("Module '{}' not found", p.name)}))
            }
        }
    }

    #[tool(description = "Get the instantiation tree rooted at a module (depth-limited BFS).")]
    async fn get_subgraph(
        &self,
        Parameters(p): Parameters<GetSubgraphParams>,
    ) -> Result<String, McpError> {
        let sg = self
            .engine
            .get_subgraph(&p.name, p.depth)
            .map_err(internal)?;
        as_json(&sg)
    }

    #[tool(
        description = "Get resolved parameter bindings and port widths for a (parent,child) edge."
    )]
    async fn get_instance_context(
        &self,
        Parameters(p): Parameters<GetInstanceContextParams>,
    ) -> Result<String, McpError> {
        let edges = self
            .engine
            .get_instance_context(&p.parent, &p.child)
            .map_err(internal)?;
        if edges.is_empty() {
            return as_json(&serde_json::json!({
                "error": format!("No INSTANTIATES edge from '{}' to '{}'", p.parent, p.child),
            }));
        }
        as_json(&edges)
    }

    #[tool(
        description = "Find all modules that instantiate the given module (reverse dependency)."
    )]
    async fn get_parents(&self, Parameters(p): Parameters<NameParams>) -> Result<String, McpError> {
        let parents = self.engine.get_parents(&p.name).map_err(internal)?;
        as_json(&parents)
    }

    #[tool(description = "Find all distinct module types directly instantiated by the given module.")]
    async fn get_children(
        &self,
        Parameters(p): Parameters<NameParams>,
    ) -> Result<String, McpError> {
        let children = self.engine.get_children(&p.name).map_err(internal)?;
        as_json(&children)
    }

    #[tool(description = "Get just the port list for a module (lightweight query).")]
    async fn get_ports(&self, Parameters(p): Parameters<NameParams>) -> Result<String, McpError> {
        match self.engine.get_module(&p.name).map_err(internal)? {
            Some(m) => as_json(&m.ports),
            None => {
                as_json(&serde_json::json!({"error": format!("Module '{}' not found", p.name)}))
            }
        }
    }

    #[tool(description = "Find modules whose port types contain a protocol keyword (e.g. 'axi').")]
    async fn find_by_protocol(
        &self,
        Parameters(p): Parameters<FindByProtocolParams>,
    ) -> Result<String, McpError> {
        let mods = self
            .engine
            .find_by_protocol(&p.protocol, opt(&p.design))
            .map_err(internal)?;
        let summarised: Vec<Value> = mods
            .iter()
            .map(|m| serde_json::json!({"name": m.name, "file_path": m.file_path}))
            .collect();
        as_json(&summarised)
    }

    #[tool(
        description = "Read targeted source lines for a module element (module/ports/params/instance)."
    )]
    async fn get_source_snippet(
        &self,
        Parameters(p): Parameters<GetSourceSnippetParams>,
    ) -> Result<String, McpError> {
        let v = self
            .engine
            .get_source_snippet(&p.module_name, &p.element, &p.instance_name)
            .map_err(internal)?;
        as_json(&v)
    }

    #[tool(description = "Trace the BFS path between two modules and return hop metadata.")]
    async fn trace_hierarchy_path(
        &self,
        Parameters(p): Parameters<TraceHierarchyPathParams>,
    ) -> Result<String, McpError> {
        let chain = self
            .engine
            .trace_hierarchy_path(&p.from_module, &p.to_module)
            .map_err(internal)?;
        if chain.is_empty() {
            return as_json(&serde_json::json!({
                "error": format!("No path from '{}' to '{}'", p.from_module, p.to_module),
            }));
        }
        as_json(&chain)
    }

    #[tool(description = "Generate an edit plan for propagating a new port through the hierarchy.")]
    async fn propagate_port(
        &self,
        Parameters(p): Parameters<PropagatePortParams>,
    ) -> Result<String, McpError> {
        let chain = self
            .engine
            .trace_hierarchy_path(&p.from_module, &p.to_module)
            .map_err(internal)?;
        if chain.is_empty() {
            return as_json(&serde_json::json!({
                "error": format!("No path from '{}' to '{}'", p.from_module, p.to_module),
            }));
        }
        as_json(&serde_json::json!({
            "signal": p.signal_name,
            "direction": p.direction,
            "width": p.width,
            "from": p.from_module,
            "to": p.to_module,
            "hops": chain,
        }))
    }

    #[tool(description = "Run structural connectivity checks on instantiations under a module.")]
    async fn check_connectivity(
        &self,
        Parameters(p): Parameters<CheckConnectivityParams>,
    ) -> Result<String, McpError> {
        let findings = self
            .engine
            .check_connectivity(&p.module_name, p.depth)
            .map_err(internal)?;
        as_json(&serde_json::json!({
            "module": p.module_name,
            "depth": p.depth,
            "issue_count": findings.len(),
            "findings": findings,
        }))
    }

    #[tool(description = "Trace cascading impact of a parameter through the hierarchy. Set recursive=true to follow across multiple levels.")]
    async fn trace_parameter(
        &self,
        Parameters(p): Parameters<TraceParameterParams>,
    ) -> Result<String, McpError> {
        let res = if p.recursive {
            self.engine
                .trace_parameter_recursive(&p.module_name, &p.param_name, p.max_depth)
                .map_err(internal)?
        } else {
            self.engine
                .trace_parameter(&p.module_name, &p.param_name)
                .map_err(internal)?
        };
        as_json(&serde_json::json!({
            "module": p.module_name,
            "parameter": p.param_name,
            "recursive": p.recursive,
            "max_depth": p.max_depth,
            "affected_instances": res.len(),
            "instances": res,
        }))
    }

    #[tool(description = "Trace where a signal (port) from a module is connected in child instantiations. Set recursive=true to follow across multiple levels.")]
    async fn trace_signal(
        &self,
        Parameters(p): Parameters<TraceSignalParams>,
    ) -> Result<String, McpError> {
        let res = if p.recursive {
            self.engine
                .trace_signal_recursive(&p.module_name, &p.signal_name, p.max_depth)
                .map_err(internal)?
        } else {
            self.engine
                .trace_signal(&p.module_name, &p.signal_name)
                .map_err(internal)?
        };
        as_json(&serde_json::json!({
            "module": p.module_name,
            "signal": p.signal_name,
            "recursive": p.recursive,
            "max_depth": p.max_depth,
            "connections": res.len(),
            "instances": res,
        }))
    }

    #[tool(description = "Compare the port interfaces of two modules for wiring compatibility.")]
    async fn match_interfaces(
        &self,
        Parameters(p): Parameters<MatchInterfacesParams>,
    ) -> Result<String, McpError> {
        let v = self
            .engine
            .match_interfaces(&p.module_a, &p.module_b, &p.prefix_a, &p.prefix_b)
            .map_err(internal)?;
        as_json(&v)
    }

    #[tool(description = "Find modules with structurally similar port signatures (Jaccard).")]
    async fn find_structurally_similar(
        &self,
        Parameters(p): Parameters<FindStructurallySimilarParams>,
    ) -> Result<String, McpError> {
        let res = self
            .engine
            .find_structurally_similar(&p.module_name, p.min_overlap, opt(&p.design))
            .map_err(internal)?;
        as_json(&serde_json::json!({"module": p.module_name, "candidates": res}))
    }

    #[tool(description = "Return basic statistics about the graph (counts + per-design).")]
    async fn graph_stats(
        &self,
        Parameters(p): Parameters<GraphStatsParams>,
    ) -> Result<String, McpError> {
        let stats = self.engine.stats(opt(&p.design)).map_err(internal)?;
        as_json(&stats)
    }
}

#[tool_handler]
impl ServerHandler for BenderKg {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo / Implementation are #[non_exhaustive], so we fill in
        // a Default and assign through field access rather than literal
        // construction.
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info.name = SERVER_NAME.into();
        info.server_info.version = SERVER_VERSION.into();
        info.server_info.title = Some("Bender RTL Knowledge Graph".into());
        info.instructions = Some(INSTRUCTIONS.into());
        info
    }
}

// =====================================================================
// Entry point + helpers
// =====================================================================

/// Run the stdio MCP server until the peer disconnects.
pub async fn serve_stdio(cfg: CoreConfig) -> anyhow::Result<()> {
    let engine = Engine::open(cfg).await?;
    let server = BenderKg::new(Arc::new(engine));
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn opt(s: &str) -> Option<&str> {
    if s.is_empty() { None } else { Some(s) }
}

fn as_json<T: serde::Serialize>(v: &T) -> Result<String, McpError> {
    serde_json::to_string(v).map_err(|e| McpError::internal_error(format!("serde: {e}"), None))
}

fn internal(e: bender_kg_core::CoreError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn server_advertises_full_tool_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.keep();
        let cfg = CoreConfig::new(path);
        let engine = Engine::open(cfg).await.unwrap();
        let server = BenderKg::new(Arc::new(engine));
        // The tool router carries one route per #[tool]-annotated method.
        assert!(server.tool_router.list_all().len() >= 18);
    }
}

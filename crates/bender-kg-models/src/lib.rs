// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! `kg.v3` intermediate representation for the bender knowledge graph.
//!
//! Every public record is `serde`-serialisable as JSON; the streaming on-disk
//! form is JSONL with one [`IrRecord`] per line, prefixed by a single
//! [`Manifest`] envelope record.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

/// Schema version emitted by this crate. Bump when wire-incompatible changes
/// are made.
pub const KG_SCHEMA_VERSION: &str = "kg.v3";

/// Wall-clock breakdown of a `bender kg build` invocation.
///
/// Populated incrementally: the extract crate fills in
/// `slang_parse_*` / `walk_design_s` / `elaborate_s` / `ir_write_s`;
/// `bender-kg-core::Engine::build` fills in `store_upsert_s`, `embed_s`,
/// and `total_s` before returning. Default value is all-zero (so callers
/// that don't care can ignore it).
///
/// Removable: this struct is only consumed by the JSON summary of
/// `bender kg build` and the bench script; deleting it just removes the
/// `phases_seconds` block from the build output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildPhases {
    pub slang_parse_s: f64,
    pub slang_parse_group_count: usize,
    pub slang_parse_max_group_s: f64,
    /// Wall-clock spent pruning the parsed-tree set to those reachable from
    /// the requested top modules. Reported separately from `walk_design_s`
    /// so callers can attribute the cost of `reachable_tree_indices` +
    /// `retain_trees`.
    pub prune_s: f64,
    pub walk_design_s: f64,
    pub elaborate_s: f64,
    pub ir_write_s: f64,
    pub store_upsert_s: f64,
    pub embed_s: f64,
    pub total_s: f64,
}

#[derive(Debug, Error)]
pub enum ModelsError {
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ModelsError>;

/// Port direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Input,
    Output,
    Inout,
    Ref,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Input
    }
}

/// Parameter kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamKind {
    Int,
    Bit,
    Type,
    String,
    Other,
}

impl Default for ParamKind {
    fn default() -> Self {
        ParamKind::Other
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortInfo {
    pub name: String,
    pub direction: Direction,
    #[serde(default)]
    pub type_str: String,
    #[serde(default)]
    pub width_expr: String,
    #[serde(default)]
    pub bit_width: Option<i64>,
    #[serde(default)]
    pub is_type_param: bool,
    #[serde(default)]
    pub type_ref: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParamInfo {
    pub name: String,
    pub kind: ParamKind,
    #[serde(default)]
    pub default_value: String,
    #[serde(default)]
    pub is_type_param: bool,
}

/// Resolved bit width of a port, including a per-subfield breakdown when
/// the port's type is a packed struct or union. `total` is the canonical
/// `getBitWidth()` of the port type; `fields` is dot-flattened across
/// nested packed structs/unions and is empty for scalar ports.
///
/// For packed arrays of structs (`req_t [N-1:0]`), `element_count` carries
/// the array length and `element` describes one element's layout (so the
/// reader knows "every one of N elements has this `total`/`fields`"). For
/// scalar arrays and non-array ports both are `None`. Only one level of
/// array unwrap is exposed; deeper nesting collapses into the parent
/// `element.total`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedPortWidth {
    pub total: i64,
    #[serde(default)]
    pub fields: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<Box<ResolvedPortWidth>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstantiationInfo {
    pub module_name: String,
    pub instance_name: String,
    /// Textual call-site expressions, keyed by the *child*'s parameter
    /// name. Captured during the syntactic walk and never overwritten.
    #[serde(default)]
    pub param_bindings: BTreeMap<String, String>,
    /// Folded literal values produced by elaboration (e.g. `"32'd32"`).
    /// Empty when the build ran without `--elab` or when slang failed
    /// to resolve a particular symbol.
    #[serde(default)]
    pub resolved_param_values: BTreeMap<String, String>,
    #[serde(default)]
    pub port_bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub resolved_port_widths: BTreeMap<String, ResolvedPortWidth>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub line_start: Option<i64>,
    #[serde(default)]
    pub line_end: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportInfo {
    pub package_name: String,
    #[serde(default)]
    pub is_wildcard: bool,
    #[serde(default)]
    pub specific_symbols: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IncludeInfo {
    pub path: String,
}

/// All extracted data for one module/interface/package.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModuleData {
    pub name: String,
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub design: String,
    #[serde(default)]
    pub is_package: bool,

    #[serde(default)]
    pub line_start: Option<i64>,
    #[serde(default)]
    pub line_end: Option<i64>,
    #[serde(default)]
    pub param_block_lines: Option<(i64, i64)>,
    #[serde(default)]
    pub port_block_lines: Option<(i64, i64)>,

    #[serde(default)]
    pub parameters: Vec<ParamInfo>,
    #[serde(default)]
    pub ports: Vec<PortInfo>,
    #[serde(default)]
    pub instantiations: Vec<InstantiationInfo>,
    #[serde(default)]
    pub imports: Vec<ImportInfo>,
    #[serde(default)]
    pub includes: Vec<IncludeInfo>,

    #[serde(default)]
    pub exported_typedefs: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Identity of a build, deterministic across invocations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DesignIdentity {
    /// Stable id derived from `(workspace, sorted(targets), sorted(defines), top)`.
    pub id: String,
    /// Human-readable alias, either explicitly passed via `--design` or
    /// auto-derived as `<top>__<id_short>`.
    pub alias: String,
    /// Top module that drove the elaboration (may be empty for whole-package builds).
    pub top: Option<String>,
    /// The Bender targets that were active.
    pub targets: Vec<String>,
    /// `+define+` plus per-target defines, formatted as `NAME` or `NAME=VALUE`.
    pub defines: Vec<String>,
    /// Absolute path of the workspace root.
    pub workspace: String,
}

impl DesignIdentity {
    /// Compute the deterministic id.
    pub fn compute_id(
        workspace: impl AsRef<Path>,
        targets: &[String],
        defines: &[String],
        top: Option<&str>,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(workspace.as_ref().to_string_lossy().as_bytes());
        hasher.update(b"\x1e");
        let mut sorted_targets = targets.to_vec();
        sorted_targets.sort();
        for t in &sorted_targets {
            hasher.update(t.as_bytes());
            hasher.update(b"\x1f");
        }
        hasher.update(b"\x1e");
        let mut sorted_defines = defines.to_vec();
        sorted_defines.sort();
        for d in &sorted_defines {
            hasher.update(d.as_bytes());
            hasher.update(b"\x1f");
        }
        hasher.update(b"\x1e");
        if let Some(t) = top {
            hasher.update(t.as_bytes());
        }
        let digest = hasher.finalize();
        hex_lower(&digest)
    }

    pub fn build(
        workspace: impl AsRef<Path>,
        targets: Vec<String>,
        defines: Vec<String>,
        top: Option<String>,
        explicit_alias: Option<String>,
    ) -> Self {
        let id = Self::compute_id(&workspace, &targets, &defines, top.as_deref());
        let id_short: String = id.chars().take(8).collect();
        let alias = match explicit_alias {
            Some(a) if !a.is_empty() => a,
            _ => match top.as_deref() {
                Some(t) if !t.is_empty() => format!("{t}__{id_short}"),
                _ => format!("design__{id_short}"),
            },
        };
        Self {
            id,
            alias,
            top,
            targets,
            defines,
            workspace: workspace.as_ref().to_string_lossy().into_owned(),
        }
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Build manifest written next to the IR jsonl. Doubles as a dedup key for
/// incremental rebuilds (`bender kg build` is idempotent when the manifest's
/// `srclist_hash` matches the prior run).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: String,
    pub identity: DesignIdentity,
    pub slang_version: Option<String>,
    pub created_at: Option<String>,
    pub file_count: usize,
    pub module_count: usize,
    pub package_count: usize,
    pub edge_count: usize,
    /// Hash of the resolved srclist (after target/define filtering).
    pub srclist_hash: String,
    pub extraction_warnings: Vec<String>,
}

impl Manifest {
    pub fn new(identity: DesignIdentity) -> Self {
        Self {
            schema_version: KG_SCHEMA_VERSION.to_string(),
            identity,
            ..Default::default()
        }
    }
}

/// One resolved-edge patch produced by deferred elaboration.
///
/// Identifies an `INSTANTIATES` edge by its `(parent_module, child_module,
/// instance_name)` triple plus the design alias and carries the JSON-
/// serialised resolved param values + port widths. Consumed by
/// `Store::update_resolved_edges`, which translates the list into one
/// `UNWIND $rows AS r MATCH (...)-[e:INSTANTIATES {...}]->(...) SET ...`
/// statement (or chunked statements when the list is large).
///
/// Lives here (not in `bender-kg-extract`) so `bender-kg-store` can
/// consume it without taking a back-edge to the extract crate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedEdgeUpdate {
    pub parent_module: String,
    pub child_module: String,
    pub instance_name: String,
    pub design: String,
    pub resolved_param_values_json: String,
    pub resolved_port_widths_json: String,
}

/// One record on the IR jsonl wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IrRecord {
    Manifest(Manifest),
    Module(ModuleData),
}

/// Streaming JSONL reader.
pub fn read_ir_jsonl<R: std::io::BufRead>(reader: R) -> impl Iterator<Item = Result<IrRecord>> {
    reader.lines().filter_map(|line| match line {
        Err(e) => Some(Err(ModelsError::Io(e))),
        Ok(line) => {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(serde_json::from_str(trimmed).map_err(ModelsError::Serde))
            }
        }
    })
}

/// Streaming JSONL writer, emitting one record per line.
pub fn write_ir_record<W: std::io::Write>(writer: &mut W, rec: &IrRecord) -> Result<()> {
    serde_json::to_writer(&mut *writer, rec)?;
    writer.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_deterministic_and_order_independent() {
        let a = DesignIdentity::compute_id(
            "/ws",
            &vec!["b".to_string(), "a".to_string()],
            &vec!["X=1".to_string(), "Y".to_string()],
            Some("top"),
        );
        let b = DesignIdentity::compute_id(
            "/ws",
            &vec!["a".to_string(), "b".to_string()],
            &vec!["Y".to_string(), "X=1".to_string()],
            Some("top"),
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn alias_falls_back_to_top_plus_short_id() {
        let id = DesignIdentity::build(
            "/ws",
            vec!["t".to_string()],
            vec![],
            Some("top_module".to_string()),
            None,
        );
        assert!(id.alias.starts_with("top_module__"));
        assert_eq!(id.alias.len(), "top_module".len() + 2 + 8);
    }

    #[test]
    fn module_roundtrip() {
        let mut m = ModuleData::default();
        m.name = "tt_fpu_v2".into();
        m.file_path = "/x/tt_fpu_v2.sv".into();
        m.parameters.push(ParamInfo {
            name: "WIDTH".into(),
            kind: ParamKind::Int,
            default_value: "32".into(),
            is_type_param: false,
        });
        m.ports.push(PortInfo {
            name: "clk".into(),
            direction: Direction::Input,
            type_str: "logic".into(),
            width_expr: "1".into(),
            bit_width: Some(1),
            is_type_param: false,
            type_ref: None,
        });
        let s = serde_json::to_string(&m).unwrap();
        let m2: ModuleData = serde_json::from_str(&s).unwrap();
        assert_eq!(m2.name, m.name);
        assert_eq!(m2.parameters[0].name, "WIDTH");
        assert_eq!(m2.ports[0].direction, Direction::Input);
    }
}

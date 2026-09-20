// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Instance context extraction - shared logic for retrieving parameter
//! and port bindings from module instantiation edges.

use crate::{as_i64_or_none, as_string, decode_json, InstanceEdge, ResolvedPortWidth};
use grafeo::Value;
use std::collections::BTreeMap;

/// Convert a Cypher query row into an `InstanceEdge` struct.
///
/// Expected row layout: `p.name, c.name, instance_name, design,
/// param_bindings_json, port_bindings_json, resolved_param_values_json,
/// resolved_port_widths_json, line_start, line_end`.
pub fn row_to_instance_edge(row: &[Value], parent_file: &str) -> InstanceEdge {
    let parent = as_string(&row[0]);
    let child = as_string(&row[1]);
    let instance_name = as_string(&row[2]);
    let design = as_string(&row[3]);
    let param_bindings: BTreeMap<String, String> =
        decode_json(&as_string(&row[4])).unwrap_or_default();
    let port_bindings: BTreeMap<String, String> =
        decode_json(&as_string(&row[5])).unwrap_or_default();
    let resolved_param_values: BTreeMap<String, String> =
        decode_json(&as_string(&row[6])).unwrap_or_default();
    let resolved_port_widths: BTreeMap<String, ResolvedPortWidth> =
        decode_json(&as_string(&row[7])).unwrap_or_default();
    let line_start = as_i64_or_none(&row[8]).filter(|v| *v >= 0);
    let line_end = as_i64_or_none(&row[9]).filter(|v| *v >= 0);

    InstanceEdge {
        parent,
        child,
        instance_name,
        param_bindings,
        resolved_param_values,
        port_bindings,
        resolved_port_widths,
        parent_file_path: parent_file.to_string(),
        line_start,
        line_end,
        design,
    }
}

/// Standard Cypher query for fetching instance edge data.
///
/// Returns: `p.name, c.name, r.instance_name, r.design,
/// r.param_bindings_json, r.port_bindings_json,
/// r.resolved_param_values_json, r.resolved_port_widths_json,
/// r.line_start, r.line_end`.
pub const INSTANCE_EDGE_QUERY: &str =
    "MATCH (p:Module {name: $p})-[r:INSTANTIATES]->(c:Module) \
     RETURN p.name, c.name, r.instance_name, r.design, \
            r.param_bindings_json, r.port_bindings_json, \
            r.resolved_param_values_json, r.resolved_port_widths_json, \
            r.line_start, r.line_end";

/// Cypher query for fetching a specific parent->child edge.
pub const INSTANCE_EDGE_QUERY_FILTERED: &str =
    "MATCH (p:Module {name: $p})-[r:INSTANTIATES]->(c:Module {name: $c}) \
     RETURN p.name, c.name, r.instance_name, r.design, \
            r.param_bindings_json, r.port_bindings_json, \
            r.resolved_param_values_json, r.resolved_port_widths_json, \
            r.line_start, r.line_end";

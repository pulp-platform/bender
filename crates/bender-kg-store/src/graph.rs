// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Graph traversal primitives - BFS, parent/child queries, path finding.

use crate::instance::{row_to_instance_edge, INSTANCE_EDGE_QUERY};
use crate::{cparam, InstanceEdge, Result};
use grafeo::GrafeoDB;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// List all instance edges originating from `parent` module.
///
/// This is used by BFS traversal and other algorithms that need to
/// explore the instantiation graph.
pub fn list_instance_edges_from(
    db: &GrafeoDB,
    parent: &str,
    parent_file: &str,
) -> Result<Vec<InstanceEdge>> {
    let r = db.execute_cypher_with_params(INSTANCE_EDGE_QUERY, cparam("p", parent))?;

    Ok(r.rows()
        .iter()
        .map(|row| row_to_instance_edge(row, parent_file))
        .collect())
}

/// Find the shortest instantiation path from `from` module to `to` module
/// using BFS traversal.
///
/// Returns the sequence of `InstanceEdge`s along the path, or an empty
/// vector if no path exists.
///
/// This is more efficient than using Cypher's `shortestPath` because we
/// need to extract all the edge metadata anyway, so we might as well do
/// the traversal in Rust.
pub fn trace_hierarchy_path(
    db: &GrafeoDB,
    module_meta_fn: &dyn Fn(&str) -> Result<(String, String)>,
    from: &str,
    to: &str,
) -> Result<Vec<InstanceEdge>> {
    let mut prev: BTreeMap<String, (String, InstanceEdge)> = BTreeMap::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    queue.push_back(from.to_string());
    visited.insert(from.to_string());

    while let Some(cur) = queue.pop_front() {
        if cur == to {
            break;
        }

        // Get parent file path for this module
        let (_design, parent_file) = module_meta_fn(&cur)?;

        for edge in list_instance_edges_from(db, &cur, &parent_file)? {
            if visited.insert(edge.child.clone()) {
                prev.insert(edge.child.clone(), (cur.clone(), edge.clone()));
                queue.push_back(edge.child.clone());
            }
        }
    }

    if !visited.contains(to) {
        return Ok(Vec::new());
    }

    // Reconstruct path by walking backwards from `to` to `from`
    let mut chain: Vec<InstanceEdge> = Vec::new();
    let mut cur = to.to_string();
    while let Some((parent, edge)) = prev.remove(&cur) {
        chain.push(edge);
        cur = parent;
        if cur == from {
            break;
        }
    }

    chain.reverse();
    Ok(chain)
}

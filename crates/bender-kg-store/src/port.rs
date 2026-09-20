// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Port analysis utilities - matching, similarity, comparison.

use bender_kg_models::{Direction, PortInfo};
use std::collections::BTreeMap;

/// Convert a Direction enum to a string.
pub fn direction_str(d: Direction) -> &'static str {
    match d {
        Direction::Input => "input",
        Direction::Output => "output",
        Direction::Inout => "inout",
        Direction::Ref => "ref",
    }
}

/// Check if two port directions are compatible for connection.
///
/// Returns true if:
/// - One is Input and the other is Output
/// - Both are Inout
pub fn directions_complement(a: Direction, b: Direction) -> bool {
    matches!(
        (a, b),
        (Direction::Input, Direction::Output)
            | (Direction::Output, Direction::Input)
            | (Direction::Inout, Direction::Inout)
    )
}

/// Strip a prefix from a port name.
///
/// Used to normalize port names when comparing interfaces with different
/// prefixes (e.g., comparing `slv_req` and `mst_req` by stripping `slv_`
/// and `mst_` respectively).
pub fn strip_prefix(name: &str, prefix: &str) -> String {
    if !prefix.is_empty() && name.starts_with(prefix) {
        name[prefix.len()..].to_string()
    } else {
        name.to_string()
    }
}

/// Parse a `port_set_json` payload back into the sorted-dedup vector.
///
/// Returns an empty vector on parse error (graceful degradation).
pub fn parse_port_set_json(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    serde_json::from_str(raw).unwrap_or_default()
}

/// Linear two-pointer intersection count over two sorted, dedup'd port
/// name vectors. O(|a| + |b|).
pub fn sorted_intersection_count(a: &[String], b: &[String]) -> usize {
    let mut i = 0;
    let mut j = 0;
    let mut n = 0;
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    n
}

/// Result of comparing two module interfaces.
#[derive(Debug, Clone)]
pub struct InterfaceComparison {
    pub matched: Vec<PortMatch>,
    pub width_conflicts: Vec<WidthConflict>,
    pub unmatched_a: Vec<String>,
    pub unmatched_b: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PortMatch {
    pub name: String,
    pub a_direction: Direction,
    pub b_direction: Direction,
    pub direction_complementary: bool,
    pub a_width: Option<i64>,
    pub b_width: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct WidthConflict {
    pub name: String,
    pub a_width: Option<i64>,
    pub b_width: Option<i64>,
}

/// Compare ports between two modules, optionally stripping prefixes.
///
/// Returns matched ports, width conflicts, and unmatched ports from each
/// module.
pub fn compare_ports(
    a_ports: &[PortInfo],
    b_ports: &[PortInfo],
    prefix_a: &str,
    prefix_b: &str,
) -> InterfaceComparison {
    let a_map: BTreeMap<String, &PortInfo> = a_ports
        .iter()
        .map(|p| (strip_prefix(&p.name, prefix_a), p))
        .collect();

    let b_map: BTreeMap<String, &PortInfo> = b_ports
        .iter()
        .map(|p| (strip_prefix(&p.name, prefix_b), p))
        .collect();

    let mut matched = Vec::new();
    let mut width_conflicts = Vec::new();
    let mut unmatched_a = Vec::new();

    for (name, pa) in &a_map {
        match b_map.get(name) {
            Some(pb) => {
                let dir_ok = directions_complement(pa.direction, pb.direction);
                let width_ok = pa.bit_width == pb.bit_width
                    || pa.bit_width.is_none()
                    || pb.bit_width.is_none();

                matched.push(PortMatch {
                    name: name.clone(),
                    a_direction: pa.direction,
                    b_direction: pb.direction,
                    direction_complementary: dir_ok,
                    a_width: pa.bit_width,
                    b_width: pb.bit_width,
                });

                if !width_ok {
                    width_conflicts.push(WidthConflict {
                        name: name.clone(),
                        a_width: pa.bit_width,
                        b_width: pb.bit_width,
                    });
                }
            }
            None => unmatched_a.push(name.clone()),
        }
    }

    let unmatched_b: Vec<String> = b_map
        .keys()
        .filter(|k| !a_map.contains_key(*k))
        .cloned()
        .collect();

    InterfaceComparison {
        matched,
        width_conflicts,
        unmatched_a,
        unmatched_b,
    }
}

/// Compute Jaccard similarity score between two port sets.
///
/// Returns a score between 0.0 and 1.0, where 1.0 means identical port sets.
pub fn compute_jaccard_similarity(
    a_sorted: &[String],
    b_sorted: &[String],
    a_cardinality: i64,
    b_cardinality: i64,
) -> f64 {
    if a_cardinality == 0 || b_cardinality == 0 {
        return 0.0;
    }

    let intersection = sorted_intersection_count(a_sorted, b_sorted) as f64;
    let union = (a_cardinality as f64) + (b_cardinality as f64) - intersection;

    if union <= 0.0 {
        return 0.0;
    }

    intersection / union
}

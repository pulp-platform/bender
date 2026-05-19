// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Synchronous graph reads. These all delegate to the Grafeo-backed
//! [`Store`], wrapping its result type into [`crate::CoreError`] for
//! callers that compose graph + vector + IR errors uniformly.

use std::collections::HashSet;

use bender_kg_models::ModuleData;

use crate::{Engine, GraphStats, InstanceEdge, Result, Subgraph};

impl Engine {
    pub fn get_module(&self, name: &str) -> Result<Option<ModuleData>> {
        Ok(self.store.get_module(name)?)
    }
    pub fn get_subgraph(&self, name: &str, depth: i32) -> Result<Subgraph> {
        Ok(self.store.get_subgraph(name, depth)?)
    }
    pub fn get_parents(&self, name: &str) -> Result<Vec<ModuleData>> {
        Ok(self.store.get_parents(name)?)
    }
    pub fn get_children(&self, name: &str) -> Result<Vec<ModuleData>> {
        Ok(self.store.get_children(name)?)
    }
    pub fn get_instance_context(&self, parent: &str, child: &str) -> Result<Vec<InstanceEdge>> {
        Ok(self.store.get_instance_context(parent, child)?)
    }
    pub fn trace_hierarchy_path(&self, from: &str, to: &str) -> Result<Vec<InstanceEdge>> {
        Ok(self.store.trace_hierarchy_path(from, to)?)
    }
    pub fn check_connectivity(&self, module: &str, depth: i32) -> Result<Vec<serde_json::Value>> {
        Ok(self.store.check_connectivity(module, depth)?)
    }
    pub fn trace_parameter(&self, module: &str, param: &str) -> Result<Vec<serde_json::Value>> {
        Ok(self.store.trace_parameter(module, param)?)
    }
    pub fn trace_signal(&self, module: &str, signal: &str) -> Result<Vec<serde_json::Value>> {
        Ok(self.store.trace_signal(module, signal)?)
    }

    /// Recursively follow a signal through the instantiation hierarchy.
    /// Returns a nested structure where each entry gains a `"children"` array
    /// containing connections at the next level (signal name as it appears in
    /// the child module).
    pub fn trace_signal_recursive(
        &self,
        module: &str,
        signal: &str,
        max_depth: i32,
    ) -> Result<Vec<serde_json::Value>> {
        let fetch = |m: &str, k: &str| Ok(self.store.trace_signal(m, k)?);
        self.trace_rec(module, signal, max_depth, 0, &mut HashSet::new(), &fetch, "child_port")
    }

    /// Recursively follow a parameter through the instantiation hierarchy.
    /// Returns a nested structure where each entry gains a `"children"` array
    /// containing further propagations from the child module's perspective.
    pub fn trace_parameter_recursive(
        &self,
        module: &str,
        param: &str,
        max_depth: i32,
    ) -> Result<Vec<serde_json::Value>> {
        let fetch = |m: &str, k: &str| Ok(self.store.trace_parameter(m, k)?);
        self.trace_rec(module, param, max_depth, 0, &mut HashSet::new(), &fetch, "child_parameter")
    }

    /// Generic DFS with cycle detection. `fetch` retrieves the flat one-hop
    /// connections for a (module, key) pair; `child_field` names the JSON
    /// field carrying the key as it appears in the child module.
    fn trace_rec(
        &self,
        module: &str,
        key: &str,
        max_depth: i32,
        depth: i32,
        on_path: &mut HashSet<(String, String)>,
        fetch: &dyn Fn(&str, &str) -> Result<Vec<serde_json::Value>>,
        child_field: &str,
    ) -> Result<Vec<serde_json::Value>> {
        if depth >= max_depth {
            return Ok(vec![]);
        }
        let path_key = (module.to_string(), key.to_string());
        if on_path.contains(&path_key) {
            return Ok(vec![]); // cycle guard
        }
        on_path.insert(path_key.clone());
        let flat = fetch(module, key)?;
        let mut result = Vec::new();
        for conn in flat {
            let child_module = conn["child"].as_str().unwrap_or("").to_string();
            let child_key   = conn[child_field].as_str().unwrap_or("").to_string();
            let children = if !child_module.is_empty() && !child_key.is_empty() {
                self.trace_rec(&child_module, &child_key, max_depth, depth + 1, on_path, fetch, child_field)?
            } else {
                vec![]
            };
            let mut entry = conn;
            entry["children"] = serde_json::json!(children);
            result.push(entry);
        }
        on_path.remove(&path_key);
        Ok(result)
    }

    pub fn find_by_protocol(
        &self,
        protocol: &str,
        design: Option<&str>,
    ) -> Result<Vec<ModuleData>> {
        Ok(self.store.find_by_protocol(protocol, design)?)
    }
    pub fn match_interfaces(
        &self,
        a: &str,
        b: &str,
        prefix_a: &str,
        prefix_b: &str,
    ) -> Result<serde_json::Value> {
        Ok(self.store.match_interfaces(a, b, prefix_a, prefix_b)?)
    }
    pub fn find_structurally_similar(
        &self,
        module: &str,
        min_overlap: f64,
        design: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        Ok(self
            .store
            .find_structurally_similar(module, min_overlap, design)?)
    }
    pub fn stats(&self, design: Option<&str>) -> Result<GraphStats> {
        Ok(self.store.stats(design)?)
    }
}

// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Source-line retrieval for ports / params / module / instance ranges.
//! All inputs come from the graph store; the file system is only touched
//! to read the requested slice.

use crate::{CoreError, Engine, Result};

impl Engine {
    pub fn get_source_snippet(
        &self,
        module_name: &str,
        element: &str,
        instance_name: &str,
    ) -> Result<serde_json::Value> {
        let m = self
            .get_module(module_name)?
            .ok_or_else(|| CoreError::NotFound(module_name.into()))?;
        let (start, end): (Option<i64>, Option<i64>) = match element {
            "module" => (m.line_start, m.line_end),
            "ports" => match m.port_block_lines {
                Some((s, e)) => (Some(s), Some(e)),
                None => (None, None),
            },
            "params" => match m.param_block_lines {
                Some((s, e)) => (Some(s), Some(e)),
                None => (None, None),
            },
            "instance" => {
                if instance_name.is_empty() {
                    return Ok(serde_json::json!({
                        "error": "instance_name required for element='instance'"
                    }));
                }
                let mut s = None;
                let mut e = None;
                for inst in &m.instantiations {
                    if inst.instance_name == instance_name {
                        s = inst.line_start;
                        e = inst.line_end;
                        break;
                    }
                }
                (s, e)
            }
            _ => {
                return Ok(serde_json::json!({
                    "error": format!(
                        "unknown element '{}'. Use: module, ports, params, instance",
                        element
                    ),
                }));
            }
        };
        let (Some(start), Some(end)) = (start, end) else {
            return Ok(serde_json::json!({
                "error": format!(
                    "no line range for element '{}' on module '{}'",
                    element, module_name
                ),
            }));
        };
        if m.file_path.is_empty() || !std::path::Path::new(&m.file_path).exists() {
            return Ok(serde_json::json!({
                "error": format!("source file not found: {}", m.file_path),
                "file_path": m.file_path,
                "line_start": start,
                "line_end": end,
            }));
        }
        let text = std::fs::read_to_string(&m.file_path)?;
        let lines: Vec<&str> = text.lines().collect();
        let s = start.max(1) as usize;
        let e = (end as usize).min(lines.len());
        let snippet = if s <= e {
            lines[s - 1..e].join("\n")
        } else {
            String::new()
        };
        Ok(serde_json::json!({
            "file_path": m.file_path,
            "line_start": start,
            "line_end": end,
            "element": element,
            "module_name": module_name,
            "snippet": snippet,
        }))
    }
}

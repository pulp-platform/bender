// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Merge the elaborated walk's per-instance contexts (resolved parameters and
//! port widths) into module-level `InstantiationInfo`.
//!
//! Elaboration is *opt-in* via `--elab` and only enriches
//! `resolved_param_values` / `resolved_port_widths`. The graph still
//! contains every parsed module that the prior pruning pass deemed
//! reachable from `--top`; callers who don't pass `--elab` skip this pass
//! entirely and pay zero elaboration cost. The textual `param_bindings`
//! map (call-site expressions captured by the syntactic walk) is never
//! mutated -- it is the source-of-truth for "what was written" and lives
//! alongside `resolved_param_values` ("what slang folded it to").

use std::collections::{BTreeMap, HashMap};

use bender_kg_models::{ModuleData, ResolvedPortWidth};
use bender_slang::{KgElabResult, KgKeyValue, KgPortWidth, SlangSession};

use crate::{ExtractError, Result};

/// Run `walk_elaborated` against `session` and merge resolved parameter
/// bindings + port widths into the matching `InstantiationInfo` records on
/// `modules`. Match key is `(parent_module_name, instance_name,
/// child_module)`.
///
/// Complexity is `O(M + total_I + C)` for `M` modules, `total_I`
/// instantiations across all modules, and `C` elab contexts: we build
/// a parent-name index and a per-module instantiation index once up
/// front, then look up in O(1) inside the hot loop. Previously this
/// loop was `O(C * (M + I))` which dominated the build wall-clock on
/// designs with many contexts (hundreds of seconds for large designs).
///
/// Returns an `InvalidInput` error if the caller named one or more `tops`
/// but slang resolved zero instance contexts — typically a mistyped module
/// name. Silent-empty was a documented footgun; surfacing it as an error
/// matches the principle "if you opt into enrichment, opt-in must succeed".
pub(crate) fn enrich(
    session: &SlangSession,
    tops: &[String],
    modules: &mut [ModuleData],
) -> Result<Vec<String>> {
    if tops.is_empty() {
        return Ok(Vec::new());
    }
    let elab: KgElabResult = session.walk_elaborated(tops)?;
    if elab.contexts.is_empty() {
        return Err(ExtractError::InvalidInput(format!(
            "--top {:?} did not match any instance in the elaborated design; \
             check the name(s) or omit --elab to build a graph without \
             instance-level enrichment",
            tops
        )));
    }

    // Indexes: `name -> module_idx` and per-module `(instance_name,
    // child_module) -> inst_idx`. Owned `String` keys because the
    // resulting borrows from `modules` will alias the mutable slice
    // inside the hot loop.
    let by_name: HashMap<String, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.clone(), i))
        .collect();
    let inst_idx_per_module: Vec<HashMap<(String, String), usize>> = modules
        .iter()
        .map(|m| {
            m.instantiations
                .iter()
                .enumerate()
                .map(|(i, inst)| ((inst.instance_name.clone(), inst.module_name.clone()), i))
                .collect()
        })
        .collect();

    let mut warnings = elab.warnings.clone();
    let mut merged = 0usize;
    let mut parent_miss = 0usize;
    let mut inst_miss = 0usize;

    for ctx in &elab.contexts {
        // Top instances have no parent in our schema; nothing to merge into.
        if ctx.parent_module.is_empty() {
            continue;
        }
        let Some(&pi) = by_name.get(ctx.parent_module.as_str()) else {
            parent_miss += 1;
            continue;
        };
        let Some(&ii) =
            inst_idx_per_module[pi].get(&(ctx.instance_name.clone(), ctx.child_module.clone()))
        else {
            inst_miss += 1;
            continue;
        };
        merged += 1;
        let inst = &mut modules[pi].instantiations[ii];
        for kv in &ctx.param_bindings {
            // Skip empties so a partial elab failure on one symbol doesn't
            // shadow the textual call-site value with "".
            if !kv.value.is_empty() {
                inst.resolved_param_values
                    .insert(kv.key.clone(), kv.value.clone());
            }
        }
        for pw in &ctx.port_widths {
            inst.resolved_port_widths
                .insert(pw.name.clone(), to_resolved_width(pw));
        }
    }
    warnings.push(format!(
        "elab: {} contexts, merged={merged} (parent_miss={parent_miss}, inst_miss={inst_miss})",
        elab.contexts.len()
    ));

    Ok(warnings)
}

/// Convert the FFI `KgPortWidth` (string-encoded field widths over cxx) into
/// the typed model record. Field-width strings that fail to parse are
/// dropped silently — slang only emits them via `std::to_string`, so the
/// fallback is purely defensive.
///
/// When `pw.element_count > 0` the port's canonical type was a packed array,
/// so we surface a one-level `element` template (per-element `total` plus
/// flattened struct fields). Scalar arrays produce an `element` with empty
/// `fields`; non-array ports leave both `element` and `element_count` as
/// `None`.
pub(crate) fn to_resolved_width(pw: &KgPortWidth) -> ResolvedPortWidth {
    let fields = parse_kv_widths(&pw.fields);
    let (element_count, element) = if pw.element_count > 0 {
        (
            Some(pw.element_count),
            Some(Box::new(ResolvedPortWidth {
                total: pw.element_total,
                fields: parse_kv_widths(&pw.element_fields),
                element_count: None,
                element: None,
            })),
        )
    } else {
        (None, None)
    };
    ResolvedPortWidth {
        total: pw.total,
        fields,
        element_count,
        element,
    }
}

fn parse_kv_widths(kvs: &[KgKeyValue]) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for kv in kvs {
        if let Ok(w) = kv.value.parse::<i64>() {
            out.insert(kv.key.clone(), w);
        }
    }
    out
}

// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! SystemVerilog -> `kg.v3` extraction pipeline.
//!
//! Mirrors `bender pickle`'s shape: per Bender source group we drive
//! `SlangSession::parse_group`, then walk the parsed trees syntactically
//! (`walk_design`) to capture every declared module, package, and
//! interface. The graph contains the *parsed* view of the design.
//!
//! `ExtractInputs.tops` (`--top` on the CLI) is REQUIRED: the parsed tree
//! set is pruned to those reachable from these tops before the syntactic
//! walk, so the resulting graph captures exactly the modules used by the
//! design. Elaboration (`walk_elaborated`) is a separate opt-in via
//! `ExtractInputs.elab` (`--elab`); when set, slang specializes parameters
//! / resolves port widths from the named hierarchy roots and we merge
//! those resolved values into the matching `InstantiationInfo` records.

mod defines;
mod elab;
mod emit;
mod parse;
mod walk;

pub use defines::target_defines;
pub use emit::{IrSink, VecSink};

use std::path::PathBuf;

use bender_kg_models::{Manifest, ModuleData};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("bender-slang error: {0}")]
    Slang(#[from] bender_slang::SlangError),
    #[error("models error: {0}")]
    Models(#[from] bender_kg_models::ModelsError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T> = std::result::Result<T, ExtractError>;

/// One Bender source group (already filtered for the active target set).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceGroupInput {
    pub files: Vec<String>,
    pub include_dirs: Vec<String>,
    /// Preprocessor defines, formatted as `NAME` or `NAME=VALUE`.
    pub defines: Vec<String>,
}

/// Aggregate input describing the full design build.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractInputs {
    pub workspace: String,
    pub targets: Vec<String>,
    /// One or more elaboration roots. REQUIRED. The graph is pruned to
    /// only the syntax trees reachable from these tops (via slang's
    /// symbol-reference graph) before the downstream walk, so the
    /// resulting graph captures exactly the modules used by the design.
    /// The first entry is recorded in the manifest for traceability.
    pub tops: Vec<String>,
    /// When `true`, run slang's elaboration pass from `tops` and enrich
    /// `InstantiationInfo` with `resolved_param_values` and
    /// `resolved_port_widths`. When `false` (default), elaboration is
    /// skipped entirely; pruning still happens.
    #[serde(default)]
    pub elab: bool,
    pub design_alias: Option<String>,
    pub groups: Vec<SourceGroupInput>,
    /// Treat all source groups as one slang compilation unit (vcs / `vlog
    /// -mfcu` semantics): `\`define`s declared in earlier groups become
    /// visible to later groups. Default `false` keeps the per-group
    /// preprocessor scoping that the simulator-script paths use.
    #[serde(default)]
    pub single_unit: bool,
    /// Best-effort parsing: report parse-time errors but don't abort the
    /// build. The indexer ingests whichever modules survived parsing.
    /// Useful for repos with encrypted vendor IP, missing `\`include`s, or
    /// other hostile inputs. Default `false` (strict).
    #[serde(default)]
    pub lenient: bool,
    /// Hint for the maximum number of parallel parse workers (`0` means
    /// "use the default of 1").
    ///
    /// Pruning (Phase 1, mandatory) requires `reachable_tree_indices` to
    /// resolve symbol references across every parsed tree from a single
    /// `SlangSession`; the C++ analyzer has no public API to merge two
    /// slang sessions, so per-worker sessions cannot share their
    /// `SourceManager` / symbol tables. As a result this hint is
    /// currently capped to `1` internally — the field exists so the CLI
    /// flag stays stable while a future change to `bender-slang` adds
    /// session merging. Setting it to `>1` today is a no-op and emits an
    /// informational warning.
    #[serde(default)]
    pub parse_jobs: u32,
}

pub use bender_kg_models::ResolvedEdgeUpdate;

/// Owned post-pipeline elaboration handle.
///
/// Yielded by [`extract_pipelined`] when `inputs.elab` is `true`; carries
/// the parsed, pruned `SlangSession` plus the `--top` roots so a caller
/// can drive `walk_elaborated` on a worker thread, overlapping it with
/// the base graph upsert. Implements `Send` (via the underlying
/// `bender_slang::SlangSession` impl) so it can be moved into a
/// `std::thread::scope` worker. Drop the handle to release the slang
/// state.
pub struct ElabHandle {
    session: bender_slang::SlangSession,
    tops: Vec<String>,
    design: String,
}

impl ElabHandle {
    /// Drive `walk_elaborated` against the carried session and translate
    /// every resolved instance context into a flat
    /// [`ResolvedEdgeUpdate`]. Returns the updates plus the elab-emitted
    /// warnings. Errors mirror the inline `elab::enrich` path: returns
    /// `InvalidInput` if no instance contexts come back (i.e. the
    /// requested `--top` matched nothing in the elaborated design).
    pub fn run(&self) -> Result<(Vec<ResolvedEdgeUpdate>, Vec<String>)> {
        let elab = self.session.walk_elaborated(&self.tops)?;
        if elab.contexts.is_empty() {
            return Err(ExtractError::InvalidInput(format!(
                "--top {:?} did not match any instance in the elaborated design; \
                 check the name(s) or omit --elab to build a graph without \
                 instance-level enrichment",
                self.tops
            )));
        }
        let mut updates = Vec::with_capacity(elab.contexts.len());
        for ctx in &elab.contexts {
            if ctx.parent_module.is_empty() {
                continue;
            }
            let mut rpv: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for kv in &ctx.param_bindings {
                if !kv.value.is_empty() {
                    rpv.insert(kv.key.clone(), kv.value.clone());
                }
            }
            let mut rpw: std::collections::BTreeMap<String, bender_kg_models::ResolvedPortWidth> =
                std::collections::BTreeMap::new();
            for pw in &ctx.port_widths {
                rpw.insert(pw.name.clone(), elab::to_resolved_width(pw));
            }
            updates.push(ResolvedEdgeUpdate {
                parent_module: ctx.parent_module.clone(),
                child_module: ctx.child_module.clone(),
                instance_name: ctx.instance_name.clone(),
                design: self.design.clone(),
                resolved_param_values_json: serde_json::to_string(&rpv)?,
                resolved_port_widths_json: serde_json::to_string(&rpw)?,
            });
        }
        Ok((updates, elab.warnings.clone()))
    }
}

/// Run the extraction pipeline against fully populated inputs.
///
/// Emits exactly one `IrRecord::Manifest` followed by N `IrRecord::Module`
/// records. Returns the manifest plus a (partial) [`BuildPhases`] populated
/// with `slang_parse_*` / `walk_design_s` / `elaborate_s` / `ir_write_s`. The
/// caller is expected to fill `store_upsert_s`, `embed_s`, and `total_s`.
pub fn extract<S: IrSink>(
    inputs: &ExtractInputs,
    sink: &mut S,
) -> Result<(Manifest, bender_kg_models::BuildPhases)> {
    if inputs.workspace.is_empty() {
        return Err(ExtractError::InvalidInput(
            "ExtractInputs.workspace must be set".into(),
        ));
    }
    if inputs.groups.is_empty() {
        return Err(ExtractError::InvalidInput(
            "ExtractInputs.groups must be non-empty".into(),
        ));
    }
    if inputs.tops.is_empty() {
        return Err(ExtractError::InvalidInput(
            "ExtractInputs.tops must be non-empty: pass at least one --top MODULE".into(),
        ));
    }

    let mut phases = bender_kg_models::BuildPhases::default();

    log_parse_jobs_advisory(inputs);

    // 1. Parse: per-group SlangSession::parse_group.
    let t_parse = std::time::Instant::now();
    let mut p = parse::parse(&inputs.groups, inputs.single_unit, inputs.lenient)?;
    phases.slang_parse_s = t_parse.elapsed().as_secs_f64();
    phases.slang_parse_group_count = p.group_durations.len();
    phases.slang_parse_max_group_s = p
        .group_durations
        .iter()
        .map(|d| d.as_secs_f64())
        .fold(0.0_f64, f64::max);
    log::info!(
        "kg.phase slang_parse {:.3}s ({} groups, max {:.3}s, single_unit={}, lenient={})",
        phases.slang_parse_s,
        phases.slang_parse_group_count,
        phases.slang_parse_max_group_s,
        inputs.single_unit,
        inputs.lenient,
    );

    // 2. Prune: keep only the trees reachable from `tops`. `walk_design`
    // and `walk_elaborated` both iterate `session.trees()`, so retaining
    // the subset in-place automatically narrows downstream work.
    let trees_before = p.session.tree_count();
    let t_prune = std::time::Instant::now();
    let kept_u32 = match p.session.reachable_indices(&inputs.tops) {
        Ok(idx) => idx.into_iter().map(|i| i as u32).collect::<Vec<u32>>(),
        Err(e) => {
            return Err(ExtractError::InvalidInput(format!(
                "--top {:?} did not match any parsed module: {}; \
                 check the name(s) or your source list",
                inputs.tops, e
            )));
        }
    };
    p.session.retain_trees(&kept_u32);
    phases.prune_s = t_prune.elapsed().as_secs_f64();
    log::info!(
        "kg.phase prune {:.3}s ({} -> {} trees)",
        phases.prune_s,
        trees_before,
        kept_u32.len(),
    );
    if kept_u32.is_empty() {
        return Err(ExtractError::InvalidInput(format!(
            "no syntax trees reachable from --top {:?}; check the name(s) or your source list",
            inputs.tops
        )));
    }

    // 3. Syntactic walk: every declared module/package/interface in the
    //    pruned set.
    let t_walk = std::time::Instant::now();
    let walked = p.session.walk_design()?;
    phases.walk_design_s = t_walk.elapsed().as_secs_f64();
    log::info!("kg.phase walk_design {:.3}s", phases.walk_design_s);

    let identity = bender_kg_models::DesignIdentity::build(
        &inputs.workspace,
        inputs.targets.clone(),
        p.all_defines.clone(),
        inputs.tops.first().cloned(),
        inputs.design_alias.clone(),
    );

    let modules: Vec<ModuleData> = walked
        .modules
        .iter()
        .map(|m| walk::convert_module(m, &identity.alias))
        .collect();
    let mut warnings: Vec<String> = walked.warnings.clone();

    // 4. Deduplicate by name; keep the richer record (better location info or
    // more instantiations).
    let mut by_name: IndexMap<String, ModuleData> = IndexMap::new();
    for m in modules.into_iter() {
        match by_name.get_mut(&m.name) {
            Some(existing)
                if (m.line_start.is_some() && existing.line_start.is_none())
                    || m.instantiations.len() > existing.instantiations.len() =>
            {
                *existing = m;
            }
            Some(_) => {}
            None => {
                by_name.insert(m.name.clone(), m);
            }
        }
    }
    let mut modules: Vec<ModuleData> = by_name.into_values().collect();

    // 5. Optional: elaborate from `--top` roots and merge resolved
    //    parameter bindings + port widths. Gated by `inputs.elab`; pruning
    //    above already used the same roots regardless.
    let t_elab = std::time::Instant::now();
    let elab_warnings = if inputs.elab {
        let w = elab::enrich(&p.session, &inputs.tops, &mut modules)?;
        phases.elaborate_s = t_elab.elapsed().as_secs_f64();
        log::info!("kg.phase elaborate {:.3}s", phases.elaborate_s);
        w
    } else {
        Vec::new()
    };
    warnings.extend(elab_warnings);

    // 6. Manifest + stream.
    let t_ir = std::time::Instant::now();
    let manifest = emit::build_manifest(identity, &modules, p.file_count, p.srclist_hash, warnings);
    emit::stream(sink, &manifest, &modules)?;
    phases.ir_write_s = t_ir.elapsed().as_secs_f64();
    log::info!("kg.phase ir_write {:.3}s", phases.ir_write_s);

    Ok((manifest, phases))
}

/// Like [`extract`] but defers the elaboration merge.
///
/// Runs parse + prune + walk_design + dedup + IR streaming (with
/// `resolved_*` fields empty), then returns:
///   * the manifest,
///   * the deduplicated, un-enriched modules (caller can re-use them
///     for upsert, embedding, etc.),
///   * the partially-populated [`bender_kg_models::BuildPhases`] (no
///     `elaborate_s` populated; the caller fills it after running the
///     handle),
///   * an [`ElabHandle`] when `inputs.elab` is `true`, otherwise
///     `None`.
///
/// The intended caller (`bender_kg_core::Engine::build`) drives
/// `handle.run()` on a worker thread in parallel with the base graph
/// upsert, then applies the resulting [`ResolvedEdgeUpdate`] list via
/// `Store::update_resolved_edges`. Everything else is identical to
/// [`extract`].
pub fn extract_pipelined<S: IrSink>(
    inputs: &ExtractInputs,
    sink: &mut S,
) -> Result<(
    Manifest,
    Vec<ModuleData>,
    bender_kg_models::BuildPhases,
    Option<ElabHandle>,
)> {
    if inputs.workspace.is_empty() {
        return Err(ExtractError::InvalidInput(
            "ExtractInputs.workspace must be set".into(),
        ));
    }
    if inputs.groups.is_empty() {
        return Err(ExtractError::InvalidInput(
            "ExtractInputs.groups must be non-empty".into(),
        ));
    }
    if inputs.tops.is_empty() {
        return Err(ExtractError::InvalidInput(
            "ExtractInputs.tops must be non-empty: pass at least one --top MODULE".into(),
        ));
    }

    let mut phases = bender_kg_models::BuildPhases::default();

    log_parse_jobs_advisory(inputs);

    let t_parse = std::time::Instant::now();
    let mut p = parse::parse(&inputs.groups, inputs.single_unit, inputs.lenient)?;
    phases.slang_parse_s = t_parse.elapsed().as_secs_f64();
    phases.slang_parse_group_count = p.group_durations.len();
    phases.slang_parse_max_group_s = p
        .group_durations
        .iter()
        .map(|d| d.as_secs_f64())
        .fold(0.0_f64, f64::max);
    log::info!(
        "kg.phase slang_parse {:.3}s ({} groups, max {:.3}s, single_unit={}, lenient={}) [pipelined]",
        phases.slang_parse_s,
        phases.slang_parse_group_count,
        phases.slang_parse_max_group_s,
        inputs.single_unit,
        inputs.lenient,
    );

    let trees_before = p.session.tree_count();
    let t_prune = std::time::Instant::now();
    let kept_u32 = match p.session.reachable_indices(&inputs.tops) {
        Ok(idx) => idx.into_iter().map(|i| i as u32).collect::<Vec<u32>>(),
        Err(e) => {
            return Err(ExtractError::InvalidInput(format!(
                "--top {:?} did not match any parsed module: {}; \
                 check the name(s) or your source list",
                inputs.tops, e
            )));
        }
    };
    p.session.retain_trees(&kept_u32);
    phases.prune_s = t_prune.elapsed().as_secs_f64();
    log::info!(
        "kg.phase prune {:.3}s ({} -> {} trees) [pipelined]",
        phases.prune_s,
        trees_before,
        kept_u32.len(),
    );
    if kept_u32.is_empty() {
        return Err(ExtractError::InvalidInput(format!(
            "no syntax trees reachable from --top {:?}; check the name(s) or your source list",
            inputs.tops
        )));
    }

    let t_walk = std::time::Instant::now();
    let walked = p.session.walk_design()?;
    phases.walk_design_s = t_walk.elapsed().as_secs_f64();
    log::info!(
        "kg.phase walk_design {:.3}s [pipelined]",
        phases.walk_design_s
    );

    let identity = bender_kg_models::DesignIdentity::build(
        &inputs.workspace,
        inputs.targets.clone(),
        p.all_defines.clone(),
        inputs.tops.first().cloned(),
        inputs.design_alias.clone(),
    );

    let raw_modules: Vec<ModuleData> = walked
        .modules
        .iter()
        .map(|m| walk::convert_module(m, &identity.alias))
        .collect();
    let warnings: Vec<String> = walked.warnings.clone();

    let mut by_name: IndexMap<String, ModuleData> = IndexMap::new();
    for m in raw_modules.into_iter() {
        match by_name.get_mut(&m.name) {
            Some(existing)
                if (m.line_start.is_some() && existing.line_start.is_none())
                    || m.instantiations.len() > existing.instantiations.len() =>
            {
                *existing = m;
            }
            Some(_) => {}
            None => {
                by_name.insert(m.name.clone(), m);
            }
        }
    }
    let modules: Vec<ModuleData> = by_name.into_values().collect();

    // Stream un-enriched IR.
    let t_ir = std::time::Instant::now();
    let manifest = emit::build_manifest(
        identity.clone(),
        &modules,
        p.file_count,
        p.srclist_hash,
        warnings,
    );
    emit::stream(sink, &manifest, &modules)?;
    phases.ir_write_s = t_ir.elapsed().as_secs_f64();
    log::info!("kg.phase ir_write {:.3}s [pipelined]", phases.ir_write_s);

    let handle = if inputs.elab {
        let parse::ParseOutcome { session, .. } = p;
        Some(ElabHandle {
            session,
            tops: inputs.tops.clone(),
            design: identity.alias,
        })
    } else {
        None
    };

    Ok((manifest, modules, phases, handle))
}

/// One-shot advisory printed when the caller sets `parse_jobs > 1`. We
/// don't run parallel parsing yet because pruning needs a single slang
/// session; this keeps the user informed that the flag is a no-op.
fn log_parse_jobs_advisory(inputs: &ExtractInputs) {
    if inputs.parse_jobs > 1 {
        log::warn!(
            "kg.parse_jobs={} ignored: parallel parsing is incompatible with --top pruning \
             (slang sessions cannot share trees today). Falling back to single-threaded parse.",
            inputs.parse_jobs
        );
    }
}

/// Convenience: write the IR straight to a JSONL file path.
pub fn extract_to_jsonl(inputs: &ExtractInputs, out_path: &PathBuf) -> Result<Manifest> {
    use std::fs::File;
    use std::io::BufWriter;
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    use std::io::Write;
    let f = File::create(out_path)?;
    let mut bw = BufWriter::new(f);
    let (m, _phases) = extract(inputs, &mut bw)?;
    bw.flush()?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn invalid_inputs_rejected() {
        let mut sink = VecSink::default();
        let inputs = ExtractInputs::default();
        let r = extract(&inputs, &mut sink);
        assert!(r.is_err());
    }

    fn write_temp(dir: &std::path::Path, name: &str, body: &str) -> String {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn single_unit_propagates_macros_across_groups() {
        let tmp = tempfile::tempdir().unwrap();
        // Group A: header-only `.sv` defining a function-style macro.
        let header = write_temp(
            tmp.path(),
            "axi_typedef.sv",
            "`define MK_T(N, W) typedef logic [W-1:0] N\n",
        );
        // Group B: uses the macro WITHOUT a `\`include`. With per-group
        // scoping (single_unit=false) this cannot resolve. With single-unit
        // it does.
        let user = write_temp(
            tmp.path(),
            "user.sv",
            "module user;\n  `MK_T(my_t, 8);\nendmodule\n",
        );
        let inputs = |single_unit| ExtractInputs {
            workspace: tmp.path().to_string_lossy().into(),
            targets: vec![],
            tops: vec!["user".into()],
            elab: false,
            design_alias: Some("xunit".into()),
            groups: vec![
                SourceGroupInput {
                    files: vec![header.clone()],
                    include_dirs: vec![],
                    defines: vec![],
                },
                SourceGroupInput {
                    files: vec![user.clone()],
                    include_dirs: vec![],
                    defines: vec![],
                },
            ],
            single_unit,
            lenient: false,
            parse_jobs: 1,
        };

        let mut s_off = VecSink::default();
        let r_off = extract(&inputs(false), &mut s_off);
        assert!(r_off.is_err(), "expected per-group scoping to fail");

        let mut s_on = VecSink::default();
        let r_on = extract(&inputs(true), &mut s_on);
        let (m, _phases) = r_on.expect("expected single-unit to succeed");
        assert!(m.module_count >= 1, "user module must be indexed");
    }
}

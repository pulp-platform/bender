// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Pipeline that turns IR into a populated graph + embedding store.
//!
//! The build always runs in this order:
//!   1. `clear_design` (idempotent reset of any prior state for the alias)
//!   2. `register_design`
//!   3. `Store::upsert_modules` — one Grafeo transaction; child stubs
//!      created by a parent's instantiation can be upgraded to full nodes
//!      by their own MERGE within the same transaction.
//!   4. `Store::upsert_embedding` per module — embeddings are stored as
//!      `Module.embedding` properties and auto-indexed by Grafeo's HNSW.

use std::path::Path;
use std::time::Instant;

use bender_kg_extract::{ExtractInputs, VecSink};
use bender_kg_models::{IrRecord, Manifest, ModuleData};

use crate::module_document;
use crate::{BuildOutcome, Engine, Result};

impl Engine {
    /// End-to-end build: extract -> persist IR + manifest -> upsert into
    /// store -> embed + index. Idempotent.
    ///
    /// When `inputs.elab && cfg.pipeline_elab`, slang's `walk_elaborated`
    /// runs on a worker thread in parallel with the base
    /// `Store::upsert_modules` call; resolved param values + port widths
    /// are patched onto the existing `INSTANTIATES` edges via a targeted
    /// UNWIND SET after both finish. Otherwise the simpler sequential
    /// path is used.
    pub async fn build(&mut self, inputs: &ExtractInputs) -> Result<BuildOutcome> {
        let t_total = Instant::now();
        if inputs.elab && self.cfg.pipeline_elab {
            self.build_pipelined(inputs, t_total).await
        } else {
            self.build_sequential(inputs, t_total).await
        }
    }

    async fn build_sequential(
        &mut self,
        inputs: &ExtractInputs,
        t_total: Instant,
    ) -> Result<BuildOutcome> {
        let mut sink = VecSink::default();
        let (manifest, mut phases) = bender_kg_extract::extract(inputs, &mut sink)?;
        self.persist_extract_artifacts(&sink.records, &manifest)?;
        self.reset_design(&manifest)?;

        let modules: Vec<&ModuleData> = sink
            .records
            .iter()
            .filter_map(|r| match r {
                IrRecord::Module(m) => Some(m),
                _ => None,
            })
            .collect();
        let t_upsert = Instant::now();
        let modules_indexed = self.store.upsert_modules(modules.iter().copied())?;
        phases.store_upsert_s = t_upsert.elapsed().as_secs_f64();
        log::info!(
            "kg.phase store_upsert {:.3}s ({} modules)",
            phases.store_upsert_s,
            modules_indexed
        );

        let t_embed = Instant::now();
        let embeddings_indexed = self.embed_and_index(&modules)?;
        phases.embed_s = t_embed.elapsed().as_secs_f64();
        if embeddings_indexed > 0 {
            log::info!(
                "kg.phase embed {:.3}s ({} vectors)",
                phases.embed_s,
                embeddings_indexed
            );
        }

        phases.total_s = t_total.elapsed().as_secs_f64();
        log::info!("kg.phase total {:.3}s", phases.total_s);
        Ok(BuildOutcome {
            manifest,
            modules_indexed,
            embeddings_indexed,
            phases,
        })
    }

    /// Pipelined build path. Runs slang elaboration on a worker thread
    /// while the main thread drives the base graph upsert; merges
    /// resolved values via UNWIND SET once both finish. Time savings
    /// scale with the smaller of the two phase times. For large designs,
    /// `--elab` can add tens of seconds of slang work that would otherwise
    /// block upsert; with pipelining the wall-clock is bounded by
    /// `max(elab, upsert)`.
    async fn build_pipelined(
        &mut self,
        inputs: &ExtractInputs,
        t_total: Instant,
    ) -> Result<BuildOutcome> {
        let mut sink = VecSink::default();
        let (manifest, modules, mut phases, handle_opt) =
            bender_kg_extract::extract_pipelined(inputs, &mut sink)?;
        self.persist_extract_artifacts(&sink.records, &manifest)?;
        self.reset_design(&manifest)?;

        let handle = handle_opt
            .expect("extract_pipelined must yield an ElabHandle when inputs.elab is true");

        let store = &self.store;
        let module_refs: Vec<&ModuleData> = modules.iter().collect();
        let t_par = Instant::now();
        let (upsert_res, elab_res) = std::thread::scope(|s| {
            let upsert_handle = s.spawn(|| store.upsert_modules(module_refs.iter().copied()));
            let elab_handle = s.spawn(move || handle.run());
            (
                upsert_handle.join().expect("kg upsert worker panicked"),
                elab_handle.join().expect("kg elab worker panicked"),
            )
        });
        let modules_indexed = upsert_res?;
        let (resolved_updates, elab_warnings) = elab_res?;
        let par_elapsed = t_par.elapsed().as_secs_f64();
        // Both phases ran concurrently; report the wall-clock of the
        // longer of the two as the "elab" + "upsert" cost. Without
        // per-thread timers we attribute the parallel time to upsert
        // (the I/O-bound side) and leave `elaborate_s` at zero so
        // bench summaries make the parallelism visible.
        phases.store_upsert_s = par_elapsed;
        log::info!(
            "kg.phase parallel(upsert,elab) {:.3}s ({} modules, {} resolved updates, {} warnings)",
            par_elapsed,
            modules_indexed,
            resolved_updates.len(),
            elab_warnings.len()
        );

        if !resolved_updates.is_empty() {
            let t_apply = Instant::now();
            let n = self.store.update_resolved_edges(&resolved_updates)?;
            log::info!(
                "kg.phase apply_resolved {:.3}s ({} edge updates)",
                t_apply.elapsed().as_secs_f64(),
                n,
            );
        }

        let t_embed = Instant::now();
        let embeddings_indexed = self.embed_and_index(&module_refs)?;
        phases.embed_s = t_embed.elapsed().as_secs_f64();
        if embeddings_indexed > 0 {
            log::info!(
                "kg.phase embed {:.3}s ({} vectors)",
                phases.embed_s,
                embeddings_indexed
            );
        }

        phases.total_s = t_total.elapsed().as_secs_f64();
        log::info!("kg.phase total {:.3}s [pipelined]", phases.total_s);
        Ok(BuildOutcome {
            manifest,
            modules_indexed,
            embeddings_indexed,
            phases,
        })
    }

    /// Load IR JSONL into the store + embedding index without
    /// re-extracting.
    pub async fn index_from_jsonl(&mut self, jsonl_path: impl AsRef<Path>) -> Result<usize> {
        let f = std::fs::File::open(&jsonl_path)?;
        let r = std::io::BufReader::new(f);
        let mut design_alias: Option<String> = None;
        let mut modules: Vec<ModuleData> = Vec::new();
        for rec in bender_kg_models::read_ir_jsonl(r) {
            match rec? {
                IrRecord::Manifest(m) => {
                    self.reset_design(&m)?;
                    design_alias = Some(m.identity.alias.clone());
                }
                IrRecord::Module(mut m) => {
                    if m.design.is_empty() {
                        if let Some(a) = &design_alias {
                            m.design = a.clone();
                        }
                    }
                    modules.push(m);
                }
            }
        }
        let count = self.store.upsert_modules(modules.iter())?;
        let refs: Vec<&ModuleData> = modules.iter().collect();
        self.embed_and_index(&refs)?;
        Ok(count)
    }

    pub async fn clear_design(&mut self, alias: &str) -> Result<()> {
        // Store::clear_design wipes both module nodes and their embeddings
        // since the embedding lives as a node property.
        self.store.clear_design(alias)?;
        Ok(())
    }

    pub async fn clear_all(&mut self) -> Result<()> {
        self.store.clear_all()?;
        for p in [self.cfg.manifest_path(), self.cfg.ir_path()] {
            if p.exists() {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(())
    }

    // ----- helpers ----------------------------------------------------------

    fn persist_extract_artifacts(&self, records: &[IrRecord], manifest: &Manifest) -> Result<()> {
        write_ir(records, &self.cfg.ir_path())?;
        std::fs::write(
            self.cfg.manifest_path(),
            serde_json::to_string_pretty(manifest)?,
        )?;
        Ok(())
    }

    fn reset_design(&mut self, manifest: &Manifest) -> Result<()> {
        let alias = manifest.identity.alias.clone();
        self.store.clear_design(&alias).ok();
        self.store.register_design(
            &alias,
            &manifest.identity.id,
            Some(manifest.identity.workspace.as_str()),
            manifest.identity.top.as_deref(),
            &manifest.identity.targets,
            &manifest.identity.defines,
        )?;
        Ok(())
    }

    fn embed_and_index(&self, modules: &[&ModuleData]) -> Result<usize> {
        if self.cfg.skip_embeddings || modules.is_empty() {
            return Ok(0);
        }
        let model = self.embedder.model().to_string();
        let mut n = 0;
        for m in modules {
            let v = self.embedder.embed_one(&module_document(m))?;
            self.store
                .upsert_embedding(&m.design, &m.name, &v, &model)?;
            n += 1;
        }
        Ok(n)
    }
}

fn write_ir(records: &[IrRecord], path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = std::io::BufWriter::new(std::fs::File::create(path)?);
    for rec in records {
        bender_kg_models::write_ir_record(&mut writer, rec)?;
    }
    std::io::Write::flush(&mut writer)?;
    Ok(())
}

// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Hybrid keyword + semantic search over the Grafeo store.
//!
//! `search_modules` fuses a BM25 keyword arm (over `Module.searchable_text`)
//! with the HNSW vector arm (`Module.embedding`) via RRF, so exact/substring
//! identifier matches rank reliably (grep-precision) while semantic neighbours
//! still surface. Each hit is hydrated from the graph store so callers always
//! get a self-contained [`crate::ModuleSearchResult`] (name + score + source
//! metadata). Batch search dedupes by module name and keeps the best score.

use indexmap::IndexMap;

use crate::{Engine, ModuleSearchResult, Result};

impl Engine {
    /// Hybrid search. `top_k == 0` means "all hits above `min_score`"
    /// (enumeration); `min_score` (`None` = keep all) drops weak fused hits.
    pub async fn search_modules(
        &self,
        query: &str,
        top_k: usize,
        min_score: Option<f32>,
        design: Option<&str>,
    ) -> Result<Vec<ModuleSearchResult>> {
        let qv = self.embedder.embed_one(query)?;
        let hits = self
            .store
            .search_modules_hybrid(query, &qv, top_k, min_score, design)?;
        let mut out = Vec::with_capacity(hits.len());
        for h in hits {
            if let Some(m) = self.store.get_module(&h.module)? {
                out.push(ModuleSearchResult {
                    name: m.name,
                    score: h.score,
                    file_path: m.file_path,
                    design: m.design,
                    description: m.description.unwrap_or_default(),
                    num_ports: m.ports.len(),
                    num_params: m.parameters.len(),
                    num_instantiations: m.instantiations.len(),
                });
            }
        }
        Ok(out)
    }

    pub async fn search_modules_batch(
        &self,
        queries: &[String],
        top_k: usize,
        min_score: Option<f32>,
        design: Option<&str>,
    ) -> Result<Vec<ModuleSearchResult>> {
        let mut by_name: IndexMap<String, ModuleSearchResult> = IndexMap::new();
        for q in queries {
            for r in self.search_modules(q, top_k, min_score, design).await? {
                let entry = by_name.entry(r.name.clone()).or_insert_with(|| r.clone());
                if r.score > entry.score {
                    *entry = r;
                }
            }
        }
        let mut out: Vec<ModuleSearchResult> = by_name.into_values().collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::{CoreConfig, Engine, module_document};
    use bender_kg_models::ModuleData;

    async fn seed_module(eng: &Engine, name: &str) {
        let mut m = ModuleData::default();
        m.name = name.into();
        m.design = "d".into();
        eng.store.upsert_module(&m).unwrap();
        let v = eng.embedder.embed_one(&module_document(&m)).unwrap();
        eng.store
            .upsert_embedding(&m.design, &m.name, &v, eng.embedder.model())
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn search_returns_self_for_seeded_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = CoreConfig::new(tmp.path());
        cfg.embed.force_hash = true;
        let eng = Engine::open(cfg).await.unwrap();
        eng.store
            .register_design("d", "ID", None, None, &["rtl".to_string()], &[])
            .unwrap();
        seed_module(&eng, "tt_fpu_v2").await;
        let hits = eng.search_modules("tt_fpu_v2", 5, None, None).await.unwrap();
        assert_eq!(hits[0].name, "tt_fpu_v2");
    }

    /// Hybrid grep-precision: an exact identifier query ranks the exact
    /// module first even when another module is a close semantic/lexical
    /// neighbour. The BM25 keyword arm carries this; a pure-vector search
    /// with the crude hash embedder could otherwise tie or invert it.
    #[tokio::test(flavor = "current_thread")]
    async fn exact_name_beats_neighbour() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = CoreConfig::new(tmp.path());
        cfg.embed.force_hash = true;
        let eng = Engine::open(cfg).await.unwrap();
        eng.store
            .register_design("d", "ID", None, None, &["rtl".to_string()], &[])
            .unwrap();
        seed_module(&eng, "axi_cdc_fifo").await;
        seed_module(&eng, "axi_cdc_fifo_gray").await;
        let hits = eng
            .search_modules("axi_cdc_fifo", 5, None, None)
            .await
            .unwrap();
        assert_eq!(hits[0].name, "axi_cdc_fifo");
    }

    /// `top_k == 0` is unbounded: every seeded module comes back.
    #[tokio::test(flavor = "current_thread")]
    async fn top_k_zero_is_unbounded() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = CoreConfig::new(tmp.path());
        cfg.embed.force_hash = true;
        let eng = Engine::open(cfg).await.unwrap();
        eng.store
            .register_design("d", "ID", None, None, &["rtl".to_string()], &[])
            .unwrap();
        for i in 0..20 {
            seed_module(&eng, &format!("cdc_mod_{i}")).await;
        }
        let hits = eng.search_modules("cdc", 0, None, None).await.unwrap();
        assert_eq!(hits.len(), 20);
    }

    /// A bounded `top_k` with a design filter must still fill up to `top_k`
    /// from the target design even when another design has many near hits
    /// that would otherwise consume the (un-over-fetched) slots.
    #[tokio::test(flavor = "current_thread")]
    async fn design_filter_fills_bounded_top_k() {
        async fn seed(eng: &Engine, name: &str, design: &str) {
            let mut m = bender_kg_models::ModuleData::default();
            m.name = name.into();
            m.design = design.into();
            eng.store.upsert_module(&m).unwrap();
            let v = eng.embedder.embed_one(&module_document(&m)).unwrap();
            eng.store
                .upsert_embedding(&m.design, &m.name, &v, eng.embedder.model())
                .unwrap();
        }
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = CoreConfig::new(tmp.path());
        cfg.embed.force_hash = true;
        let eng = Engine::open(cfg).await.unwrap();
        eng.store
            .register_design("keep", "IDK", None, None, &["rtl".to_string()], &[])
            .unwrap();
        eng.store
            .register_design("other", "IDO", None, None, &["rtl".to_string()], &[])
            .unwrap();
        // Many "other"-design hits, a few "keep"-design hits.
        for i in 0..30 {
            seed(&eng, &format!("fifo_other_{i}"), "other").await;
        }
        for i in 0..5 {
            seed(&eng, &format!("fifo_keep_{i}"), "keep").await;
        }
        let hits = eng
            .search_modules("fifo", 5, None, Some("keep"))
            .await
            .unwrap();
        assert_eq!(hits.len(), 5, "over-fetch should fill top_k from 'keep'");
        assert!(hits.iter().all(|h| h.design == "keep"));
    }
}

// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Semantic search over the Grafeo HNSW vector index.
//!
//! Each hit is hydrated from the graph store so callers always get a
//! self-contained [`crate::ModuleSearchResult`] (name + score + source
//! metadata). Batch search dedupes by module name and keeps the best score.

use indexmap::IndexMap;

use crate::{Engine, ModuleSearchResult, Result};

impl Engine {
    pub async fn search_modules(
        &self,
        query: &str,
        top_k: usize,
        design: Option<&str>,
    ) -> Result<Vec<ModuleSearchResult>> {
        let qv = self.embedder.embed_one(query)?;
        let hits = self.store.search_modules_by_vector(&qv, top_k, design)?;
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
        design: Option<&str>,
    ) -> Result<Vec<ModuleSearchResult>> {
        let mut by_name: IndexMap<String, ModuleSearchResult> = IndexMap::new();
        for q in queries {
            for r in self.search_modules(q, top_k, design).await? {
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

    #[tokio::test(flavor = "current_thread")]
    async fn search_returns_self_for_seeded_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = CoreConfig::new(tmp.path());
        cfg.embed.force_hash = true;
        let eng = Engine::open(cfg).await.unwrap();
        let mut m = ModuleData::default();
        m.name = "tt_fpu_v2".into();
        m.design = "d".into();
        eng.store
            .register_design("d", "ID", None, None, &["rtl".to_string()], &[])
            .unwrap();
        eng.store.upsert_module(&m).unwrap();
        let v = eng.embedder.embed_one(&module_document(&m)).unwrap();
        eng.store
            .upsert_embedding(&m.design, &m.name, &v, eng.embedder.model())
            .unwrap();
        let hits = eng.search_modules("tt_fpu_v2", 5, None).await.unwrap();
        assert_eq!(hits[0].name, "tt_fpu_v2");
    }
}

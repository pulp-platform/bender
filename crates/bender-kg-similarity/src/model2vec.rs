// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! `model2vec-rs` backend. Pure-Rust static embeddings; no ONNX, no glibc dep.

use model2vec_rs::model::StaticModel;

use crate::{EmbedError, Embedder, Result};

/// Real text embedder backed by [`model2vec_rs::model::StaticModel`].
pub struct Model2VecEmbedder {
    inner: StaticModel,
    dim: usize,
    model_id: String,
}

impl Model2VecEmbedder {
    /// Load a model from a HuggingFace repo id (e.g. `minishlab/potion-base-8M`)
    /// or a local directory containing `tokenizer.json`, `model.safetensors`,
    /// and `config.json`.
    pub fn load(repo_or_path: &str) -> Result<Self> {
        let inner = StaticModel::from_pretrained(repo_or_path, None, None, None)
            .map_err(|e| EmbedError::Init(format!("model2vec load {repo_or_path}: {e}")))?;
        // Probe the embedding dim with a single-token call. Cheap (<1 ms).
        let probe = inner.encode(&["probe".to_string()]);
        let dim = probe
            .first()
            .map(|v| v.len())
            .ok_or_else(|| EmbedError::Init("model2vec returned no embeddings on probe".into()))?;
        Ok(Self {
            inner,
            dim,
            model_id: repo_or_path.to_string(),
        })
    }
}

impl Embedder for Model2VecEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn model(&self) -> &str {
        &self.model_id
    }

    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.inner.encode_single(text))
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(self.inner.encode(texts))
    }
}

// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! Text -> dense-vector embedding adapter.
//!
//! Two backends:
//! - [`HashEmbedder`]: deterministic, no model download, fine for tests and a
//!   degraded `bender kg search` flow. Uses signed feature hashing.
//! - `Model2VecEmbedder` (`model2vec` feature, default-on): pure-Rust static
//!   embeddings via [`model2vec-rs`](https://docs.rs/model2vec-rs). No ONNX,
//!   no glibc dependency, fast on CPU. Default model is
//!   `minishlab/potion-base-8M` (256-dim).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "model2vec")]
mod model2vec;
#[cfg(feature = "model2vec")]
pub use model2vec::Model2VecEmbedder;

/// Default dimensionality of the deterministic-fallback embedder. Matches
/// `model2vec` `minishlab/potion-base-8M` so the two backends are
/// interchangeable for downstream code that fixes a dimension.
pub const DEFAULT_DIM: usize = 256;

/// Default model id resolved by the `model2vec` backend.
pub const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("embed init error: {0}")]
    Init(String),
    #[error("embed runtime error: {0}")]
    Runtime(String),
}

pub type Result<T> = std::result::Result<T, EmbedError>;

/// Configuration for [`build`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedConfig {
    /// Embedding dimensionality (only used by the [`HashEmbedder`] fallback;
    /// model2vec inherits dim from the loaded model).
    pub dim: usize,
    /// Model id. For the `model2vec` backend this is a HuggingFace repo id
    /// (e.g. `minishlab/potion-base-8M`) or a local path containing
    /// `tokenizer.json`, `model.safetensors`, `config.json`.
    pub model: String,
    /// Force the deterministic [`HashEmbedder`] even when `model2vec` is
    /// compiled in. Useful for tests and offline CI.
    pub force_hash: bool,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            dim: DEFAULT_DIM,
            model: DEFAULT_MODEL.to_string(),
            force_hash: false,
        }
    }
}

/// Generic embedder interface.
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;
    fn model(&self) -> &str;
    fn embed_one(&self, text: &str) -> Result<Vec<f32>>;
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed_one(t)).collect()
    }
}

/// Deterministic hash-based embedder. Produces unit-norm vectors of length
/// `dim` via signed feature hashing on whitespace tokens.
pub struct HashEmbedder {
    dim: usize,
    model: String,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            model: format!("hash-fallback@{dim}"),
        }
    }
}

impl Embedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut buckets = vec![0.0f32; self.dim];
        for tok in text
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty())
        {
            let h = sha256_u64(tok);
            let bin = (h as usize) % self.dim;
            // Sign from a separate bit so collisions don't always reinforce.
            let sign = if (h >> 32) & 1 == 0 { 1.0 } else { -1.0 };
            buckets[bin] += sign;
        }
        let norm: f32 = buckets.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in buckets.iter_mut() {
                *v /= norm;
            }
        }
        Ok(buckets)
    }
}

fn sha256_u64(s: &str) -> u64 {
    let d = Sha256::digest(s.as_bytes());
    u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
}

/// Build the configured embedder.
///
/// Resolution order:
/// 1. If `force_hash` is set, always [`HashEmbedder`].
/// 2. If the `model2vec` feature is enabled, attempt to load
///    [`Model2VecEmbedder`]. On load failure we log nothing (this crate
///    has no logger dep) and fall back to [`HashEmbedder`].
/// 3. Otherwise [`HashEmbedder`] at `cfg.dim`.
pub fn build(cfg: &EmbedConfig) -> Result<Box<dyn Embedder>> {
    if cfg.force_hash {
        return Ok(Box::new(HashEmbedder::new(cfg.dim)));
    }
    #[cfg(feature = "model2vec")]
    {
        match Model2VecEmbedder::load(&cfg.model) {
            Ok(e) => return Ok(Box::new(e)),
            Err(_) => {
                // Model load can fail offline or on bad path; fall through.
            }
        }
    }
    Ok(Box::new(HashEmbedder::new(cfg.dim)))
}

/// Cosine similarity between two same-length vectors. Returns 0 on length
/// mismatch or zero-magnitude inputs.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_embedder_is_deterministic() {
        let e = HashEmbedder::new(64);
        let a = e.embed_one("clock domain crossing fifo").unwrap();
        let b = e.embed_one("clock domain crossing fifo").unwrap();
        assert_eq!(a, b);
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn similar_texts_score_higher_than_unrelated() {
        let e = HashEmbedder::new(256);
        let a = e.embed_one("axi master interface module").unwrap();
        let b = e.embed_one("axi master interface").unwrap();
        let c = e.embed_one("totally unrelated thing about coffee").unwrap();
        assert!(cosine(&a, &b) > cosine(&a, &c));
    }

    #[test]
    fn force_hash_returns_hash_backend_under_default_features() {
        let cfg = EmbedConfig {
            force_hash: true,
            ..EmbedConfig::default()
        };
        let e = build(&cfg).unwrap();
        assert!(e.model().starts_with("hash-fallback@"));
        assert_eq!(e.dim(), DEFAULT_DIM);
    }
}

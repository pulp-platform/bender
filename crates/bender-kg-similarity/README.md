# bender-kg-similarity

> **Internal crate:** `bender-kg-similarity` is an internal crate of [Bender](https://github.com/pulp-platform/bender). It does not provide a stable public API — breaking changes may occur at any time without notice.

`bender-kg-similarity` is the embedding adapter for the `bender kg` subsystem. It converts a module's textual representation into a dense vector that is stored in the HNSW index inside `bender-kg-store` and used for semantic module search.

## Responsibilities

- Accept a `ModuleData` record and produce a fixed-dimension `f32` embedding vector.
- Provide a configurable backend (currently a local ONNX model via `ort`).
- Be called by `bender-kg-core` during `bender kg build` and incremental updates.

The embedding model path and dimension are configured through `CoreConfig`, which is managed by `bender-kg-core`.

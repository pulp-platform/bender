# bender-kg-store

> **Internal crate:** `bender-kg-store` is an internal crate of [Bender](https://github.com/pulp-platform/bender). It does not provide a stable public API — breaking changes may occur at any time without notice.

`bender-kg-store` is the persistence layer of the `bender kg` subsystem. It manages a Grafeo graph database (property graph + HNSW vector index + BM25 text index) that stores the extracted design knowledge.

## Responsibilities

- **Module nodes** — upsert and retrieve `ModuleData` records in the graph, including ports, parameters, and import lists serialised as JSON blobs.
- **Instantiation edges** — store and query `INSTANTIATES` relationships with parameter and port binding metadata.
- **Vector embeddings** — store and nearest-neighbour search over dense module embeddings for semantic and structural similarity.
- **Graph traversal** — BFS/DFS helpers (`graph.rs`) for hierarchy path-finding and parent/child queries.
- **Parameter and signal tracing** — `param.rs` utilities for matching parameter references and signal identifiers across module boundaries.

The database files are written to `~/.local/share/bender/kg/` by default.

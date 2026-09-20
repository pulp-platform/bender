# bender-kg-core

> **Internal crate:** `bender-kg-core` is an internal crate of [Bender](https://github.com/pulp-platform/bender). It does not provide a stable public API — breaking changes may occur at any time without notice.

`bender-kg-core` is the orchestration layer of the `bender kg` subsystem. It composes extraction (`bender-kg-extract`), storage (`bender-kg-store`), and embedding (`bender-kg-similarity`) into a single typed API consumed by both the CLI (`bender kg query`) and the MCP server (`bender-kg-mcp`).

## Responsibilities

- **Build / update** — drive extraction and incremental ingestion into the graph and vector stores.
- **Query** — expose a typed `Engine` with methods for every supported query operation:
  - `search_modules` / `search_modules_batch` — semantic and keyword search.
  - `get_module` / `get_subgraph` / `get_instance_context` — module inspection.
  - `get_parents` / `get_children` — hierarchy navigation.
  - `get_ports` / `find_by_protocol` / `match_interfaces` — port and protocol analysis.
  - `get_source_snippet` — source location lookup.
  - `trace_hierarchy_path` — shortest instantiation path between two modules.
  - `trace_parameter` / `trace_parameter_recursive` — parameter dataflow tracing.
  - `trace_signal` / `trace_signal_recursive` — signal connectivity tracing.
  - `check_connectivity` — reachability and port binding validation.
  - `find_structurally_similar` — structural similarity search.
  - `graph_stats` — database statistics.

Configuration (database path, embedding model, etc.) is provided through `CoreConfig`.

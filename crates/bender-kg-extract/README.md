# bender-kg-extract

> **Internal crate:** `bender-kg-extract` is an internal crate of [Bender](https://github.com/pulp-platform/bender). It does not provide a stable public API — breaking changes may occur at any time without notice.

`bender-kg-extract` implements the SystemVerilog → knowledge-graph extraction pipeline. It drives `bender-slang` to parse source files and converts the resulting Slang AST into `bender-kg-models` IR types that can be ingested by `bender-kg-store`.

## Pipeline

1. **Parse** — invoke Slang on the source files collected by `bender sources`.
2. **Extract** — walk the Slang AST to collect module declarations, port lists, parameter declarations, package imports, and instantiation edges.
3. **Emit** — write IR records to a caller-supplied `IrSink` (in-memory or JSONL file).

The main entry points are:
- `extract()` — single-threaded extraction to a sink.
- `extract_pipelined()` — multi-threaded extraction with a worker pool.
- `extract_to_jsonl()` — convenience wrapper that writes IR to a JSONL manifest file.

# bender-kg-mcp

> **Internal crate:** `bender-kg-mcp` is an internal crate of [Bender](https://github.com/pulp-platform/bender). It does not provide a stable public API — breaking changes may occur at any time without notice.

`bender-kg-mcp` exposes the `bender kg` query surface as an [MCP](https://modelcontextprotocol.io/) server over stdio. It is launched by `bender kg mcp-server` and lets AI assistants (Claude, Cursor, etc.) query the design knowledge graph directly.

## MCP Tools

| Tool | Description |
|------|-------------|
| `search_modules` | Semantic and keyword search for modules |
| `search_modules_batch` | Batch variant of `search_modules` |
| `get_module` | Full module record: ports, parameters, imports |
| `get_subgraph` | Instantiation sub-graph rooted at a module |
| `get_instance_context` | Instance binding details for a specific instantiation |
| `get_parents` | Modules that instantiate a given module |
| `get_children` | Modules instantiated by a given module |
| `get_ports` | Port list with widths and types |
| `find_by_protocol` | Find modules by port protocol pattern |
| `get_source_snippet` | Source file excerpt for a module |
| `trace_hierarchy_path` | Shortest instantiation path between two modules |
| `check_connectivity` | Validate port connectivity between modules |
| `trace_parameter` | Trace parameter propagation (optionally recursive) |
| `trace_signal` | Trace signal connectivity (optionally recursive) |
| `match_interfaces` | Match port lists across candidate modules |
| `find_structurally_similar` | Structurally similar module candidates |
| `graph_stats` | Database statistics |

## Usage

The server is started automatically when an MCP client connects via `bender kg mcp-server`. It communicates over stdin/stdout using the MCP JSON-RPC protocol.

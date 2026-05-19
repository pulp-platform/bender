# bender-kg-models

> **Internal crate:** `bender-kg-models` is an internal crate of [Bender](https://github.com/pulp-platform/bender). It does not provide a stable public API — breaking changes may occur at any time without notice.

`bender-kg-models` defines the intermediate representation (IR) types shared across the knowledge-graph subsystem. All other `bender-kg-*` crates depend on these types.

## Types

- **`ModuleData`** — a parsed SystemVerilog module: name, design, file path, ports, parameters, instantiations, and package imports.
- **`PortInfo`** — port metadata: name, direction (`Direction`), type string, struct breakdown, and resolved width.
- **`ParamInfo`** — parameter metadata: name, kind (`ParamKind`), and default value.
- **`ImportInfo`** — a `import pkg::*;` or selective import statement.
- **`InstantiationInfo`** — an instantiation edge: parent module, child module, instance name, parameter bindings, and port bindings.

All types implement `serde::Serialize` / `serde::Deserialize` and are stored in the Grafeo graph database by `bender-kg-store`.

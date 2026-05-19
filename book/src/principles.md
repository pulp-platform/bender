# Principles

Bender is built around two core principles, supported by three feature tiers that build on each other.

## Be as opt-in as possible

Bender does not assume a specific EDA tool, workflow, or directory layout beyond a few key files. All features are designed to be modular, so they can be picked up individually and integrated into an existing flow. As long as a [`Bender.yml`](./manifest.md) is present, Bender can manage the package.

## Allow for reproducible builds

Bender maintains a precise [lockfile](./lockfile.md) which records the exact Git revision each dependency was resolved to. Committing this file alongside the manifest lets the exact source state of a package — for example at a tape-out or a release — be reconstructed after the fact.

---

## Feature Tiers

Bender's functionality is organized into three tiers, each building on the previous one. A package only needs to opt into the tiers it uses.

### Tier 1: Source Collection

Collect and organize the HDL source files of a hardware IP:

- Maintain the required order across files, e.g. for package declarations before their use.
- Stay language-agnostic across SystemVerilog and VHDL.
- Allow files to be organized into recursive groups.
- Track defines and include directories individually for each group.

See [Sources](./sources.md) for the manifest format.

### Tier 2: Dependency Management

Manage other packages an IP depends on and provide a local checkout of their sources:

- Support transitive dependencies.
- Resolve dependencies directly from Git rather than a central package registry. Projects containing IP under NDA can therefore use private repositories or local paths without exposing them.
- Use [Semantic Versioning](https://semver.org/) to constrain compatible revisions.

See [Dependencies](./dependencies.md) for details.

### Tier 3: Tool Script Generation

Generate source file listings and compilation scripts for various EDA tools, so the same set of resolved sources can be fed into simulation, synthesis, and downstream flows without manually maintaining tool-specific file lists.

See [Generating Tool Scripts](./workflow/scripts.md) for the supported formats and options.

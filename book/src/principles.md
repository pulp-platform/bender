# Principles

Bender is built around the following core principles:

- **Be as opt-in as possible.** We do not assume any specific EDA tool, workflow, or directory layout (besides a few key files). All features are designed to be as modular as possible, such that the user can integrate them into their respective flow.

- **Allow for reproducible builds.** Bender maintains a precise *lock file* which tracks the exact git hash a dependency has been resolved to. This allows the source code of a package to be reliable reconstructed after the fact.

- **Collect source files.** The first feature tier of Bender is to collect the source files in a hardware IP. In doing this, it shall do the following:
  - Maintain the required order across source files, e.g. for package declarations before their use.
  - Be as language-agnostic as possible, supporting both SystemVerilog and VHDL.
  - Allow source files to be organized into recursive groups.
  - Track defines and include directories individually for each group.

- **Manage dependencies.** The second feature tier of Bender is to maintain other packages an IP may depend on, and to provide a local checkout of the necessary source files. Specifically, it shall:
  - Support transitive dependencies
  - Not rely on a central package registry, unlike e.g. npm, cargo, or brew (necessary because parts of a project are usually under NDA)
  - Enforce strict use of [semantic versioning](https://semver.org/)

- **Generate tool scripts.** The third feature tier of Bender is the ability to generate source file listings and compilation scripts for various tools.

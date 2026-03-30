# Bender Documentation

Welcome to the official documentation for **Bender**, a dependency management tool specifically designed for hardware design projects.

Bender helps you manage complex hardware IP hierarchies, ensuring that every member of your team and every CI runner is working with the exact same source code.

## Key Features

- **Hierarchical Dependency Management:** Resolve and manage transitive dependencies across multiple Git repositories or local paths.
- **Reproducible Builds:** A precise lockfile mechanism (`Bender.lock`) ensures bit-identical design states across environments.
- **HDL-Aware Source Collection:** Automatically manages file ordering, include directories, and preprocessor defines for SystemVerilog and VHDL.
- **Target-Based Filtering:** Use powerful boolean expressions to include or exclude files based on your flow (simulation, synthesis, etc.).
- **Local Development Workflow:** Easily modify dependencies in-place using the `clone` and `snapshot` flow without breaking official manifests.
- **Tool Script Generation:** Generate compilation and simulation scripts for major EDA tools like QuestaSim, VCS, Vivado, Verilator, and more.

## Getting Started

If you are new to Bender, we recommend following these steps:

1.  **[Installation](./installation.md):** Get the `bender` binary onto your system.
2.  **[Getting Started](./getting_started.md):** A quick tutorial to create your first Bender project.
3.  **[Concepts](./concepts.md):** Dive deeper into how Bender works under the hood.
4.  **[Workflows](./workflows.md):** Practical guides for daily development tasks.

---

Bender is an open-source project maintained by the [PULP Platform](https://pulp-platform.org/) at ETH Zurich and the University of Bologna.

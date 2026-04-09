# Concepts

This section explains the core ideas and files that make Bender work. Understanding these concepts will help you manage complex hardware projects more effectively.

- **[Principles](./principles.md):** The high-level goals and design philosophy behind Bender.
- **[Manifest](./manifest.md):** How to define your package's metadata, dependencies, and sources.
- **[Lockfile](./lockfile.md):** How Bender ensures reproducible builds across different environments.
- **[Workspace](./local.md):** Overriding settings for your local development workspace.
- **[Comparing the Files](./bender_files.md):** A quick comparison of the three core files (`.yml`, `.lock`, `.local`).
- **[Dependencies](./dependencies.md):** How Bender handles hierarchical and transitive dependencies.
- **[Sources](./sources.md):** Managing HDL source files, include directories, and defines.
- **[Targets](./targets.md):** Using boolean expressions to conditionally include or exclude files and dependencies.

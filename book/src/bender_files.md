# Bender Files

Bender relies on three core files to manage your hardware project. While they all use the YAML format, they serve very different roles in the development lifecycle.

## Comparison Overview

| Feature | `Bender.yml` | `Bender.lock` | `Bender.local` |
| :--- | :--- | :--- | :--- |
| **Role** | Manifest (Intent) | Lockfile (Reality) | Local Overrides |
| **Main Content** | Dependencies & Version Ranges | Exact Git Revisions | Local Paths & Tool Config |
| **Managed By** | User (Manual) | `bender update` (Auto) | User or `bender clone` |
| **Version Control** | **Commit** | **Commit** | **Ignore** (`.gitignore`) |
| **Shared?** | Yes, with everyone | Yes, for reproducibility | No, unique to your machine |

## Summary of Roles

### `Bender.yml` (The Manifest)
This is your **Intent**. It defines the requirements of your package. You use it to specify which other packages you need (e.g., "I need `axi` version `0.21.x`"). It is the only file required to define a Bender package.

### `Bender.lock` (The Lockfile)
This is the **Reality**. It is a machine-generated file that captures the exact state of your dependency tree. It records the specific Git commit hash for every dependency. By committing this file, you ensure that every developer and CI machine works with the exact same source code.

### `Bender.local` (Local Overrides)
This is your **Workspace**. It allows you to override the shared configuration for your local environment. Its most common use is to point a dependency to a local directory (via `bender clone`) so you can modify its code and see the effects immediately in your top-level project.

## Interaction Flow

1.  **Define** your requirements in `Bender.yml`.
2.  Run `bender update` to resolve those requirements into a fixed `Bender.lock`.
3.  Run `bender checkout` to download the exact source code specified in the lockfile.
4.  (Optional) Use `Bender.local` to temporarily swap a dependency for a local version during development.

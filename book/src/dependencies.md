# Dependencies

Bender is designed to manage complex, hierarchical dependency trees for hardware projects. A dependency is any external package that provides its own [`Bender.yml`](./manifest.md) manifest.

## Dependency Types

Dependencies are defined in the `dependencies` section of your [`Bender.yml`](./manifest.md).

### Git Dependencies
Git is the primary way to distribute Bender packages. You can specify them in two ways:

#### Version-based (Recommended)
Bender uses [Semantic Versioning (SemVer)](https://semver.org/) to find the best compatible version. You can use [SemVer operators](https://docs.rs/semver/latest/semver/enum.Op.html) to specify version ranges — the syntax matches Cargo's, see the [Cargo dependency reference](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html) for a more accessible overview:

```yaml
dependencies:
  common_cells: { git: "https://github.com/pulp-platform/common_cells.git", version: "1.21.0" }
  axi: { git: "https://github.com/pulp-platform/axi.git", version: ">=0.23.0, <0.26.0" }
```

If you have defined a [remote](#remotes) in your manifest, you can use the shorthand notation instead:

```yaml
dependencies:
  common_cells: "1.21.0"
  axi: { version: ">=0.23.0, <0.26.0" }
```

> **Note:** Bender only recognizes Git tags that follow the `vX.Y.Z` format (e.g., `v1.2.1`).

#### Revision-based
Use this for specific commits, branches, or tags that don't follow SemVer.
```yaml
dependencies:
  pulp_soc: { git: "https://github.com/pulp-platform/pulp_soc.git", rev: "develop" }
```

### Path Dependencies
Path dependencies point to a local directory. They are never versioned; Bender simply uses the code found at that location.
```yaml
dependencies:
  my_local_ip: { path: "../local_ips/my_ip" }
```

## Remotes

To avoid repeating full Git URLs, you can define `remotes` in your manifest.

### Single Remote
If you only define a single remote, it is automatically treated as the default:

```yaml
remotes:
  pulp: "https://github.com/pulp-platform"

dependencies:
  common_cells: "1.21.0" # Automatically searched in the 'pulp' remote
```

### Multiple Remotes
When using multiple remotes, you must explicitly mark one as the `default` if you want to use shortened dependency syntax:

```yaml
remotes:
  pulp:
    url: "https://github.com/pulp-platform"
    default: true
  openhw: "https://github.com/openhwgroup"

dependencies:
  common_cells: "1.21.0"              # Uses the default 'pulp' remote
  cva6: { version: "4.0.0", remote: openhw } # Explicitly uses 'openhw'
```

### Differing Repository Name

By default, Bender appends the dependency's local name to the remote URL when resolving the Git URL. If the upstream repository is named differently from how you want to refer to the dependency locally, use `upstream_name`. Without this field, a mismatch between a dependency's local name and the `package.name` declared inside the dependency's own [`Bender.yml`](./manifest.md) triggers warning `W11`.

```yaml
remotes:
  pulp: "https://github.com/pulp-platform"

dependencies:
  cells: { version: "1.21.0", upstream_name: "common_cells" }
  # Resolves to https://github.com/pulp-platform/common_cells.git
```

You can also embed the dependency name explicitly in the remote URL using the `{}` placeholder, which is useful for non-trivial URL patterns:

```yaml
remotes:
  pulp: "https://gitlab.example.com/scm/{}.git"
```

## Targets

Dependencies can be conditionally included or configured using targets. For details on how to use target expressions or pass targets to dependencies, see the [Targets](./targets.md) documentation.

> **Note:** A `target` on a dependency only filters that dependency out of *source listings and generated scripts*. It does **not** affect dependency resolution: every dependency declared in [`Bender.yml`](./manifest.md) is still resolved and recorded in [`Bender.lock`](./lockfile.md) regardless of which targets are active.

## Git LFS Support

Bender detects whether a dependency uses **Git Large File Storage (LFS)** via its `.gitattributes` and reacts as follows:

- If LFS is detected and `git-lfs` is installed, Bender configures LFS and pulls the required files automatically.
- If LFS is detected but `git-lfs` is **not** installed, Bender emits warning `W26` and continues the checkout. You may end up with pointer files instead of the actual large files, which can cause downstream build failures — install `git-lfs` to resolve this.
- If LFS is disabled in your configuration (`git_lfs: false`) but the dependency appears to use LFS, Bender emits warning `W27`.
- If the repository does not use LFS, Bender skips LFS operations entirely.

## Submodules

If a dependency contains a `.gitmodules` file, Bender initializes and updates its Git submodules recursively after checkout by default.

Cloning submodules is often the slowest part of fetching dependencies, and submodules frequently hold software or tooling that is irrelevant to the hardware build. You can therefore disable submodule cloning:

- Set `git_submodules: false` in your [configuration](./configuration.md#git_submodules) to skip submodules persistently for a project.
- Pass `--git-submodules false` (or set `BENDER_GIT_SUBMODULES=false`) to skip them for a single invocation. The flag overrides the configured value in either direction.

Only disable submodules when none of your dependencies reference sources that live inside a submodule.

## Version Resolution and the Lockfile

When you run `bender update`, Bender performs the following:
1.  **Resolution:** It scans the entire dependency tree and finds a set of versions that satisfy all constraints.
2.  **Locking:** The exact versions and Git commit hashes are written to [`Bender.lock`](./lockfile.md).

For details on updating dependencies, see [Adding and Updating Dependencies](./workflow/dependencies.md)

**Reproducibility:** Once a [`Bender.lock`](./lockfile.md) exists, running `bender checkout` will always download the exact same code, even if newer compatible versions have been released. Always commit your [`Bender.lock`](./lockfile.md) to version control.

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

> **Note:** By default, Bender only recognizes Git tags that follow the `vX.Y.Z` format (e.g., `v1.2.1`). See [Version Namespaces](#version-namespaces) if you need a different prefix.

#### Version Namespaces

The leading `v` in a version tag is simply the default *prefix*. If you maintain your own versioned fork of an open-source IP — for example to ship internally patched releases — you can tag those releases under your own namespace and depend on them with `version_prefix`:

```yaml
dependencies:
  # Resolves only tags of the form `companyX-v<semver>`, e.g. `companyX-v1.2.0`.
  common_cells: { git: "...", version: "1.21.0", version_prefix: "companyX-v" }
```

You can equivalently embed the prefix directly in the version string:

```yaml
dependencies:
  common_cells: { git: "...", version: "companyX-v1.21.0" }
```

The embedded form requires the version requirement to begin with a number (e.g. `companyX-v1.21.0`, `companyX-v1.*`). For operator-based ranges such as `>=1.21.0`, use the `version_prefix` field alongside a plain `version`. If both a field and an embedded prefix are given, they must agree.

The prefix is the entire literal string preceding the semantic version, so you are free to choose any convention (`companyX-v`, `acme-`, …). Setting `version_prefix: ""` selects tags carrying no prefix at all (`1.2.0` rather than `v1.2.0`). A dependency without a prefix keeps the default `v`, so existing manifests and lockfiles are unaffected.

Prefixes only apply to Git version dependencies. On a path or revision dependency the field has nothing to act on, so Bender rejects it rather than ignoring it silently — the same treatment `version` and `rev` get where they cannot apply. A dependency given only a `version` resolves through the default remote and is a Git version dependency, so `version_prefix` is accepted there too.

Namespaces are **strict and never mix**:

- A dependency resolves *only* tags carrying its own prefix. There is no fallback to the default `v` namespace (or any other).
- A dependency pinned to a custom namespace never sees releases in the default one. `bender audit --check-upstream` reports the highest default-namespace release when it carries a higher version number, so a fork that has fallen behind upstream stays visible.
- If the same dependency is required with two different prefixes anywhere in the dependency tree, Bender never guesses. It reports the clash like any other conflicting requirement: on a terminal it asks you to pick one of the requirements, and without one (in CI, say) it fails. To settle it permanently, add an [`overrides`](./configuration.md) entry pinning the dependency to a single namespace — note that overrides live in `.bender.yml`, not in `Bender.yml`:

```yaml
# .bender.yml
overrides:
  common_cells: { git: "...", version: "1.21.0", version_prefix: "companyX-v" }
```

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

When submodules are disabled, Bender emits a warning for each dependency that carries submodules, listing the unchecked-out submodule paths and the `git submodule update --init --recursive` command to fetch them back into its checkout.

## Version Resolution and the Lockfile

When you run `bender update`, Bender performs the following:
1.  **Resolution:** It scans the entire dependency tree and finds a set of versions that satisfy all constraints.
2.  **Locking:** The exact versions and Git commit hashes are written to [`Bender.lock`](./lockfile.md).

For details on updating dependencies, see [Adding and Updating Dependencies](./workflow/dependencies.md)

**Reproducibility:** Once a [`Bender.lock`](./lockfile.md) exists, running `bender checkout` will always download the exact same code, even if newer compatible versions have been released. Always commit your [`Bender.lock`](./lockfile.md) to version control.

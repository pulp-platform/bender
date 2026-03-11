# Dependencies

Bender is designed to manage complex, hierarchical dependency trees for hardware projects. A dependency is any external package that provides its own `Bender.yml` manifest.

## Dependency Types

Dependencies are defined in the `dependencies` section of your `Bender.yml`.

### Git Dependencies
Git is the primary way to distribute Bender packages. You can specify them in two ways:

> **Important:** The shorthand notation (e.g., `common_cells: "1.21.0"`) is only available if you have defined at least one [remote](#remotes) in your manifest. If no remote is specified, you must use the full `git` URL (see [revision](#revision-based))

#### Version-based (Recommended)
Bender uses [Semantic Versioning (SemVer)](https://semver.org/) to find the best compatible version. You can use standard [SemVer operators](https://docs.rs/semver/latest/semver/enum.Op.html) to specify version ranges:

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

## Targets

Dependencies can be conditionally included or configured using targets. For details on how to use target expressions or pass targets to dependencies, see the [Targets](./targets.md) documentation.

## Git LFS Support

Bender automatically detects if a dependency uses **Git Large File Storage (LFS)**. If `git-lfs` is installed on your system, Bender will automatically pull the required large files during the checkout process.

## Version Resolution and the Lockfile

When you run `bender update`, Bender performs the following:
1.  **Resolution:** It scans the entire dependency tree and finds a set of versions that satisfy all constraints.
2.  **Locking:** The exact versions and Git commit hashes are written to `Bender.lock`.

**Reproducibility:** Once a `Bender.lock` exists, running `bender checkout` will always download the exact same code, even if newer compatible versions have been released. Always commit your `Bender.lock` to version control.

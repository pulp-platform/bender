# Global and Local Configuration

Bender uses a flexible configuration system that combines files, environment variables, and command-line flags. This page documents the configuration fields and their resolution order; for the role of [`Bender.local`](./local.md) within this system and its use as a per-workspace override file, see [Local Configuration](./local.md).

## Configuration Methods and Precedence

When resolving a configuration setting, Bender follows this order of precedence (highest to lowest):

1.  **Command-Line Flags:** Explicitly passed arguments (e.g., `--git-throttle 8`).
2.  **Environment Variables:** System-level variables (e.g., `BENDER_GIT_THROTTLE=8`).
3.  **Configuration Files:** Settings loaded from YAML files. These files are merged, with lower-level files overwriting higher-level ones:
    -   [`Bender.local`](./local.md) (Local workspace, ignored by Git)
    -   `.bender.yml` (Project-specific, checked into Git)
    -   `$HOME/.config/bender.yml` (User-specific)
    -   `/etc/bender.yml` (System-wide)

### Path Substitution
On Unix-like systems, paths within configuration files can use environment variables (e.g., `$HOME` or `${VAR}`). These will be automatically substituted when the configuration is loaded.

---

## Configuration Fields

### `database`
The directory where Bender stores cloned and checked-out dependencies.
- **Config Key:** `database`
- **Default:** `.bender` in the project root.
- **Example:** `database: /var/cache/bender_dependencies`

### `db_dir`
Optional override for the directory that holds bare git repositories and their lock files (i.e. `<db_dir>/git/db/` and `<db_dir>/git/locks/`). When set, it takes precedence over `database` (whether explicitly configured or left at its default) for these two paths only; the working-tree checkouts continue to follow `database` (or [`workspace.checkout_dir`](./manifest.md) in the project manifest). This makes it possible to share the heavy git data across projects on a persistent runner without also relocating per-project checkouts. See [Continuous Integration › Sharing the Database](./workflow/ci.md#sharing-the-database-across-runs-and-projects) for the recommended setup.
- **Config Key:** `db_dir`
- **Env Var:** `BENDER_DB_DIR` (used only when no configuration file sets `db_dir`; configuration files always take precedence).
- **Default:** unset (falls back to `database`).
- **Example:** `db_dir: /var/cache/bender_shared`

> **Note:** Older Bender versions (pre-`db_dir`) silently ignore this field and fall back to their per-project default, so it is safe to ship in a shared configuration that mixed bender versions may read.

### `git`
The command or path used to invoke Git.
- **Config Key:** `git`
- **Default:** `git`
- **Example:** `git: /usr/local/bin/git-wrapper.sh`

### `git_throttle`
The maximum number of concurrent Git operations.
- **Config Key:** `git_throttle`
- **CLI Flag:** `--git-throttle`
- **Env Var:** `BENDER_GIT_THROTTLE`
- **Default:** `4`
- **Example:** `git_throttle: 8`

### `git_lfs`
Enable or disable Git Large File Storage (LFS) support. Requires `git-lfs` to be installed on the system.
- **Config Key:** `git_lfs`
- **Default:** `true`
- **Example:** `git_lfs: false`

### `overrides`
Forces specific dependencies to use a particular version or local path. This is primarily used in [`Bender.local`](./local.md) for development.
- **Config Key:** `overrides`
- **Example:**
  ```yaml
  overrides:
    common_cells: { path: "../local_development/common_cells" }
  ```

### `plugins`
Auxiliary plugin dependencies that are loaded for every package. These allow you to provide additional Bender subcommands across your entire environment. The entry uses the same format as the `dependencies` section of a manifest.
- **Config Key:** `plugins`

> **Deprecated:** Configuring `plugins` from the configuration file is deprecated and may be removed in a future release. Prefer declaring `plugins` in the package manifest ([`Bender.yml`](./manifest.md)).

---

## Global CLI Options

The following options can be set via command-line flags or environment variables, but do not have a corresponding key in the configuration files.

### `dir`
Sets a custom root working directory. This directory is used as the starting point to search for `Bender.yml` and configuration files. It also determines the default location of the `database`.
- **CLI Flag:** `-d`, `--dir`
- **Env Var:** `BENDER_DIR`
- **Default:** Current working directory.

### `local`
Disables fetching of remotes. Useful for working on air-gapped computers or when you want to ensure no network operations occur. When set, Bender emits warning `W14` to remind you that resolution may not pick up newly available versions.
- **CLI Flag:** `--local`
- **Env Var:** `BENDER_LOCAL`

### `verbose`
Increases logging verbosity.
- **CLI Flag:** `-v`, `-vv`, `-vvv` (info, debug, trace)
- **Env Var:** `BENDER_VERBOSE` (set to `1`, `2`, or `3`)

### `no_progress`
Disables interactive progress bars.
- **CLI Flag:** `--no-progress`
- **Env Var:** `BENDER_NO_PROGRESS`

### `suppress`
Suppresses specific warnings.
- **CLI Flag:** `--suppress <WARNING>` (can be used multiple times)
- **Env Var:** `BENDER_SUPPRESS_WARNINGS` (comma-separated list)

---

## Usage Example

A typical [`Bender.local`](./local.md) file used for local development might look like this:

```yaml
# Speed up git operations locally
git_throttle: 8

# Work on a local copy of common_cells
overrides:
  common_cells: { path: "../common_cells" }
```

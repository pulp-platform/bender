# Global and Local Configuration

Bender uses a cascading configuration system that allows you to set global defaults while still providing the flexibility to override settings for specific projects or local environments.

## Configuration Files

Bender looks for configuration files in the following order, merging them as they are encountered:

1.  **System-wide:** `/etc/bender.yml`
2.  **User-specific:** `$HOME/.config/bender.yml` (or the equivalent on your OS)
3.  **Project-specific (checked in):** `.bender.yml` in your project root.
4.  **Local Workspace (ignored):** `Bender.local` in your project root.

The contents of these files are merged such that a configuration in a lower-level file (like `Bender.local`) will overwrite a configuration in a higher-level file (like `/etc/bender.yml`).

## Configuration Fields

The configuration files use the YAML format and support the following fields:

### `database`
The directory where Bender stores cloned and checked-out dependencies.
- **Default:** `.bender` in the project root.
- **Example:** `database: /var/cache/bender_dependencies`

### `git`
The command or path used to invoke Git.
- **Default:** `git`
- **Example:** `git: /usr/local/bin/git-wrapper.sh`

### `git_throttle`
The maximum number of concurrent Git operations.
- **Default:** `4`
- **Example:** `git_throttle: 2`

### `git_lfs`
Enable or disable Git Large File Storage (LFS) support.
- **Default:** `true`
- **Example:** `git_lfs: false`

### `overrides`
Forces specific dependencies to use a particular version or local path. This is primarily used in `Bender.local` for development.
- **Example:**
  ```yaml
  overrides:
    common_cells: { path: "../local_development/common_cells" }
  ```

### `plugins`
Auxiliary plugin dependencies that are loaded for every package. These allow you to provide additional Bender subcommands across your entire environment.

## Usage Example

A typical `Bender.local` file used for local development might look like this:

```yaml
# Speed up git operations locally
git_throttle: 8

# Work on a local copy of common_cells
overrides:
  common_cells: { path: "../common_cells" }
```

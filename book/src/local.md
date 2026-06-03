# Local Configuration (`Bender.local`)

`Bender.local` is an optional, user-specific configuration file used to override the project's default settings. Its primary purpose is to allow local development of dependencies without modifying the shared [`Bender.yml`](./manifest.md) or [`Bender.lock`](./lockfile.md).

`Bender.local` is one entry in Bender's configuration file chain. See [Configuration](./configuration.md) for the full precedence order, the list of available fields, and the equivalent environment variables and CLI flags.

## Overriding Dependencies

The most common use for `Bender.local` is the `overrides` section. This forces Bender to use a specific version or a local path for a dependency, regardless of what the manifest requires.

Entries in `overrides` use the same syntax as entries in the `dependencies` section of [`Bender.yml`](./manifest.md) (see [Dependencies](./dependencies.md)), with the exception that target expressions are not supported.

```yaml
overrides:
  # Force a local path for 'common_cells'
  common_cells: { path: "../local_development/common_cells" }
  
  # Force a specific git URL/revision
  axi: { git: "https://github.com/my_fork/axi.git", rev: "experimental_branch" }
```

When an override is present, Bender will prioritize it over any other version resolution and emit warning `W18` listing the package and the override target, so you can spot accidental overrides in `Bender.local`.

> **Note:** `overrides` only replace dependencies that already exist somewhere in the resolved dependency tree. They cannot be used to introduce new dependencies that are not pulled in by any package's [`Bender.yml`](./manifest.md).

## Management

`Bender.local` can be managed both manually and automatically. Several Bender commands manage it for you during the development process:

- [`bender clone`](./workflow/package_dev.md#cloning-dependencies): Automates moving a dependency to a local working directory and adds a `path` override.
- [`bender snapshot`](./workflow/package_dev.md#snapshotting): Updates `Bender.local` with the current Git hashes of your local checkouts.

For a detailed guide on using these commands for multi-package development, see the [Package Development Workflow](./workflow/package_dev.md).

## Other Configurations

`Bender.local` can also be used to configure tool-specific settings:

```yaml
# Change the directory where dependencies are stored (default is .bender)
database: my_deps_cache

# Share only the bare git repos and lock files across projects, while
# keeping per-project checkouts under each project's own .bender/.
# Bender versions before 0.32 silently ignore this field.
db_dir: /var/cache/bender_shared

# Use a custom git binary or wrapper
git: /usr/local/bin/git-wrapper.sh
```

## Best Practices

- **Don't Commit It:** `Bender.local` should **rarely** be checked into version control. It can contain paths and settings specific to your local machine. Always add it to your `.gitignore`.
- **Use for Development:** Think of it as your "scratchpad" for multi-package development. Once your changes to a dependency are stable and released (tagged), remember to remove the override from `Bender.local` and update your [`Bender.yml`](./manifest.md) with the new version.

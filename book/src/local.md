# Local Configuration (`Bender.local`)

`Bender.local` is an optional, user-specific configuration file used to override the project's default settings. Its primary purpose is to allow local development of dependencies without modifying the shared `Bender.yml` or `Bender.lock`.

## Overriding Dependencies

The most common use for `Bender.local` is the `overrides` section. This forces Bender to use a specific version or a local path for a dependency, regardless of what the manifest requires.

```yaml
overrides:
  # Force a local path for 'common_cells'
  common_cells: { path: "../local_development/common_cells" }
  
  # Force a specific git URL/revision
  axi: { git: "https://github.com/my_fork/axi.git", rev: "experimental_branch" }
```

When an override is present, Bender will prioritize it over any other version resolution.

## Management

`Bender.local` can be managed both manually and automatically. Several Bender commands manage it for you during the development process:

- **[`bender clone`](./workflow/package_dev.md#cloning-dependencies):** Automates moving a dependency to a local working directory and adds a `path` override.
- **[`bender snapshot`](./workflow/package_dev.md#snapshotting):** Updates `Bender.local` with the current Git hashes of your local checkouts.

For a detailed guide on using these commands for multi-package development, see the [Package Development Workflow](./workflow/package_dev.md).

## Other Configurations

`Bender.local` can also be used to configure tool-specific settings:

```yaml
# Change the directory where dependencies are stored (default is .bender)
database: my_deps_cache

# Use a custom git binary or wrapper
git: /usr/local/bin/git-wrapper.sh
```

## Best Practices

- **Don't Commit It:** `Bender.local` should **rarely** be checked into version control. It contains paths and settings specific to your local machine. Always add it to your `.gitignore`.
- **Use for Development:** Think of it as your "scratchpad" for multi-package development. Once your changes to a dependency are stable and released (tagged), remember to remove the override from `Bender.local` and update your `Bender.yml` with the new version.

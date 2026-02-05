# Configuration Format (`bender.yml`, `Bender.local`)

Bender looks for a configuration file in the following places:

- `/etc/bender.yml`
- `$HOME/.config/bender.yml`

It will also look recursively upwards from the current working directory for the following:

- `.bender.yml`
- `Bender.local`

The contents of these files are merged as they are encountered, such that a configuration in `foo/.bender.yml` will overwrite a configuration in `foo/bar/.bender.yml`.

The configuration file generally looks as follows:

```yaml
# Location of the cloned and checked-out dependencies. Optional.
# Default: ".bender" in the current package's root directory.
database: some/directory

# The command to use to invoke `git`. Optional.
# Default: "git"
git: git-wrapper.sh


# Overrides for dependencies. Optional.
# Forces a dependencies to use specific versions or local paths. Useful for
# locally resolving dependency conflicts in a package's own Bender.local file.
# Format is the same as `dependencies` in a package manifest.
overrides:
  common_cells: { path: "/var/magic/common_cells" }
  apb_uart:     { git: "git@github.com:pulp-platform/apb_uart.git"}

# Auxiliary plugin dependencies. Optional.
# Additional dependencies that will be loaded for every package in order to
# provide the `plugins` listed in their manifests.
# Format is the same as `dependencies` in a package manifest.
# DEPRECATED: This will be removed at some point.
plugins:
  additional-tools: { path: "/usr/local/additional-tools" }

# Number of parallel git tasks. Optional.
# Default: 4
# The number of parallel git operations executed by bender can be adjusted to
# manage performance and load on git servers. Can be overriden as a command
# line argument.
git_throttle: 2

# Enable git lfs. Optional.
# Default: true
# Some git dependencies may use git-lfs for additional source files. As
# fetching these files may not always be desired or requried, it can be
# disabled. For multiple conflicting settings will use true.
git_lfs: false
```

[Relevant code](https://github.com/pulp-platform/bender/blob/master/src/config.rs)

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## Unreleased
### Added
- Add `vivado-sim` format to the script command.  This format differs from the `vivado` format in that it targets simulation, not synthesis; other than that, it is equivalent to the `vivado` format.

## 0.16.1 - 2020-03-10
### Changed
- `script vivado` now emits paths relative to the project root with the `ROOT` variable.

### Fixed
- `script vcs`: Fix shell syntax to define `ROOT` variable.

## 0.16.0 - 2020-02-17
### Added
- Add `vcs` format to the `script` command.

### Changed
- `script`: `vcom-arg` or `vlog-arg` can be used with format `vsim` or `vcs`.

## 0.15.0 - 2020-02-13
### Added
- `script vivado`: Add options to restrict output to defines, includes, or sources.

### Changed
- `script`: Raise error if `vcom-arg` or `vlog-arg` are used with format other than `vsim`.
- `script vivado`:
  - Add `-norecurse` to `add_files` to prevent warnings.
  - Set `include_dirs` and `verilog_define` properties also for `simset` to prevent critical warnings.  This can be disabled with the new `--no-simset` option.

### Fixed
- `script vivado`: Fix name of `include_dirs` property.

## 0.14.0 - 2019-10-16
### Added
- Add `vivado` format to the `script` command.

## 0.13.3 - 2019-07-29
### Changed
- Bump rustc minimum version to 1.36

### Fixed
- Fix rustc 1.36 `as_ref` [regression](https://github.com/rust-lang/rust/issues/60958)
- Emit format-specific target defines even if no target is specified.

## 0.13.2 - 2019-07-29
### Fixed
- Fix capitalization of target defines such as `TARGET_SYNTHESIS`.

## 0.13.1 - 2019-07-18
### Fixed
- Omit unsupported `-2008` flag for VHDL analysis commands in Synopsys scripts.

## 0.13.0 - 2019-07-17
### Added
- Add the `frozen` option to prevent any dependency updates for a package.
- Add the `workspace` section to carry workspace configuration.
- Add the `workspace.checkout_dir` option to keep local working copies of each dependency.

### Changed
- Make order of packages and defines deterministic.
- Move the `package_links` option to `workspace.package_links`.

## 0.12.1 - 2019-05-08
### Added
- Add `-f` flag to `sources` for printing a flattend source listing.

### Changed
- Make JSON output human readable.

### Fixed
- Fix target defines to be all uppercase.
- Fix emission of target defines for source files which have no other defines.

## 0.12.0 - 2019-04-17
### Added
- Add `script` subcommand.
- Add `vsim` and `synopsys` script output formats.

## 0.11.0 - 2019-03-26
### Added
- Add `atty` dependency.
- Add `itertools` dependency.

### Changed
- Ask the user to resolve dependency conflicts if stdin/stdout are attached to an interactive console.
- Update `semver` dependency to v0.9.

## 0.10.0 - 2019-01-08
### Changed
- Create relative package links if possible.

## 0.9.0 - 2019-01-08
### Added
- Add the `udpate` subcommand that explicitly updates dependencies.
- Add dependency on the `dirs` crate.

### Changed
- No longer automatically update dependencies when the manifest changes.

## 0.8.0 - 2018-12-07
### Added
- Add `export_include_dirs` section to manifest, allowing packages to expose a subdirectory as include file search path.

### Changed
- Rename `package-links` manifest section to `package_links`.

## 0.7.1 - 2018-12-07
### Fixed
- Fix an issue where path dependencies would not properly resolve transitively.

## 0.7.0 - 2018-09-20
### Added
- Submodules are now checked out as well.
- Locally symlinked packages via the `package-links` list in the manifest.

### Changed
- By default, the `.bender` working directory is now a subdirectory of the root working directory.

## 0.6.2 - 2018-03-23
### Fixed
- Show checkout message only when actually checking out.
- Don't show the `--debug` option in release builds.

## 0.6.1 - 2018-03-22
### Added
- Show message when checking out a repository.
- Show message when fetching from a git remote.
- `--debug` option for debug builds.

### Changed
- Improved error messages when constraints cannot be met.

### Fixed
- Limit the number of concurrent git network activity to 8.

## 0.6.0 - 2018-03-01
### Added
- Plugin packages can now be declared in the `plugins` section of the config.
- Add the `package` field to source groups.
- Add the `bender config` command to dump the tool configuration.

## 0.5.0 - 2018-02-22
### Added
- Add plugin support. The manifest may now have a `plugins` section.
- Add include directories and preprocessor defines support. These can now be specified for source groups.
- Add `overrides` section to config.
- Add target specifications for source files.

### Changed
- Output of `bender sources` is now JSON formatted.
- Items in lock files have deterministic order.

## 0.4.0 - 2018-02-16
### Added
- Add `sources` section to manifest.
- Add `bender sources` command to access the source file manifest of the root package and its dependencies.

### Changed
- Dependency resolution only runs if lock file is older than manifest.

### Fixed
- Manifests of path dependencies are now read properly.
- Relative paths in config files are now treated as relative to the config file, not the current working directory.

## 0.3.2 - 2018-02-14
### Changed
- Only update git databases when manifest file changes.
- Only load manifests once per session.
- Only update checkouts once per session.

## 0.3.1 - 2018-02-14
### Fixed
- Fix crash when dependencies have no manifest.

## 0.3.0 - 2018-02-13
### Added
- Initial implementation of recursive dependency resolution.
- The `bender packages` command to access the package dependency graph.

### Changed
- Lockfile now contains the dependency names for each package.

## 0.2.1 - 2018-02-06
### Fixed
- Help page now shows proper program name, version, and authors.

## 0.2.0 - 2018-02-06
### Changed
- Use `git archive | tar xf -` to create checkouts.

### Fixed
- Fix creating a checkout of a branch.
- Fail gracefully if a dependency repository is empty.

## 0.1.0 - 2018-02-01
### Added
- Initial support for checkouts.
- Resolution of the root package's dependencies.
- The `bender path` command to get a path to the checked out dependencies.

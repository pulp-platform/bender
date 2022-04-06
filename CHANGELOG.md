# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## Unreleased
### Added
- `parents` command outputs more detail:
  - source information in case multiple dependencies do not match
  - information about an override if one is present

### Changed
- Interpret all dependency names as lowercase to reduce ambiguity

### Fixed
- Fix panic of `parents` command if a dependency does not have a manifest or if the manifest does not match the `Bender.lock` file.
- Use correct manifest of checked out dependency in case folder already exists in `checkout_dir`.
- Fix vcs script argument to use `vhdlan-bin` for vhdlan binary
- Fix incomplete dependency version update when refetching from remote
- Fix panic when using `checkout_dir` if the directory does not yet exist

## 0.24.0 - 2022-01-06
### Added
- Add error if a cyclical dependency is detected to avoid infinite loop
- Add error on dependency mismatch between `Bender.yml` and `Bender.lock`
- Add hint to work around the "too many open files" error (issue #52).
- Add warning for mismatch of dependency name and name in package
- Add dependency information to `sources` command
- Add `--fetch/-f` argument to `bender update` to force re-fetch of git dependencies from their remotes
- Add global `--local` argument to disable remote accesses of git commands, e.g. for air-gapped computers
- Add `precision` format to the script command.
- Extend the `sources` and `scripts` commands with three flags to work with a subset of packages (`-p`/`--package`, `-e`/`--exclude`, and `-n`/`--no-deps`; see ReadMe or command help for the description).

### Changed
- Reduce the number of open files in large repositories by changing the method to get the Git commit hash from a tag (from individual calls to `git rev-parse --verify HASH^{commit}` to `git show-ref --dereference`).
- Change behavior of `export_include_dirs` in `Bender.yml` file to not include these directories globally, but only include the directories exported within the package and direct dependencies (this may break unclean implementations).

### Fixed
- Fix absolute path of path dependencies inside git dependencies.

## 0.23.2 - 2021-11-30
### Changed
- Wrap defines in quotes for the VCS's shell script

## 0.23.1 - 2021-09-29
### Fixed
- CI -> changed from travis to github actions

## 0.23.0 - 2021-09-13
### Changed
- Change `parents` command, now displays version requirement of each package
- updated resolver to use a single Core and SessionIo object for performance
- Add HashMap to git_versions in SessionIo to avoid multiple identical git calls, significantly improves performance

## 0.22.0 - 2021-01-21
### Added
- Add `clone` command to checkout individual ips to a directory and create a path reference in Bender.local
- Add `parents` command to get list of packages requiring the queried package

### Fixed
- Fix plugins to work for scripts in root repository
- Force git fetch on unsatisfied requirements to check for newly added tags in server repository

## 0.21.0 - 2020-11-04
### Added
- Add option to pass additional defines through the command line ([#34](https://github.com/fabianschuiki/bender/pull/34))

### Changed
- Switch to Rust 2018 edition
- Update dependency `serde_yaml`, `tokio-timer`, `semver`, `blake2`, `typed-arena`, `dirs`, `pathdiff`, and `itertools`
- Extended documentation in the README

## 0.20.0 - 2020-07-04
### Added
- Add `riviera` format to the script command. ([#29](https://github.com/fabianschuiki/bender/pull/29))

## 0.19.0 - 2020-04-27
### Added
- Add `flist` scripts target emitting a plain file list.

### Changed
- `script` now inserts Tcl `catch` statements for `synopsys` and `vsim` to abort elaboration on the first error.  This can be disabled with the new `--no-abort-on-error` flag.

## 0.18.0 - 2020-04-03
### Added
- Add `verilator` and `genus` formats to the script command. ([#27](https://github.com/fabianschuiki/bender/pull/27))

## 0.17.0 - 2020-03-11
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

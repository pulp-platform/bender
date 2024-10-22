# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## Unreleased
### Fixed
- Put `vcs`, `vsim`, and `riviera` defines in quotes.
- Fix `genus` script initialization.
- Update Readme with for script formats.
- Fix vendor file mappings when combining into a single directory.
- Make panic an error when lockfile is not up to date with dependencies.
- Fix Readme dependency version indication for exact match.
- Fix vendor file copying for symbolic links.

### Added
- Add `completion` command to generate shell autocomplete integration.
- Add abort on error for `vcs` script type.
- Add warning to update command when using overrides.
- Add support for branchless commits in dependency repositories.

### Changed
- Bump dependencies.

### Added
- Add flag for `rtl` target to files without target in script and sources.

## 0.28.1 - 2024-02-22
### Added
- Add `flist-plus` script format for file list with plusargs.

### Fixed
- Ensure defines/includes/sources are included in script when specifying multiple `--only-*`.

### Changed
- For `bender clone`: Add a relative path to the lockfile to align to change in v0.27.0.

## 0.28.0 - 2024-01-19
### Added
- Add macOS binary for releases
- Add `init` command to initialize a Bender.yml file of an IP.
- Allow environment variables in dependency and sources paths.
- Add windows binary and compatibility for release.

### Fixed
- Documentation and Error Message fixes.

### Changed
- Complete revamp of script generation, now using templates. Script formats are homogenized and custom templates are enabled.

## 0.27.4 - 2023-11-14
### Added
- Add clearer error message when commits are no longer available upstream.
- Improve Readme git explanation

### Fixed
- Fix CI GNU release.

## 0.27.3 - 2023-09-12
### Added
- Add `--checkout` flag to `path` command to force checkout if needed.
- Add `--no-checkout` flag to `update` command to prevent checkout after update if not needed.

### Changed
- `path` and local links: Skip checkout if package path already exists (can be overruled by `--checkout` flag)
- `update`: Default to automatically perform checkout after update (can be overruled by `--no-checkout` flag)

### Fixed
- Improve ReadMe and Warning information for `vendor` upstream linking.
- Ensure `workspace.package_links` symlinks are properly updated when executing the `clone` command.

## 0.27.2 - 2023-07-12
### Added
- Add information on expected location for manifest file not found.

### Fixed
- Use `IndexMap` and `IndexSet` instead of the `std Hash*` equivalents to preserve ordering
- Change GNU release to be built to a more compatible binary (manylinux container).
- Parse override dependencies in lowercase to align to change in 0.25.0

### Changed
- Adjusted hash input for dependency checkout to ensure consistency within a project.

## 0.27.1 - 2023-01-25
### Fixed
- Fixed accidental debug print in `sources` command

## 0.27.0 - 2023-01-16
### Added
- Add `--no-default-target` flag to `script` command to remove default targets
- Add `fusesoc` command to generate FuseSoC `.core` files.
- Add rhel and almalinux releases

### Changed
- Reworked `import` command to `vendor`, refactor corresponding Manifest entry (`vendor_package` instead of `external_import`)
- Update `clap` to v4, changes CLI
- Use relative paths in Lockfile if path dependency is in a subdirectory

### Fixed
- Streamline `import` command for initializing a repository

## 0.26.1 - 2022-08-29
### Fixed
- Keep `export_incdirs` from excluded dependencies in sources and scripts

## 0.26.0 - 2022-08-26
### Added
- Add clippy to github CI and fix noted issues
- Add `import` command to import files from non-bender repositories

### Changed
- Update `tokio` dependency, update to `futures=0.3`, rewrite for `async/await`. Bumps minimum rust to 1.57
- Update `serde_yaml` dependency to `0.9`. Bumps minimum rust to 1.58
- Partially checkout path dependencies in git dependencies
- Add warnings if manifest file is not found for a dependency

## 0.25.3 - 2022-08-05
### Added
- Added `formality` script.

### Changed
- Update dependencies, with slight restructuring updating `clap` to v3.1, bumps minimum rust to 1.54
- Removed `defines` from VHDL in `synopsys` script.
- Updated CI release OSs to currently supported releases, added debian

## 0.25.2 - 2022-04-13
### Added
- Add `checkout` command to fetch and checkout dependencies

### Fixed
- Ensure consistency when manually chosing a version if there are conflicts.
- Update `Bender.lock` when running `bender clone`. Running `bender update` is no longer required afterwards.

## 0.25.1 - 2022-04-08
### Fixed
- For multiple compatible versions, select the highest compatible version, not the newest commit with a compatible version.

## 0.25.0 - 2022-04-06
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

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## Unreleased

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

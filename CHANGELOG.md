# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- Add plugin support. The manifest may now have a `plugins` section.
- Add include directories and preprocessor defines support. These can now be specified for source groups.

### Changed
- Output of `bender sources` is now JSON formatted.

## [0.4.0] - 2018-02-16
### Added
- Add `sources` section to manifest.
- Add `bender sources` command to access the source file manifest of the root package and its dependencies.

### Changed
- Dependency resolution only runs if lock file is older than manifest.

### Fixed
- Manifests of path dependencies are now read properly.
- Relative paths in config files are now treated as relative to the config file, not the current working directory.

## [0.3.2] - 2018-02-14
### Changed
- Only update git databases when manifest file changes.
- Only load manifests once per session.
- Only update checkouts once per session.

## [0.3.1] - 2018-02-14
### Fixed
- Fix crash when dependencies have no manifest.

## [0.3.0] - 2018-02-13
### Added
- Initial implementation of recursive dependency resolution.
- The `bender packages` command to access the package dependency graph.

### Changed
- Lockfile now contains the dependency names for each package.

## [0.2.1] - 2018-02-06
### Fixed
- Help page now shows proper program name, version, and authors.

## [0.2.0] - 2018-02-06
### Changed
- Use `git archive | tar xf -` to create checkouts.

### Fixed
- Fix creating a checkout of a branch.
- Fail gracefully if a dependency repository is empty.

## [0.1.0] - 2018-02-01
### Added
- Initial support for checkouts.
- Resolution of the root package's dependencies.
- The `bender path` command to get a path to the checked out dependencies.

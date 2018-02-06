# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Changed
- Use `git init --separate-git-dir` to create checkouts.

### Fixed
- Fix creating a checkout of a branch.
- Fail gracefully if a dependency repository is empty.

## [0.1.0] - 2018-02-01
### Added
- Initial support for checkouts.
- Resolution of the root package's dependencies.
- The `bender path` command to get a path to the checked out dependencies.

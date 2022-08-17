# bender

Bender is a dependency management tool for hardware design projects. It provides a way to define dependencies among IPs, execute unit tests, and verify that the source files are valid input for various simulation and synthesis tools.

![Build Status](https://github.com/pulp-platform/bender/actions/workflows/ci.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/bender.svg)](https://crates.io/crates/bender)
[![dependency status](https://deps.rs/repo/github/pulp-platform/bender/status.svg)](https://deps.rs/repo/github/pulp-platform/bender)
![Crates.io](https://img.shields.io/crates/l/bender)


## Table of Contents

- [Principles](#principles)
- [Installation](#installation)
- [Workflow](#workflow)
- [Package Structure](#package-structure)
- [Manifest Format (`Bender.yml`)](#manifest-format-benderyml)
  - [Dependencies](#dependencies)
  - [Sources](#sources)
  - [Targets](#targets)
- [Configuration Format (`bender.yml`, `Bender.local`)](#configuration-format-benderyml-benderlocal)
- [Commands](#commands)


## Principles

Bender is built around the following core principles:

- **Be as opt-in as possible.** We do not assume any specific EDA tool, workflow, or directory layout (besides a few key files). All features are designed to be as modular as possible, such that the user can integrate them into their respective flow.

- **Allow for reproducible builds.** Bender maintains a precise *lock file* which tracks the exact git hash a dependency has been resolved to. This allows the source code of a package to be reliable reconstructed after the fact.

- **Collect source files.** The first feature tier of Bender is to collect the source files in a hardware IP. In doing this, it shall do the following:
  - Maintain the required order across source files, e.g. for package declarations before their use.
  - Be as language-agnostic as possible, supporting both SystemVerilog and VHDL.
  - Allow source files to be organized into recursive groups.
  - Track defines and include directories individually for each group.

- **Manage dependencies.** The second feature tier of Bender is to maintain other packages an IP may depend on, and to provide a local checkout of the necessary source files. Specifically, it shall:
  - Support transitive dependencies
  - Not rely on a central package registry, unlike e.g. npm, cargo, or brew (necessary because parts of a project are usually under NDA)
  - Enforce strict use of [semantic versioning](https://semver.org/)

- **Generate tool scripts.** The third feature tier of Bender is the ability to generate source file listings and compilation scripts for various tools.


## Installation

To use Bender for a single project, the simplest is to download and use a precompiled binary.  We provide binaries for all current versions of Ubuntu and CentOS, as well as generic Linux, on each release.  Open a terminal and enter the following command:
```sh
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh
```
The command downloads and executes a script that detects your distribution and downloads the appropriate `bender` binary of the latest release to your current directory.  If you need a specific version of Bender (e.g., `0.21.0`), append ` -s -- 0.21.0` to that command.  Alternatively, you can manually download a precompiled binary from [our Releases on GitHub][releases].

If you prefer building your own binary, you need to [install Rust][rust-installation].  You can then build and install Bender for the current user with the following command:
```sh
cargo install bender
```
If you need a specific version of Bender (e.g., `0.21.0`), append ` --version 0.21.0` to that command.

To install Bender system-wide, you can simply copy the binary you have obtained from one of the above methods to one of the system directories on your `PATH`.  Even better, some Linux distributions have Bender in their repositories.  We are currently aware of:

### [ArchLinux ![aur-shield](https://img.shields.io/aur/version/bender)][aur-bender]

Please extend this list through a PR if you know additional distributions.


## Workflow

The workflow of bender is based on a configuration and a lock file. The configuration file lists the sources, dependencies, and tests of the package at hand. The lock file is used by the tool to track which exact version of a package is being used. Adding this file to version control, e.g. for chips that will be taped out, makes it easy to reconstruct the exact IPs that were used during a simulation, synthesis, or tapeout.

Upon executing any command, bender checks to see if dependencies have been added to the configuration file that are not in the lock file. It then tries to find a revision for each added dependency that is compatible with the other dependencies and add that to the lock file. In a second step, bender tries to ensure that the checked out revisions match the ones in the lock file. If not possible, appropriate errors are generated.

The update command reevaluates all dependencies in the configuration file and tries to find for each a revision that satisfies all recursive constraints. If semantic versioning is used, this will update the dependencies to newer versions within the bounds of the version requirement provided in the configuration file.


## Package Structure

Bender looks for the following three files in a package:

- `Bender.yml`: This is the main **package manifest**, and the only required file for a directory to be recognized as a Bender package. It contains metadata, dependencies, and source file lists.

- `Bender.lock`: The **lock file** is generated once all dependencies have been successfully resolved. It contains the exact revision of each dependency. This file *may* be put under version control to allow for reproducible builds. This is handy for example upon taping out a design. If the lock file is missing or a new dependency has been added, it is regenerated.

- `Bender.local`: This optional file contains **local configuration overrides**. It should be ignored in version control, i.e. added to `.gitignore`. This file can be used to override dependencies with local variants. It is also used when the user asks for a local working copy of a dependency.

[Relevant code](https://github.com/pulp-platform/bender/blob/master/src/cli.rs)


## Manifest Format (`Bender.yml`)

The package manifest describes the package, its metadata, its dependencies, and its source files. All paths in the manifest may be relative, in which case they are understood to be relative to the directory that contains the manifest.

```yaml
# Package metadata. Required.
package:
  # The name of the package. Required.
  name: magic-chip

  # The list of package authors and contributors. Optional.
  # By convention, authors should be listed in the form shown below.
  authors: ["John Doe <john@doe.si>"]

# Other packages this package depends on. Optional.
dependencies:
  # Path dependency.
  axi: { path: "../axi" }

  # Registry dependency. Not supported at the moment.
  # common_verification: "0.2"

  # Git version dependency.
  common_verification: { git: "git@github.com:pulp-platform/common_verification.git", version: "0.1" }

  # Git revision dependency.
  common_cells: { git: "git@github.com:pulp-platform/common_cells.git", rev: master }

# Freeze any dependency updates. Optional. False if omitted.
# Useful for chip packages. Once the chip is in final tapeout mode, and
# dependency updates would require disastrous amounts of re-verification.
frozen: true

# List of source files in this package. Optional.
sources:
  # Individual source files are simple string entries:
  - src/package.sv
  - src/file1.vhd
  - src/file2.vhd

  # Source files can be grouped:
  - files:
      - src/stuff/pkg.sv
      - src/stuff/top.sv

  # Grouped source files may have additional include dirs, defines, and target:
  - include_dirs:
      - src/include
      - src/stuff/include
    defines:
      # Define without a value.
      EXCLUDE_MAGIC: ~
      # Define with a value.
      PREFIX_NAME: stuff
    target: all(asic, synthesis, freepdk45)
    files:
      - src/core/pkg.sv
      - src/core/alu.sv
      - src/core/top.sv

# A list of include directories which should implicitly be added to source
# file groups of packages that have the current package as a dependency.
# Optional.
export_include_dirs:
  - include
  - uvm/magic/include

# Additional workspace configuration. Optional.
workspace:
  # Create symlinks to dependencies.
  # A list of paths at which bender will create a symlink to the checked-out
  # version of the corresponding package.
  package_links:
    links/axi: axi
    common: common_cells

  # A directory where the dependencies will be checked out. Optional.
  # If specified, bender will check out the dependencies once and leave them
  # for the user to modify and keep up to date.
  # CAUTION: Bender will not touch these after the initial checkout.
  # Useful for chip packages, if the intent is to commit all dependencies into
  # the chip's version control.
  checkout_dir: deps

# Map of package-provided commands that can be called as `bender <cmd>`.
# Optional. Only available in dependent packages.
plugins:
  hello: scripts/hello.sh

# List of imported files from external repositories not supporting bender. Optional
vendor:
    # package name
  - name: lowrisc_opentitan
    # target directory
    target_dir: vendor/lowrisc_opentitan
    # upstream dependency (i.e. git repository similar to dependencies)
    upstream: { git: "https://github.com/lowRISC/opentitan.git", rev: "47a0f4798febd9e53dd131ef8c8c2b0255d8c139" }
    # paths to exclude from upstream dependency
    exclude_from_upstream:
      - "ci/*"
    # directory containing patch files
    patch_dir: "vendor/patches"
    # file mapping from remote repository to local repository, with optional patch_dir containing patches
    mapping: 
      - {from: 'hw/ip/prim/rtl/prim_subreg.sv', to: 'src/prim_subreg.sv' }
      - {from: 'hw/ip/prim/rtl/prim_subreg_arb.sv', to: 'src/prim_subreg_arb.sv' }
      - {from: 'hw/ip/prim/rtl/prim_subreg_ext.sv', to: 'src/prim_subreg_ext.sv', patch_dir: 'lowrisc_opentitan' }
      - {from: 'hw/ip/prim/rtl/prim_subreg_shadow.sv', to: 'src/prim_subreg_shadow.sv' }
```

[Relevant code](https://github.com/pulp-platform/bender/blob/master/src/config.rs)


### Dependencies

Dependencies are specified in the `dependencies` section of the package manifest, or the `overrides` section in the configuration file. There are different kinds of dependencies, as described in the following.

#### Path

    mydep: { path: "../path/to/mydep" }

Path dependencies are not considered versioned. Either all versions of dependency `mydep` point to the same path, or otherwise the resolution will fail.

#### Git

    mydep: { git: "git@github.com:pulp-platform/common_verification.git", rev: "<commit-ish>" }
    mydep: { git: "git@github.com:pulp-platform/common_verification.git", version: "1.1" }

Git dependencies are automatically checked out and cloned, and are considered for version resolution. The `rev` field can be a git "commit-ish", which essentially is a commit hash, a tag name, or a branch name, where the newest name that starts with the indicated revision is selected. The `version` field can be any of the [semver predicates](https://docs.rs/semver/#requirements), such as a simple version `X.Y.Z` (or `X.Y`), prefixing `^` to only allow that specific version, `~` to limit updates to patches, or defining custom ranges with `>=U.V.W, <X.Y.Z`. More detail on how the `version` field is parsed can be found in the [cargo documentation](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html). The highest compatible version is selected.

All git tags of the form `vX.Y.Z` are considered a version of the package.

[Relevant dependency resolution code](https://github.com/pulp-platform/bender/blob/master/src/resolver.rs)


### Sources

The source files listed in the `sources` section of the package manifest are a recursive structure. Each entry in the list can either be a single source file, or a group of source files:

```yaml
# Format of the `sources` section in the manifest:
sources:
  - <file or group 1>
  - <file or group 2>
  - ...

  # A source file is formatted as follows:
  - src/top.sv

  # A source group is formatted as follows.
  # Be careful about the `-`, which may appear on the same line as the first
  # field of the source group.
  -
    # List of include directories. Optional.
    include_dirs:
      - <include dir 1>
      - <include dir 2>
      - ...
    # List of defines. Optional.
    defines:
      # Defines without value:
      <define name 1>: ~
      <define name 2>: ~
      # Defines with value:
      <define name 3>: <define value 3>
      <define name 4>: <define value 4>
      ...
    # Target specifier. Optional.
    target: <target specifier>
    # Recursive list of source files and groups:
    files:
      - <file or group 1>
      - <file or group 2>
      - ...
```

The `target` specification configures a source group to be included or excluded under certain circumstances. See below for details. The `include_dirs` field specifies the `+incdir+...` statements to be added to any compilation command for the group. The `defines` field specifies the `+define+...` statements to be added add to any compilation command for this group.


### Targets

Targets are flags that can be used to filter source files and dependencies. They are used to differentiate the steps in the ASIC/FPGA design flow, the EDA tools, technology targets, and more. They can also be used to have different versions of an IP optimized for different chips or technologies.

Targets specify a simple expression language, as follows:

- `*` matches any target
- `name` matches the target "name"
- `all(T1, ..., TN)` matches if all of the targets T1 to TN match (boolean *AND*)
- `any(T1, ..., TN)` matches if any of the targets T1 to TN match (boolean *OR*)
- `not(T)` matches if target T does *not* match (boolean *NOT*)

The following targets are automatically set by various bender subcommands:

- `synthesis` for synthesis tool script generation
- `simulation` for simulation tool script generation

Individual commands may also set tool-specific targets:

- `vsim`
- `vcs`
- `verilator`
- `synopsys`
- `riviera`
- `genus`
- `vivado`

Individual commands may also set vendor-specific targets:

- `xilinx`
- `synopsys`

Individual commands may also set technology-specific targets:

- `asic`
- `fpga`

Additionally, we suggest to use the following targets to identify source code and netlists at different stages in the design process:

- `test` for testbench code
- `rtl` for synthesizable RTL code
- `gate` for gate-level netlists

[Relevant code](https://github.com/pulp-platform/bender/blob/master/src/target.rs)

### Vendor

Section to list files and directories copied and patched within this repository from external repositories not supporting bender.
To update, see below `vendor` command.


## Configuration Format (`bender.yml`, `Bender.local`)

Bender looks for a configuration file in the following places:

- `/etc/bender.yml`
- `$HOME/.config/bender.yml`

It will also look recursively upwards from the current working directory for the following:

- `.bender.yml`
- `Bender.local`

The contents of these files are merged as they are encountered, such that a configuration in `foo/bar/.bender.yml` will overwrite a configuration in `foo/.bender.yml`.

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

# Auxiliary plugin dependencies. Optional.
# Additional dependencies that will be loaded for every package in order to
# provide the `plugins` listed in their manifests.
# Format is the same as `dependencies` in a package manifest.
# DEPRECATED: This will be removed at some point.
plugins:
  additional-tools: { path: "/usr/local/additional-tools" }
```

[Relevant code](https://github.com/pulp-platform/bender/blob/master/src/config.rs)


## Commands

`bender` is the entry point to the dependency management system. Bender always operates within a package; starting at the current working directory, search upwards the file hierarchy until a `Bender.yml` is found, which marks the package.


### `path` --- Get the path of a checked-out package

The `bender path <PKG>` prints the path of the checked-out version of package `PKG`.

Useful in scripts:

    #!/bin/bash
    cat `bender path mydep`/src/hello.txt


### `packages` --- Display the dependency graph

- `bender packages`: List the package dependencies. The list is sorted and grouped according to a topological sorting of the dependencies. That is, leaf dependencies are compiled first, then dependent ones.
- `bender packages -f`: Produces the same list, but flattened.
- `bender packages -g`: Produces a graph description of the dependencies of the form `<pkg>TAB<dependencies...>`.


### `sources` --- List source files
[Code](https://github.com/pulp-platform/bender/blob/master/src/cmd/sources.rs)

Produces a *sources manifest*, a JSON description of all files needed to build the project.

The manifest is recursive by default; meaning that dependencies and groups are nested. Use the `-f`/`--flatten` switch to produce a simple flat listing.

To enable specific targets, use the `-t`/`--target` option.

To get the sources for a subset of packages, exclude specific packages and their dependencies, or exclude all dependencies, the following flags exist:

- `-p`/`--package`: Specify package to show sources for.
- `-e`/`--exclude`: Specify package to exclude from sources.
- `-n`/`--no-deps`: Exclude all dependencies, i.e. only top level or specified package(s).

For multiple packages (or excludes), multiple `-p` (or `-e`) arguments can be added to the command.


### `config` --- Emit the current configuration

The `bender config` command prints the currently active configuration as JSON to standard output.


### `script` --- Generate tool-specific scripts

The `bender script <format>` command can generate scripts to feed the source code of a package and its dependencies into a vendor tool.

Supported formats:

- `flist`: A flat file list amenable to be directly inlined into the invocation command of a tool, e.g. `verilate $(bender script flist)`.
- `vsim`: A Tcl compilation script for Mentor ModelSim/QuestaSim.
- `vcs`:  A Tcl compilation script for VCS.
- `verilator`: Command line arguments for Verilator.
- `synopsys`: A Tcl compilation script for Synopsys DC and DE.
- `riviera`: A Tcl compilation script for Aldec Riviera-PRO.
- `genus`:  A Tcl compilation script for Cadence Genus.
- `vivado`: A Tcl file addition script for Xilinx Vivado.
- `vivado-sim`: Same as `vivado`, but specifically for simulation targets.

Furthermore, similar flags to the `sources` command exist.


### `update` --- Re-resolve dependencies

Whenever you update the list of dependencies, you likely have to run `bender update` to re-resolve the dependency versions, and recreate the `Bender.lock` file.

Calling update with the `--fetch/-f` flag will force all git dependencies to be re-fetched from their corresponding urls.

> Note: Actually this should be done automatically if you add a new dependency. But due to the lack of coding time, this has to be done manually as of now.


### `clone` --- Clone dependency to make modifications

The `bender clone <PKG>` command checks out the package `PKG` into a directory (default `working_dir`, can be overridden with `-p / --path <DIR>`). 
To ensure the package is correctly linked in bender, the `Bender.local` file is modified to include a `path` dependency override, linking to the corresponding package.

This can be used for development of dependent packages within the parent repository, allowing to test uncommitted and committed changes, without the worry that bender would update the dependency.

To clean up once the changes are added, ensure the correct version is referenced by the calling packages and remove the path dependency in Bender.local.

> Note: The location of the override may be updated in the future to prevent modifying the human-editable `Bender.local` file by adding a persistent section to `Bender.lock`.

### `parents` --- Lists packages calling the specified package

The `bender parents <PKG>` command lists all packages calling the `PKG` package.

### `checkout` --- Checkout all dependencies referenced in the Lock file

This command will ensure all dependencies are downloaded from remote repositories. This is usually automatically executed by other commands, such as `sources` and `script`.

### `vendor` --- Copy files from dependencies that do not support bender

This command will update the dependencies listed in the `vendor` section of the `Bender.yml` file, fetching the files from the remote repositories and applying the necessary patch files.
This command will only refetch and patch the files in the top-level Bender.yml file.
Please make sure you manage the includes and sources required for these files separately, as this command only fetches the files and patches them.
This is in part based on [lowRISC's `vendor.py` script](https://github.com/lowRISC/opentitan/blob/master/util/vendor.py).

[aur-bender]: https://aur.archlinux.org/packages/bender
[releases]: https://github.com/pulp-platform/bender/releases
[rust-installation]: https://doc.rust-lang.org/book/ch01-01-installation.html

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

As an alternative binary installer path, we are migrating releases to [`cargo-dist`](https://github.com/axodotdev/cargo-dist). If you already use [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall), you can install via:
```sh
cargo binstall bender
```
For now, both approaches are supported.

If you prefer building your own binary, you need to [install Rust][rust-installation].  You can then build and install Bender for the current user with the following command:
```sh
cargo install bender
```
If you need a specific version of Bender (e.g., `0.21.0`), append ` --version 0.21.0` to that command.

To enable optional features (including the Slang-backed `pickle` command), install with:
```sh
cargo install bender --all-features
```
This may increase build time and additional build dependencies.

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

  # A short description of the package. Optional.
  description: "This is a magical chip"

# Specify git remotes for dependencies. Optional.
remotes:
  pulp:
    url: "https://github.com/pulp-platform"
    default: true # Only required if multiple remotes are specified.

  # Additional non-default remotes (HTTP or SSH).
  openhw: "https://github.com/openhwgroup"
  # Template remote URL where `{}` is a placeholder for dependency name.
  # If no placeholder is found, "<url>/{}.git" is used.
  internal: "git@gitlab.company.com:internal-repo/{}/release"

# Other packages this package depends on. Optional.
dependencies:
  # Path dependency.
  axi: { path: "../axi" }

  # Git version dependency from default remote.
  apb: "0.2"

  # Git version dependency from non-default remote.
  fll: { version: "0.8", remote: "internal" }

  # Git version dependency with explicit git url.
  ara: { git: "https://github.com/john_doe/ara.git", version: "2" }

  # Git revision dependency (always requires explicit git url).
  spatz: {git: "https://github.com/pulp-platform/spatz.git", rev: "fixes" }

  # Git version dependency, only included if target "test" or "regression_test" is set.
  common_verification: { version: "0.2", target: "any(test, regression_test)" }

  # Git revision dependency, passing a custom target.
  # (equivalent to `-t common_cells:cc_custom_target`).
  common_cells: { version: "1.39", pass_targets: ["cc_custom_target"] }

  # Git version dependency, passing conditional targets to a dependency
  # (equivalent to `-t cva6:cv64a6_imafdcv_sv39` if target 64bit is set,
  # `-t cva6:cv32a6_imac_sv32` if target 32bit is set)
  ariane:
    remote: openhw
    version: 5.3.0
    pass_targets:
      - {target: 64bit, pass: "cv64a6_imafdcv_sv39"}
      - {target: 32bit, pass: "cv32a6_imac_sv32"}

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

  # Source files can use glob patterns to include all matching files:
  - src/more_stuff/**/*.sv

  # Source files can have custom fileendings
  - sv: vendor/encrypted_sv_src.svp
  - v: vendor/encrypted_v_src.vp
  - vhd: vendor/encrypted_vhd_src.e

  # File list in another external file, supporting simple file names, `+define+` and `+incdir+`
  - external_flists:
      - other_file_list.f
    files: []

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

# List of vendorized files from external repositories not supporting bender. Optional.
vendor_package:
    # package name
  - name: lowrisc_opentitan
    # target directory
    target_dir: vendor/lowrisc_opentitan
    # upstream dependency (i.e. git repository similar to dependencies, only supports commit hash)
    upstream: { git: "https://github.com/lowRISC/opentitan.git", rev: "47a0f4798febd9e53dd131ef8c8c2b0255d8c139" }
    # paths to include from upstream dependency. Per default, all paths are included. Optional.
    include_from_upstream:
      - "src/*"
    # paths to exclude from upstream dependency. Paths that also match a pattern in include_from_upstream are excluded. Optional.
    exclude_from_upstream:
      - "ci/*"
    # directory containing patch files. Optional.
    patch_dir: "vendor/patches"
    # custom file mapping from remote repository to local repository, with optional patch_dir containing patches. Optional. Note: mappings make upstreaming patches slightly more complicated. Avoid if not necessary.
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

Git dependencies are automatically checked out and cloned, and are considered for version resolution. The `rev` field can be a git "commit-ish", which essentially is a commit hash, a tag name, or a branch name, where the newest name that starts with the indicated revision is selected. The `version` field can be any of the [semver predicates](https://docs.rs/semver/#requirements), such as a simple version `X.Y.Z` (or `X.Y`), prefixing `=` to only allow that specific version, `~` to limit updates to patches, or defining custom ranges with `>=U.V.W, <X.Y.Z`. More detail on how the `version` field is parsed can be found in the [cargo documentation](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html). The highest compatible version is selected.

All git tags of the form `vX.Y.Z` are considered a version of the package.

> Note: Git tags without the `v` prefix will not be detected by bender. eg: use `v1.2.3`, and **NOT** `1.2.3`

[Relevant dependency resolution code](https://github.com/pulp-platform/bender/blob/master/src/resolver.rs)

#### Git LFS Support

Bender detects if a repository requires Git LFS and if the `git-lfs` tool is installed on your system.

- If the repository uses LFS (detected via `.gitattributes`) and `git-lfs` is installed, Bender will automatically configure LFS and pull the required files.
- If the repository appears to use LFS but `git-lfs` is **not** installed, Bender will print a warning (`W33`) but proceed with the checkout. In this case, you may end up with pointer files instead of the actual large files, which can cause build failures.
- If the repository does not use LFS, Bender skips LFS operations entirely to save time.

#### Target handling

Specified dependencies can be filtered, similar to the sources below. For consistency, this filtering does **NOT** apply during an update, i.e., all dependencies will be accounted for in the Bender.lock file. The target filtering only applies for sources and script outputs. This can be used e.g., to include specific IP only for testing.

#### Passing targets

For sources and script generation, targets can be passed from a package to its dependency directly in the `Bender.yml` file. This allows for enabling and disabling of specific features. Furthermore, these passed targets can be again filtered with a target specification applied to the specific target. This can be used e.g., to enable specific features of dependencies.


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
    # Optional setting to override other files in any source that have the same file basename.
    override_files: true
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

Do not use `:` in your custom targets, as this is used to separate targets to apply to individual packages.

Do not start the target name with `-`, as this is used to remove target application.

[Relevant code](https://github.com/pulp-platform/bender/blob/master/src/target.rs)


### Override Files
If the `override_files` setting is applied to a source, then any files in that source will override other files that share the same basename. The overridden file will be removed from the output and replaced with the overriding file. For example, if `override_files` is applied to a source that has the file `src/core/pkg.sv`, then any other files that are also `pkg.sv` but in a different path will be removed and replaced with `src/core/pkg.sv`. If a file in an override files source does not override any other file, it will not be present in the output.


#### Example:
```yaml
sources:
  - files:
      - src/core/pkg.sv
      - src/core/alu.sv
      - src/core/top.sv
  - target: custom_pkg
    override_files: true
    files:
      - src/custom/pkg.sv
      - src/custom/adder.sv
```
If Bender is run with the `custom_pkg` target, the output files will be:

```
src/custom/pkg.sv
src/core/alu.sv
src/core/top.sv
```

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

To enable specific targets, use the `-t`/`--target` option. Adding a package and colon `<PKG>:<TARGET>` before a target will apply the target only to that specific package. Prefixing a target with `-` will remove that specific target, even for predefined targets (e.g., `-t-<TARGET>` or `-t <PKG>:-<TARGET>`).

To get the sources for a subset of packages, exclude specific packages and their dependencies, or exclude all dependencies, the following flags exist:

- `-p`/`--package`: Specify package to show sources for.
- `-e`/`--exclude`: Specify package to exclude from sources.
- `-n`/`--no-deps`: Exclude all dependencies, i.e. only top level or specified package(s).

For multiple packages (or excludes), multiple `-p` (or `-e`) arguments can be added to the command.


### `config` --- Emit the current configuration

The `bender config` command prints the currently active configuration as JSON to standard output.


### `script` --- Generate tool-specific scripts

The `bender script <format>` command can generate scripts to feed the source code of a package and its dependencies into a vendor tool. These scripts are rendered using internally stored templates with the [tera](https://keats.github.io/tera/docs/) crate, but custom templates can also be used.

Supported formats:

- `flist`: A flat whitespace-separated file list.
- `flist-plus`: A flat file list amenable to be directly inlined into the invocation command of a tool, e.g. `verilate $(bender script flist)`.
- `vsim`: A Tcl compilation script for Mentor ModelSim/QuestaSim.
- `vcs`:  A Tcl compilation script for VCS.
- `verilator`: Command line arguments for Verilator.
- `synopsys`: A Tcl compilation script for Synopsys DC and DE.
- `formality`: A Tcl compilation script for Formality (as reference design).
- `riviera`: A Tcl compilation script for Aldec Riviera-PRO.
- `genus`:  A Tcl compilation script for Cadence Genus.
- `vivado`: A Tcl file addition script for Xilinx Vivado.
- `vivado-sim`: Same as `vivado`, but specifically for simulation targets.
- `precision`: A Tcl compilation script for Mentor Precision.
- `template`: A custom [tera](https://keats.github.io/tera/docs/) template, provided using the `--template` flag.
- `template_json`: The json struct used to render the [tera](https://keats.github.io/tera/docs/) template.

Furthermore, similar flags to the `sources` command exist.

### `pickle` --- Parse and rewrite SystemVerilog sources with Slang

The `bender pickle` command parses SystemVerilog sources with Slang and prints the resulting source again. It supports optional renaming and trimming of unreachable files for specified top modules.

This command is only available when Bender is built with Slang support (for example via `cargo install bender --all-features`).

Useful options:
- `--top <MODULE>`: Trim output to files reachable from one or more top modules.
- `--prefix <PFX>` / `--suffix <SFX>`: Add a prefix and/or suffix to renamed symbols.
- `--exclude-rename <NAME>`: Exclude specific symbols from renaming.
- `--ast-json`: Emit AST JSON instead of source code.
- `--expand-macros`, `--strip-comments`, `--squash-newlines`: Control output formatting.
- `-I <DIR>`, `-D <DEFINE>`: Add extra include directories and preprocessor defines.

Furthermore, similar flags to the `sources` and `script` command exist:

- `-t`/`--target`: Enable specific targets.
- `-p`/`--package`: Specify package to show sources for.
- `-e`/`--exclude`: Specify package to exclude from sources.
- `-n`/`--no-deps`: Exclude all dependencies, i.e. only top level or specified package(s).

Examples:

```sh
# Keep only files reachable from top module `top`.
bender pickle --top my_top

# Rename symbols, but keep selected names unchanged.
bender pickle --top my_top --prefix p_ --suffix _s --exclude-rename my_top
```


### `update` --- Re-resolve dependencies

Whenever you update the list of dependencies, you likely have to run `bender update` to re-resolve the dependency versions, and recreate the `Bender.lock` file.

Calling update with the `--fetch/-f` flag will force all git dependencies to be re-fetched from their corresponding urls.

> Note: Actually this should be done automatically if you add a new dependency. But due to the lack of coding time, this has to be done manually as of now.


### `clone` --- Clone dependency to make modifications

The `bender clone <PKG>` command checks out the package `PKG` into a directory (default `working_dir`, can be overridden with `-p / --path <DIR>`).
To ensure the package is correctly linked in bender, the `Bender.local` file is modified to include a `path` dependency override, linking to the corresponding package.

This can be used for development of dependent packages within the parent repository, allowing to test uncommitted and committed changes, without the worry that bender would update the dependency.

To clean up once the changes are added, ensure the correct version is referenced by the calling packages and remove the path dependency in `Bender.local`, or have a look at `bender snapshot`.

> Note: The location of the override may be updated in the future to prevent modifying the human-editable `Bender.local` file by adding a persistent section to `Bender.lock`.

> Note: The newly created directory will be a git repo with a remote origin pointing to the `git` tag of the resolved dependency (usually evaluated from the manifest (`Bender.yml`)). You may need to adjust the git remote URL to properly work with your remote repository.

### `snapshot` --- Relinks current checkout of cloned dependencies

After working on a dependency cloned with `bender clone <PKG>`, modifications are generally committed to the parent git repository. Once committed, this new hash can be quickly used by bender by calling `bender snapshot`.

With `bender snapshot`, all dependencies previously cloned to a working directory are linked to the git repositories and commit hashes currently checked out. The `Bender.local` is modified correspondingly to ensure reproducibility. Once satisfied with the changes, it is encouraged to properly tag the dependency with a version, remove the override in the `Bender.local`, and update the required version in the `Bender.yml`.

### `parents` --- Lists packages calling the specified package

The `bender parents <PKG>` command lists all packages calling the `PKG` package.

### `checkout` --- Checkout all dependencies referenced in the Lock file

This command will ensure all dependencies are downloaded from remote repositories. This is usually automatically executed by other commands, such as `sources` and `script`.

### `fusesoc` --- Create FuseSoC `.core` files

This command will generate FuseSoC `.core` files from the bender representation for open-source compatibility to the FuseSoC tool. It is intended to provide a basic manifest file in a compatible format, such that any project wanting to include a bender package can do so without much overhead.

If the `--single` argument is provided, only to top-level `Bender.yml` file will be parsed and a `.core` file generated.

If the `--single` argument is *not* provided, bender will walk through all the dependencies and generate a FuseSoC `.core` file where none is present. If a `.core` file is already present in the same directory as the `Bender.yml` for the corresponding dependency, this will be used to link dependencies (if multiple are available, the user will be prompted to select one). Previously generated `.core` files will be overwritten, based on the included `Created by bender from the available manifest file.` comment in the `.core` file.

The `--license` argument will allow you to add multiple comment lines at the top of the generated `.core` files, e.g. a License header string.

The `--fuse-vendor` argument will assign a vendor string to all generated `.core` dependencies for the VLNV name.

The `--fuse-version` argument will assign a version to the top package being handled for the VLNV name.

### `vendor` --- Copy files from dependencies that do not support bender

Collection of commands to manage monorepos. Requires a subcommand.

Please make sure you manage the includes and sources required for these files separately, as this command only fetches the files and patches them.
This is in part based on [lowRISC's `vendor.py` script](https://github.com/lowRISC/opentitan/blob/master/util/vendor.py).

#### `vendor init` --- (Re-)initialize the vendorized dependencies

This command will (re-)initialize the dependencies listed in the `vendor_package` section of the `Bender.yml` file, fetching the files from the remote repositories, applying the necessary patch files, and writing them to the respective `target_dir`.

If the `-n/--no-patch` argument is passed, the dependency is initialized without applying any patches.

#### `vendor diff` --- Print a diff of local, unpatched changes

This command will print a diff to the remote repository with the patches in `patch_dir` applied.

#### `vendor patch` --- Generate a patch file from local changes

If there are local, *staged* changes in a vendored dependency, this command prompts for a commit message and generates a patch for that dependency. The patch is written into `patch_dir`.

If the `--plain` argument is passed, this command will *not* prompt for a commit message and generate a patch of *all* (staged and unstaged) local changes of the vendored dependency.

#### Example workflow

Let's assume we would like to vendor a dependency `my_ip` into a project `monorepo`.
A simple configuration in a `Bender.yml` could look as follows (see the `Bender.yml` description above for more information on this):

```yaml
vendor_package:
  - name: my_ip
    target_dir: deps/my_ip
    upstream: { git: "<url>", rev: "<commit-hash>" }
    patch_dir: "deps/patches/my_ip"
```

Executing `bender vendor init` will now clone this dependency from `upstream` and place it in `target_dir`.

Next, let's assume that we edit two files within the dependency, `deps/my_ip/a` and `deps/my_ip/b`.
We can print these changes with the command `bender vendor diff`.

Now, we would like to generate a patch with the changes in `deps/my_ip/a` (but not those in `deps/my_ip/b`).
We stage the desired changes using `git add deps/my_ip/a` (of course, you can also just stage parts of a file using `git add --patch`).
The command `bender vendor patch` will now ask for a commit message that will be associated with this patch.
Then, it will place a patch that contains our changes in `deps/my_ip/a` into `deps/patches/my_ip/0001-commit-message.patch` (the number will increment if a numbered patch is already present).

We can easily create a corresponding commit in the monorepo.
`deps/my_ip/a` is still staged from the previous step.
We only have to `git add deps/patches/my_ip/0001-commit-message.patch` and `git commit` for an atomic commit in the monorepo that contains both our changes to `deps/my_ip/a` and the corresponding patch.

Upstreaming patches to the dependency is easy as well.
We clone the dependencies' repository, check out `<commit-hash>` and create a new branch.
Now, `git am /path/to/monorepo/deps/patches/my_ip/0001-commit-message.patch` will create a commit out of this patch -- including all metadata such as commit message, author(s), and timestamp.
This branch can then be rebased and a pull request can be opened from it as usual.

Note: when using mappings in your `vendor_package`, the patches will be relative to the mapped directory.
Hence, for upstreaming, you might need to use `git am --directory=<mapping.from>` instead of plain `git am`.

### `completion` --- Generate shell completion script

The `bender completion <SHELL>` command prints a completion script for the given shell.

Installation and usage of these scripts is shell-dependent. Please refer to your shell's documentation
for information on how to install and use the generated script
([bash](https://www.gnu.org/software/bash/manual/html_node/Programmable-Completion.html),
[zsh](https://zsh.sourceforge.io/Doc/Release/Completion-System.html),
[fish](https://fishshell.com/docs/current/completions.html)).

Supported shells:
- `bash`
- `elvish`
- `fish`
- `powershell`
- `zsh`

[aur-bender]: https://aur.archlinux.org/packages/bender
[releases]: https://github.com/pulp-platform/bender/releases
[rust-installation]: https://doc.rust-lang.org/book/ch01-01-installation.html

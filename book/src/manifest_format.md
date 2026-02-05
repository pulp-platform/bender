# Manifest Format (`Bender.yml`)

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


## Dependencies

Dependencies are specified in the `dependencies` section of the package manifest, or the `overrides` section in the configuration file. There are different kinds of dependencies, as described in the following.

### Path

    mydep: { path: "../path/to/mydep" }

Path dependencies are not considered versioned. Either all versions of dependency `mydep` point to the same path, or otherwise the resolution will fail.

### Git

    mydep: { git: "git@github.com:pulp-platform/common_verification.git", rev: "<commit-ish>" }
    mydep: { git: "git@github.com:pulp-platform/common_verification.git", version: "1.1" }

Git dependencies are automatically checked out and cloned, and are considered for version resolution. The `rev` field can be a git "commit-ish", which essentially is a commit hash, a tag name, or a branch name, where the newest name that starts with the indicated revision is selected. The `version` field can be any of the [semver predicates](https://docs.rs/semver/#requirements), such as a simple version `X.Y.Z` (or `X.Y`), prefixing `=` to only allow that specific version, `~` to limit updates to patches, or defining custom ranges with `>=U.V.W, <X.Y.Z`. More detail on how the `version` field is parsed can be found in the [cargo documentation](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html). The highest compatible version is selected.

All git tags of the form `vX.Y.Z` are considered a version of the package.

> Note: Git tags without the `v` prefix will not be detected by bender. eg: use `v1.2.3`, and **NOT** `1.2.3`

[Relevant dependency resolution code](https://github.com/pulp-platform/bender/blob/master/src/resolver.rs)

### Git LFS Support

Bender detects if a repository requires Git LFS and if the `git-lfs` tool is installed on your system.

- If the repository uses LFS (detected via `.gitattributes`) and `git-lfs` is installed, Bender will automatically configure LFS and pull the required files.
- If the repository appears to use LFS but `git-lfs` is **not** installed, Bender will print a warning (`W33`) but proceed with the checkout. In this case, you may end up with pointer files instead of the actual large files, which can cause build failures.
- If the repository does not use LFS, Bender skips LFS operations entirely to save time.

### Target handling

Specified dependencies can be filtered, similar to the sources below. For consistency, this filtering does **NOT** apply during an update, i.e., all dependencies will be accounted for in the Bender.lock file. The target filtering only applies for sources and script outputs. This can be used e.g., to include specific IP only for testing.

### Passing targets

For sources and script generation, targets can be passed from a package to its dependency directly in the `Bender.yml` file. This allows for enabling and disabling of specific features. Furthermore, these passed targets can be again filtered with a target specification applied to the specific target. This can be used e.g., to enable specific features of dependencies.


## Sources

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


## Targets

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

## Vendor

Section to list files and directories copied and patched within this repository from external repositories not supporting bender.
To update, see below `vendor` command.

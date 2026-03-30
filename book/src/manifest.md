# Manifest (`Bender.yml`)

The package manifest describes the package, its metadata, its dependencies, and its source files. All paths in the manifest may be relative, in which case they are understood to be relative to the directory that contains the manifest. A manifest is required for each bender package.

It is strongly recommended to start the Manifest file with a license header (open-source or proprietary) as a comment. This provides clarity on the project's usage.

```yaml
# Copyright (c) 2026 ETH Zurich and University of Bologna.
# Licensed under the Apache License, Version 2.0, see LICENSE for details.
# SPDX-License-Identifier: Apache-2.0
```

The first section of the manifest should be the package description, outlining the package itself. The package name is required and must match with the name used to call this bender package as a dependency. The name is interpreted in a case-insensitive manner within bender, so additional care should be taken to avoid name conflicts.

Additionally, authors and a description can be specified. Bender currently does not use these fields but supports their existence.

```yaml
# Package metadata. Required.
package:
  # The name of the package. Required.
  name: magic-chip

  # The list of package authors and contributors. Optional.
  authors: ["John Doe <john@doe.si>"]

  # A short description of the package. Optional.
  description: "This is a magical chip"
```

The `remotes` section allows you to define shorthand names for Git repositories. This makes the `dependencies` section much cleaner by avoiding repeated long URLs.

```yaml
# Specify git remotes for dependencies. Optional.
remotes:
  pulp: "https://github.com/pulp-platform"
  openhw:
    url: "https://github.com/openhwgroup"
    default: true # Used if no remote is specified in a dependency
```

The next section in the manifest is the dependencies.
 Basic projects not requiring any modules from dependencies can omit this section. All packages this bender project depends on should be listed here for proper functionality, including the version requirements. More details on the specific format can be found [here](./dependencies.md).

```yaml
# Other packages this package depends on. Optional.
dependencies:
    common_cells: { git: "https://github.com/pulp-platform/common_cells.git", version: "1.39" }
```

The sources section lists the HDL source files belonging to this package. It is optional for packages that only provide headers or are otherwise used without their own source files. More details on the format can be found [here](./sources.md).

```yaml
# List of source files in this package. Optional.
sources:
  # Individual source files.
  - src/pkg.sv
  - src/top.sv
```

Include directories that should be passed to all packages depending on this one are listed under `export_include_dirs`. This is the standard mechanism for sharing header files. The include directories listed here also apply to all files in the current package.

```yaml
# Include directories exported to dependent packages. Optional.
export_include_dirs:
  - include
```

Setting `frozen` to `true` prevents Bender from updating dependencies beyond what is recorded in the lockfile. This is useful for chip packages in tapeout mode where dependency changes would require disastrous amounts of re-verification.

```yaml
# Freeze dependency updates. Optional. Defaults to false.
frozen: true
```

The workspace section provides additional options for the local working environment. It is not relevant for library packages and is typically only used in the top-level chip package. Package links creates symlinks for the specified dependencies to known locations, while checkout_dir enforces the checkout of all dependencies to a specific location.

```yaml
# Additional workspace configuration. Optional.
workspace:
  # Create symlinks to checked-out dependencies.
  package_links:
    links/axi: axi
    common: common_cells

  # Directory where dependencies will be checked out. Optional.
  # Once set, Bender performs the initial checkout and then leaves the directory
  # untouched. Useful for chip packages that commit all dependencies into their
  # own version control.
  checkout_dir: deps
```

Packages can expose shell scripts as bender subcommands using the `plugins` section. These commands are available to packages that depend on this one and can be invoked as `bender <cmd>`.

```yaml
# Package-provided commands callable as `bender <cmd>`. Optional.
plugins:
  hello: scripts/hello.sh
```

The `vendor_package` section lists files copied from external repositories that do not support Bender. Vendored files are managed with the `bender vendor` command. More details on the workflow can be found [here](./workflow/vendor.md).

```yaml
# Vendorized files from external repositories not supporting Bender. Optional.
vendor_package:
  - name: lowrisc_opentitan
    target_dir: vendor/lowrisc_opentitan
    # Only commit hashes are supported for the upstream revision.
    upstream: { git: "https://github.com/lowRISC/opentitan.git", rev: "47a0f4798febd9e53dd131ef8c8c2b0255d8c139" }
    # Custom file mappings from upstream paths to local paths. Optional.
    mapping:
      - { from: "hw/ip/prim/rtl/prim_subreg.sv", to: "src/prim_subreg.sv" }
```

# Sources

The `sources` section in `Bender.yml` defines the source files, include directories, and preprocessor definitions that make up your package.

## Basic File Listing

The simplest way to include files is to list them as strings. All paths are relative to the location of the `Bender.yml` file.

```yaml
sources:
  - src/my_pkg.sv
  - src/my_ip.sv
  - src/my_vhdl.vhd
```

## Source Groups

You can group files together to apply common settings like include directories, preprocessor defines, or [targets](./targets.md).

```yaml
sources:
  - include_dirs:
      - include
      - src/common/include
    defines:
      USE_FAST_ALU: ~        # Define without a value
      BIT_WIDTH: 64          # Define with a value
    target: synthesis
    files:
      - src/rtl/alu.sv
      - src/rtl/top.sv
```

- **include_dirs**: Paths added to the `+incdir+` flag during compilation.
- **defines**: Preprocessor macros added via `+define+`.
- **target**: A [target expression](./targets.md) that determines if this entire group is included in the current flow.

## Glob Patterns

Bender supports glob patterns for automatically including multiple files without listing them individually:

```yaml
sources:
  - src/*.sv                # All .sv files in the src directory
  - src/submodules/**/*.sv  # All .sv files in all subdirectories (recursive)
```

## Custom File Types

While Bender automatically detects file types by standard extensions (`.sv`, `.v`, `.vhd`), you can explicitly specify the type for encrypted or non-standard files:

```yaml
sources:
  - sv: vendor/encrypted_src.svp
  - v: vendor/legacy_code.vp
  - vhd: vendor/encrypted_vhdl.e
```

## External File Lists (`external_flists`)

If you have existing EDA tool file lists (often `.f` or `.flist`), you can include them directly. Bender will attempt to parse them for source files, include directories, and defines:

```yaml
sources:
  - external_flists:
      - scripts/files.f
    files: []
```

## File Overrides

The `override_files: true` flag allows a source group to replace files with the same basename from other parts of the dependency tree. This is a powerful mechanism for "patching" dependencies or swapping implementations at the top level.

```yaml
sources:
  - override_files: true
    files:
      - patches/axi_fifo.sv # Will replace any 'axi_fifo.sv' found in the dependency tree
```

## Exported Include Directories

If your package provides headers that its *dependents* need (e.g., UVM macros or shared packages), use the `export_include_dirs` section at the top level of the manifest:

```yaml
package:
  name: my_utils

export_include_dirs:
  - include
```

Any package that depends on `my_utils` will automatically have the `include` directory added to its include search path.

> **Best Practice:** To avoid naming collisions, place your headers in a sub-folder named after your package.
>
> **The Problem:** If two packages (`axi` and `apb`) both export a header named `typedefs.svh`, a file including `` `include "typedefs.svh" `` will be ambiguous, and the compiler will pick whichever directory it finds first in the include path.
>
> **The Solution:** Structure your files as `include/axi/typedefs.svh` and `include/apb/typedefs.svh`, then include them with the package prefix:
> ```systemverilog
> `include "axi/typedefs.svh"
> `include "apb/typedefs.svh"
> ```

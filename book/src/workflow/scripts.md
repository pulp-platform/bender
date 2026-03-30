# Generating Tool Scripts

Bender's `script` command is the bridge between dependency management and your EDA tools. It generates TCL or shell scripts that include all necessary source files, include directories, and preprocessor defines in the correct order.

## Basic Usage

The `script` command requires a format (the target EDA tool or output style):

```sh
bender script <FORMAT>
```

For example, to generate a script for ModelSim/QuestaSim:

```sh
bender script vsim > compile.tcl
```

## Supported Formats

Bender supports a wide range of EDA tools and generic formats:

| Format | Tool / Use Case | Output Type |
| :--- | :--- | :--- |
| `vsim` | ModelSim / QuestaSim | TCL |
| `vcs` | Synopsys VCS | Shell |
| `verilator` | Verilator | Shell |
| `vivado` | Xilinx Vivado (Synthesis) | TCL |
| `vivadosim` | Xilinx Vivado (Simulation) | TCL |
| `synopsys` | Synopsys Design Compiler | TCL |
| `genus` | Cadence Genus | TCL |
| `flist` / `flist-plus` | Generic file lists | Text |

For a full list of formats and their specific options, see the [Command Reference](../commands.md#bender-script).

## Targets and Filtering

When you generate a script, Bender automatically activates certain [built-in targets](../targets.md#built-in-targets). For example, `bender script vsim` automatically enables the `simulation` and `vsim` targets.

You can manually enable additional targets using the `-t/--target` flag:

```sh
# Generate a simulation script with the 'test' and 'gate' targets enabled
bender script vsim -t test -t gate > compile.tcl
```

## Useful Flags

The `script` command provides several flags to fine-tune the generated output:

### Package Filtering
- **`-p/--package <PKG>`**: Only include source files from the specified package (and its dependencies).
- **`-n/--no-deps`**: Exclude all dependencies. This generates a script containing only the files from the current package or the packages explicitly listed with `-p`.
- **`-e/--exclude <PKG>`**: Exclude a specific package from the generated script.

### RTL Assumption
- **`--assume-rtl`**: Automatically adds the `rtl` target to any source group that does not have an explicit target specification. This is a common shorthand for generating synthesis scripts without having to tag every RTL file.

### Compilation Control
- **`--compilation-mode <separate|common>`**: 
    - `separate` (default): Compiles each source group as a separate unit. 
    - `common`: Attempts to compile all source groups together in a single compilation unit.
- **`--no-abort-on-error`**: Tells the EDA tool to continue analysis/compilation even if errors are encountered in individual files.

### Metadata and Debugging
- **`--source-annotations`**: Includes comments in the generated script that indicate which Bender package and source group each file belongs to. This is very helpful for debugging ordering or missing-file issues.
- **`--no-default-target`**: Disables the automatic activation of built-in targets (like `simulation` for `vsim`). Use this if you want absolute control over which targets are active.

## Advanced Options

### Adding Defines
You can pass additional preprocessor macros to all files in the script using the `-D` flag:

```sh
bender script vsim -D USE_DDR4 -D CLK_FREQ=100
```

### Relative Paths
For generic file lists (`flist` or `flist-plus`), you can force Bender to use paths relative to the current directory using the `--relative-path` flag. This is useful for sharing file lists between different machines or environments.

### Custom Templates
Bender uses the [Tera](https://keats.github.io/tera/) templating engine for its scripts. If the built-in formats don't meet your needs, you can provide your own template file:

```sh
bender script template --template my_custom_format.tera > output.txt
```

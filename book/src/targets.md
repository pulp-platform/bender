# Targets

Targets are the primary mechanism in Bender for managing project configurations. They allow you to conditionally include source files, include directories, and dependencies based on the current context (e.g., simulation vs. synthesis, or FPGA vs. ASIC).

## Target Expressions

Bender uses a simple boolean expression language for targets:

- `*`: Wildcard, matches all target sets.
- `name`: Matches if the target `name` is active (case-insensitive).
- `all(T1, T2, ...)`: Matches if **all** arguments match (boolean AND).
- `any(T1, T2, ...)`: Matches if **at least one** argument matches (boolean OR).
- `not(T)`: Matches if `T` does **not** match (boolean NOT).
- `(T)`: Parentheses for grouping.

### Syntax Rules
Target names can contain alphanumeric characters, dots (`.`), underscores (`_`), and hyphens (`-`).

**Restrictions:**
- They **cannot** contain colons (`:`), as colons are used for package-specific targets in the CLI.
- They **cannot** start with a hyphen (`-`), as leading hyphens are used to disable targets in the CLI.

### Logical Examples
- `all(asic, synthesis)`: Matches when both 'asic' and 'synthesis' are set.
- `any(vsim, vcs, riviera)`: Matches if any of the listed simulation tools are active.
- `not(simulation)`: Matches only when 'simulation' is **not** set.
- `any(test, all(rtl, simulation))`: Matches for testbenches, or for RTL code during simulation.

## Usage in Bender.yml

### Source Groups
You can wrap a group of files in a `target` specification. This allows you to manage different implementations or verification components within the same package.

#### Testbench Inclusion
Ensures that verification-only code is never included in the synthesis or production flow:

```yaml
sources:
  # RTL sources
  - files:
      - src/core.sv
      - src/alu.sv

  # Testbench only
  - target: test
    files:
      - tb/driver.sv
      - tb/tb_top.sv
```

#### Simulation vs. Synthesis
Commonly used to swap between an actual hardware macro and a fast behavioral model for simulation:

```yaml
sources:
  # Behavioral model for faster simulation
  - target: all(simulation, not(synthesis))
    files:
      - src/behavioral/ip_model.sv
```

#### Technology Selection
Useful when choosing between different physical implementations, such as ASIC standard cells versus FPGA primitives:

```yaml
sources:
  - target: asic
    files:
      - target/asic/clock_gate.sv
  - target: fpga
    files:
      - target/fpga/clock_gate.sv
```

#### Core Configuration
Targets can be used to select between different hardware architectures or feature sets:

```yaml
sources:
  # 32-bit architecture
  - target: rv32
    files:
      - src/core/alu_32.sv
      - src/core/regfile_32.sv
  # 64-bit architecture
  - target: rv64
    files:
      - src/core/alu_64.sv
      - src/core/regfile_64.sv
```

### Dependencies
You can make a dependency conditional using target expressions. This is commonly used to include verification IP or platform-specific components only when needed:

```yaml
dependencies:
  # Included only during simulation
  uvm: { version: "1.2.0", target: simulation }
  # Included for either test or simulation
  common_verification: { version: "0.2", target: any(test, simulation) }
```

## Built-in Targets

Bender automatically activates certain targets based on the subcommand and output format. These "default targets" ensure that tool-specific workarounds or flow-specific files are included correctly. You can disable this behavior with the `--no-default-target` flag.

### Script Format Targets

The `bender script` command activates the following targets based on the chosen format:

| Format | Default Targets |
| :--- | :--- |
| `flist`, `flist-plus` | `flist` |
| `vsim` | `vsim`, `simulation` |
| `vcs` | `vcs`, `simulation` |
| `verilator` | `verilator`, `synthesis` |
| `synopsys` | `synopsys`, `synthesis` |
| `formality` | `synopsys`, `synthesis`, `formality` |
| `riviera` | `riviera`, `simulation` |
| `genus` | `genus`, `synthesis` |
| `vivado` | `vivado`, `fpga`, `xilinx`, `synthesis` |
| `vivadosim` | `vivado`, `fpga`, `xilinx`, `simulation` |
| `precision` | `precision`, `fpga`, `synthesis` |

### Special Targets

- **RTL:** If you use the `--assume-rtl` flag, Bender will automatically assign the `rtl` target to any source group that does not have an explicit target specification.
- **ASIC:** While `asic` is a common convention, it is **not** set automatically by Bender. It should be manually activated via `-t asic` when needed.

## Activating Targets via CLI

Use the `-t` or `--target` flag with Bender commands:

```bash
# Enable the 'test' target
bender script vsim -t test

# Enable multiple targets
bender script vivado -t synthesis -t fpga
```

### Advanced CLI Syntax
- **Package-specific:** `-t my_pkg:my_target` activates `my_target` only for `my_pkg`.
- **Negative targets:** `-t -old_target` explicitly disables `old_target`.

## Passing Targets Hierarchically

Bender allows you to "configure" your dependencies by passing specific targets down to them using the `pass_targets` field. This is a powerful way to propagate global settings or select implementations in sub-modules.

### Simple Passing
You can pass a target name as a string, which will then always be active for that specific dependency:

```yaml
dependencies:
  my_submodule:
    version: "1.0.0"
    pass_targets: ["enable_debug"] # 'enable_debug' is always active for my_submodule
```

### Conditional Passing
You can also pass targets conditionally, based on the targets active in the parent package:

```yaml
dependencies:
  ariane:
    version: 5.3.0
    pass_targets:
      - {target: rv64, pass: "cv64a6_imafdcv_sv39"}
      - {target: rv32, pass: "cv32a6_imac_sv32"}
```

In this example, if the `rv64` target is active globally, Bender will ensure the `cv64a6_imafdcv_sv39` target is active specifically for the `ariane` dependency.

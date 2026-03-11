# Targets

Targets are the primary mechanism in Bender for managing project configurations. They allow you to conditionally include source files, include directories, and dependencies based on the current context (e.g., simulation vs. synthesis, or FPGA vs. ASIC).

## Target Expressions

Bender uses a simple boolean expression language for targets:

- `*`: Matches any target (wildcard).
- `name`: Matches if the target `name` is active.
- `all(T1, T2, ...)`: Matches if **all** listed targets are active (AND).
- `any(T1, T2, ...)`: Matches if **any** of the listed targets are active (OR).
- `not(T)`: Matches if target `T` is **not** active (NOT).
- `(T)`: Parentheses for grouping.

Target names are case-insensitive and cannot contain colons (`:`) or start with a hyphen (`-`).

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
Dependencies can also be conditional. This is useful for verification IP that is only needed during testing:

```yaml
dependencies:
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

A parent package can pass targets to its dependencies using `pass_targets`. This is useful for propagating configuration flags or selecting specific implementations in sub-modules:

```yaml
dependencies:
  ariane:
    version: 5.3.0
    pass_targets:
      - {target: rv64, pass: "cv64a6_imafdcv_sv39"}
      - {target: rv32, pass: "cv32a6_imac_sv32"}
```

In this example, if the `rv64` target is active globally, Bender will ensure the `cv64a6_imafdcv_sv39` target is active specifically for the `ariane` dependency.

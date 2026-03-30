# Getting Started

This guide will walk you through creating your first Bender project, adding a dependency, and generating a simulation script.

## 1. Create a New Project

Start by creating a directory for your project and initializing it with Bender:

```sh
mkdir my_new_ip
cd my_new_ip
bender init
```

This creates a default `Bender.yml` file. Bender will automatically try to fill in your name and email from your Git configuration.

## 2. Add a Dependency

Open `Bender.yml` in your editor and add the `common_cells` library to the `dependencies` section:

```yaml
package:
  name: my_new_ip
  authors: ["John Doe <john@doe.com>"]

dependencies:
  common_cells: { git: "https://github.com/pulp-platform/common_cells.git", version: "1.21.0" }

sources:
  - src/my_new_ip.sv
```

## 3. Resolve and Checkout

Now, tell Bender to resolve the version of `common_cells` and download it:

```sh
bender update
```

This command creates a `Bender.lock` file (the "exact" version chosen) and downloads the source code into a hidden `.bender` directory.

## 4. Add Source Code

Create a simple SystemVerilog file in `src/my_new_ip.sv`:

```systemverilog
module my_new_ip (
  input  logic clk_i,
  output logic dummy_o
);
  // We can use a module from common_cells here!
  // ...
endmodule
```

## 5. Generate a Simulation Script

Finally, generate a compilation script for your EDA tool (e.g., QuestaSim/ModelSim):

```sh
bender script vsim > compile.tcl
```

You can now run `vsim -do compile.tcl` in your terminal to compile the entire project.

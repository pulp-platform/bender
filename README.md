# bender

Bender is a dependency management tool for hardware design projects. It provides a way to define dependencies among IPs, execute unit tests, and verify that the source files are valid input for various simulation and synthesis tools.


## Workflow

The workflow of bender is based on a configuration and a lock file. The configuration file lists the sources, dependencies, and tests of the package at hand. The lock file is used by the tool to track which exact version of a package is being used. Adding this file to version control, e.g. for chips that will be taped out, makes it easy to reconstruct the exact IPs that were used during a simulation, synthesis, or tapeout.

Upon executing any command, bender checks to see if dependencies have been added to the configuration file that are not in the lock file. It then tries to find a revision for each added dependency that is compatible with the other dependencies and add that to the lock file. In a second step, bender tries to ensure that the checked out revisions match the ones in the lock file. If not possible, appropriate errors are generated.

The update command reevaluates all dependencies in the configuration file and tries to find for each a revision that satisfies all recursive constraints. If semantic versioning is used, this will update the dependencies to newer versions within the bounds of the version requirement provided in the configuration file.


## Package Structure

Bender looks for the following three files in a package:

- `Bender.yml`:

  This is the main package manifest, and the only required file for a directory to be recognized as a Bender package. It contains metadata, dependencies, and source file lists.

- `Bender.lock`:

  The lock file is generated once all dependencies have been successfully resolved. It contains the exact revision of each dependency. This file *may* be put under version control to allow for reproducible builds. This is handy for example upon taping out a design. If the lock file is missing or a new dependency has been added, it is regenerated.

- `Bender.local`:

  This file contains local configuration overrides. It should be ignored in version control, i.e. added to `.gitignore`. This file can be used to override dependencies with local variants. It is also used when the user asks for a local working copy of a dependency.


## Targets

Targets are flags that can be used to filter source files and dependencies. They are used to differentiate the step in the ASIC/FPGA design flow, the EDA tool, technology target, etc. The following table lists the targets that should be adhered to:

- `test`: Set this target when verifying your design through unit tests or testbenches. Use the target to enable source files that contain testbenches, UVM models, etc.

- **Tool**: You should set exactly one of the following to indicate with which tool you are working.

  - `vsim`: Set this target when working with ModelSim vsim. Automatically set by the *bender-vsim* plugin.

  - `vcs`: Set this target when working with Synopsys VCS. Automatically set by the *bender-vcs* plugin.

  - `synopsys`: Set this target when working with Synopsys Design Compiler. Automatically set by the *bender-synopsys* plugin.

  - `vivado`: Set this target when working with Xilinx Vivado. Automatically set by the *bender-vivado* plugin.

- **Abstraction**: You should set exactly one of the following to indicate at which abstraction level you are working on.

  - `rtl`: Set this target when working with the Register Transfer Level description of a design. If this target is set, only behavioural and no technology-specific modules should be used.

  - `gate`: Set this target when working with gate-level netlists, for example after synthesis or layout.

- **Stage**: You should set exactly one of the following to indicate what you are using the design for.

  - `simulation`: Set this target if you simulate the design. This target should be used to include protocol checkers and other verification modules.

  - `synthesis`: Set this target if you synthesize the design. The target can be used to disable various parts of the source files which are not synthesizable.

- **Technology**: You should set exactly one of the following pairs of targets to indicate what FPGA or ASIC technology you target.

  - `fpga xilinx`
  - `fpga altera`
  - `asic umc65`
  - `asic gf28`
  - `asic gf22`
  - `asic stm28fdsoi`

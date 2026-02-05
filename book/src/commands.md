# Commands

`bender` is the entry point to the dependency management system. Bender always operates within a package; starting at the current working directory, search upwards the file hierarchy until a `Bender.yml` is found, which marks the package.


## `path` --- Get the path of a checked-out package

The `bender path <PKG>` prints the path of the checked-out version of package `PKG`.

Useful in scripts:

    #!/bin/bash
    cat `bender path mydep`/src/hello.txt


## `packages` --- Display the dependency graph

- `bender packages`: List the package dependencies. The list is sorted and grouped according to a topological sorting of the dependencies. That is, leaf dependencies are compiled first, then dependent ones.
- `bender packages -f`: Produces the same list, but flattened.
- `bender packages -g`: Produces a graph description of the dependencies of the form `<pkg>TAB<dependencies...>`.


## `sources` --- List source files
[Code](https://github.com/pulp-platform/bender/blob/master/src/cmd/sources.rs)

Produces a *sources manifest*, a JSON description of all files needed to build the project.

The manifest is recursive by default; meaning that dependencies and groups are nested. Use the `-f`/`--flatten` switch to produce a simple flat listing.

To enable specific targets, use the `-t`/`--target` option. Adding a package and colon `<PKG>:<TARGET>` before a target will apply the target only to that specific package. Prefixing a target with `-` will remove that specific target, even for predefined targets (e.g., `-t-<TARGET>` or `-t <PKG>:-<TARGET>`).

To get the sources for a subset of packages, exclude specific packages and their dependencies, or exclude all dependencies, the following flags exist:

- `-p`/`--package`: Specify package to show sources for.
- `-e`/`--exclude`: Specify package to exclude from sources.
- `-n`/`--no-deps`: Exclude all dependencies, i.e. only top level or specified package(s).

For multiple packages (or excludes), multiple `-p` (or `-e`) arguments can be added to the command.


## `config` --- Emit the current configuration

The `bender config` command prints the currently active configuration as JSON to standard output.


## `script` --- Generate tool-specific scripts

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


## `update` --- Re-resolve dependencies

Whenever you update the list of dependencies, you likely have to run `bender update` to re-resolve the dependency versions, and recreate the `Bender.lock` file.

Calling update with the `--fetch/-f` flag will force all git dependencies to be re-fetched from their corresponding urls.

> Note: Actually this should be done automatically if you add a new dependency. But due to the lack of coding time, this has to be done manually as of now.


## `clone` --- Clone dependency to make modifications

The `bender clone <PKG>` command checks out the package `PKG` into a directory (default `working_dir`, can be overridden with `-p / --path <DIR>`).
To ensure the package is correctly linked in bender, the `Bender.local` file is modified to include a `path` dependency override, linking to the corresponding package.

This can be used for development of dependent packages within the parent repository, allowing to test uncommitted and committed changes, without the worry that bender would update the dependency.

To clean up once the changes are added, ensure the correct version is referenced by the calling packages and remove the path dependency in `Bender.local`, or have a look at `bender snapshot`.

> Note: The location of the override may be updated in the future to prevent modifying the human-editable `Bender.local` file by adding a persistent section to `Bender.lock`.

> Note: The newly created directory will be a git repo with a remote origin pointing to the `git` tag of the resolved dependency (usually evaluated from the manifest (`Bender.yml`)). You may need to adjust the git remote URL to properly work with your remote repository.

## `snapshot` --- Relinks current checkout of cloned dependencies

After working on a dependency cloned with `bender clone <PKG>`, modifications are generally committed to the parent git repository. Once committed, this new hash can be quickly used by bender by calling `bender snapshot`.

With `bender snapshot`, all dependencies previously cloned to a working directory are linked to the git repositories and commit hashes currently checked out. The `Bender.local` is modified correspondingly to ensure reproducibility. Once satisfied with the changes, it is encouraged to properly tag the dependency with a version, remove the override in the `Bender.local`, and update the required version in the `Bender.yml`.

## `parents` --- Lists packages calling the specified package

The `bender parents <PKG>` command lists all packages calling the `PKG` package.

## `checkout` --- Checkout all dependencies referenced in the Lock file

This command will ensure all dependencies are downloaded from remote repositories. This is usually automatically executed by other commands, such as `sources` and `script`.

## `fusesoc` --- Create FuseSoC `.core` files

This command will generate FuseSoC `.core` files from the bender representation for open-source compatibility to the FuseSoC tool. It is intended to provide a basic manifest file in a compatible format, such that any project wanting to include a bender package can do so without much overhead.

If the `--single` argument is provided, only to top-level `Bender.yml` file will be parsed and a `.core` file generated.

If the `--single` argument is *not* provided, bender will walk through all the dependencies and generate a FuseSoC `.core` file where none is present. If a `.core` file is already present in the same directory as the `Bender.yml` for the corresponding dependency, this will be used to link dependencies (if multiple are available, the user will be prompted to select one). Previously generated `.core` files will be overwritten, based on the included `Created by bender from the available manifest file.` comment in the `.core` file.

The `--license` argument will allow you to add multiple comment lines at the top of the generated `.core` files, e.g. a License header string.

The `--fuse-vendor` argument will assign a vendor string to all generated `.core` dependencies for the VLNV name.

The `--fuse-version` argument will assign a version to the top package being handled for the VLNV name.

## `vendor` --- Copy files from dependencies that do not support bender

Collection of commands to manage monorepos. Requires a subcommand.

Please make sure you manage the includes and sources required for these files separately, as this command only fetches the files and patches them.
This is in part based on [lowRISC's `vendor.py` script](https://github.com/lowRISC/opentitan/blob/master/util/vendor.py).

### `vendor init` --- (Re-)initialize the vendorized dependencies

This command will (re-)initialize the dependencies listed in the `vendor_package` section of the `Bender.yml` file, fetching the files from the remote repositories, applying the necessary patch files, and writing them to the respective `target_dir`.

If the `-n/--no-patch` argument is passed, the dependency is initialized without applying any patches.

### `vendor diff` --- Print a diff of local, unpatched changes

This command will print a diff to the remote repository with the patches in `patch_dir` applied.

### `vendor patch` --- Generate a patch file from local changes

If there are local, *staged* changes in a vendored dependency, this command prompts for a commit message and generates a patch for that dependency. The patch is written into `patch_dir`.

If the `--plain` argument is passed, this command will *not* prompt for a commit message and generate a patch of *all* (staged and unstaged) local changes of the vendored dependency.

### Example workflow

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

## `completion` --- Generate shell completion script

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

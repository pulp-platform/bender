# Inspecting Source Files

Once your dependencies are resolved and checked out, you can use Bender to inspect the collection of source files, include directories, and defines that make up your project.

## Listing All Sources

The `sources` command prints a hierarchical view of all files in your project, including those from dependencies:

```sh
bender sources
```

> **Note:** The output is in **JSON format**. This is intended for machine consumption (e.g., by other scripts or build systems) and is not primarily meant to be read by humans.

## Filtering Sources

In most hardware projects, you only want to see a subset of files at any given time (e.g., only synthesis RTL or only a specific IP's files).

### Filtering by Target
Use the `-t/--target` flag to filter the source list based on [target expressions](../targets.md):

```sh
# Show only files relevant for simulation
bender sources -t simulation

# Show files for synthesis, excluding those specifically for FPGAs
bender sources -t synthesis -t "-fpga"
```

### Filtering by Package
If you only want to see the files for a specific package (and its dependencies), use the `-p/--package` flag:

```sh
bender sources -p common_cells
```

To see **only** the files of the specified package without its dependencies, add the `--no-deps` flag.

## Flattened Output

If you need a simple list of all files (e.g., to pipe into another tool), use the `-f/--flat` flag:

```sh
bender sources -f
```

This removes the hierarchical grouping and target information, providing a clean list of absolute paths.

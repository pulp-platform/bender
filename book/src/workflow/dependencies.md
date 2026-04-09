# Adding and Updating Dependencies

As working with dependencies is one of bender's main features, there are a few commands to ensure functionality and assist with understanding the dependency structure.

## New dependencies
Once new dependencies are added to the manifest, bender needs to first be made aware of their existence. Otherwise, some commands will not work correctly and return an error. To update dependencies, run the following command:

```sh
bender update
```

In case other dependencies already exist and you do not want to re-resolve these, you can add the `--new-only` flag to the update command.

## Updating dependencies
Similar to when adding new dependencies, updating existing dependencies to more recent versions is also done with the `update` command.

### Single Dependency Update
You don't always have to update your entire dependency tree. To update only a specific package, provide its name:

```sh
bender update <PKG_NAME>
```

### Recursive Updates
By default, updating a single dependency will not update its own child dependencies. If you want to update a package and all of its dependencies recursively, use the `-r/--recursive` flag:

```sh
bender update <PKG_NAME> --recursive
```

## Checking out Dependencies

While `bender update` resolves versions and updates the lockfile, it does not necessarily download all the source code. To ensure all dependencies are locally available, use:

```sh
bender checkout
```

Bender will download the exact revisions specified in `Bender.lock`. This command is safe to run multiple times; it will only download missing packages or update those that have changed in the lockfile.

> **Note:** Many other commands (like `sources` or `script`) will automatically trigger a checkout if they detect missing dependencies.

## Inspecting the Dependency Tree

To see the full list of dependencies and how they relate to each other, use the `packages` command:

```sh
bender packages
```

This prints a tree structure showing every package in your project, its resolved version, and its source. Use the `-f/--flat` flag if you just want a simple list of names and versions.

## Finding Package Paths

If you need to know where a specific dependency is stored (e.g., to point an external tool to it), use:

```sh
bender path <PKG_NAME>
```

This will output the absolute path to the package's root directory.

## Checking Usage (`parents`)

If you are wondering why a specific dependency was included or why a certain version was forced, you can use:

```sh
bender parents <PKG_NAME>
```

This will show you all packages in your tree that depend on `<PKG_NAME>` and what version constraints they have placed on it.

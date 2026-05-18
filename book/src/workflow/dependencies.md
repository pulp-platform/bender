# Adding and Updating Dependencies

As working with dependencies is one of bender's main features, there are a few commands to ensure functionality and assist with understanding the dependency structure. For background on how dependencies are declared in the [`Bender.yml`](../manifest.md) manifest, see [Dependencies](../dependencies.md).

## New dependencies
Once new dependencies are added to the [manifest](../manifest.md), bender needs to first be made aware of their existence. Otherwise, the internally used dependency tree will be incorrect or mismatched, and some commands will not work correctly, returning an error. To update dependencies, run the following command:

```sh
bender update
```

In case other dependencies already exist and you do not want to re-resolve these, you can add the `--new-only` flag to the update command.

> **Note:** On update, bender creates or modifies the [`Bender.lock`](../lockfile.md), which keeps track of the currently selected dependency versions.

> **Note:** Most bender commands will automatically run an update if no [`Bender.lock`](../lockfile.md) is found.

## Updating dependencies
Similar to when adding new dependencies, updating existing dependencies to more recent versions is also done with the `update` command.

During an update, Bender picks the highest SemVer-compatible version and a unique revision that simultaneously satisfies the constraints of every package in the dependency tree. If the requirements conflict and no single revision satisfies them, Bender will prompt you to choose how to resolve the conflict. See [Dependencies](../dependencies.md) for more on version resolution.

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

While `bender update` resolves versions and updates the [`Bender.lock`](../lockfile.md), it does not necessarily download all the source code. To ensure all dependencies are locally available, use:

```sh
bender checkout
```

Bender will download the exact revisions specified in [`Bender.lock`](../lockfile.md). This command is safe to run multiple times; it will only download missing packages or update those that have changed in the [`Bender.lock`](../lockfile.md).

> **Note:** Many other commands (like `sources` or `script`) will automatically trigger a checkout if they detect missing dependencies.

## Inspecting the Dependency Tree (`packages`)

To see the full list of dependencies and how they relate to each other, use the `packages` command:

```sh
bender packages
```

This prints a tree structure showing every package in your project, its resolved version, and its source. Use the `-f/--flat` flag if you just want a simple list of names and versions.

## Finding Package Paths (`path`)

If you need to know where a specific dependency is stored (e.g., to point an external tool to it), use:

```sh
bender path <PKG_NAME>
```

This will output the absolute path to the dependency's checkout directory. If the package has not yet been checked out, `bender path` will check it out first.

## Checking Usage (`parents`)

If you are wondering why a specific dependency was included or why a certain version was forced, you can use:

```sh
bender parents <PKG_NAME>
```

This will show you all packages in your tree that depend on `<PKG_NAME>` and what version constraints they have placed on it.

## Auditing the Dependency Tree (`audit`)

To get a quick overview of the state of your dependencies, including outdated packages, version conflicts, and dependencies pinned to a Git hash rather than a SemVer version, use:

```sh
bender audit
```

The output classifies each package as one of:
- **Up-to-date:** the resolved version is the highest available.
- **Auto-update:** a newer version exists that still satisfies all constraints; can be picked up by running `bender update`.
- **Update:** a newer version exists but is outside the current SemVer constraints; requires manual updating of constraints in [`Bender.yml`](../manifest.md).
- **Hash:** the dependency is pinned to a specific Git revision rather than a SemVer version.
- **Path:** the dependency points to a local directory.
- **Conflict:** the dependency tree has incompatible version requirements or remote URLs — use `bender parents <PKG>` to investigate.

Pass `--only-update` to show only packages that have a possible update, or `-f/--fetch` to force a re-fetch of all Git remotes before auditing.

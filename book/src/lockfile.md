# Lockfile (`Bender.lock`)

The lockfile, named `Bender.lock`, is a machine-generated file that records the exact version and Git revision (commit hash) of every dependency in your project's tree.

## Why a Lockfile?

While `Bender.yml` specifies your *intent* (e.g., "I need `common_cells` version 1.21.x"), the lockfile specifies the *reality* (e.g., "`common_cells` is version 1.21.5 at commit `a1b2c3d`").

The lockfile ensures:
- **Reproducible Builds:** Everyone on your team is working with the exact same code.
- **CI Stability:** Your CI pipeline won't suddenly fail because a dependency released a new (but incompatible) version.
- **Speed:** Bender doesn't need to re-resolve the entire dependency tree if the lockfile is already present.

## How it Works

The lockfile is managed by two primary commands:

- **[`bender update`](./workflow/dependencies.md#updating-dependencies):** Scans manifests, resolves constraints, and **updates** the `Bender.lock`.
- **[`bender checkout`](./workflow/dependencies.md#checking-out-dependencies):** Reads the `Bender.lock` and ensures the local state matches the exact recorded revisions.

## Structure of the Lockfile

The lockfile is written in YAML. For each package, it stores:

```yaml
packages:
  common_cells:
    revision: 290c010c26569ec18683510e1688536f98768000
    version: 1.21.0
    source:
      git: "https://github.com/pulp-platform/common_cells.git"
    dependencies:
      - tech_cells_generic
```

- **revision:** The full 40-character Git commit hash.
- **version:** The SemVer version that was resolved.
- **source:** Where to download the package from.
- **dependencies:** A list of other packages that this specific package depends on, ensuring the entire tree is captured.

## Best Practices

- **Commit it:** Always check `Bender.lock` into your version control (Git).
- **Update with intention:** Only run `bender update` when you actually want to pull in newer versions of your dependencies.
- **Review changes:** When `Bender.lock` changes, review the diff to see exactly which packages were upgraded.

## Frozen Manifests

If you want to prevent accidental updates to your project's dependency tree, you can set `frozen: true` in your `Bender.yml`. 

```yaml
package:
  name: my_chip
  frozen: true # Prevents 'bender update' from running
```

When `frozen: true` is set, `bender update` will fail, ensuring that your `Bender.lock` remains unchanged until you explicitly unfreeze the manifest. This is mostly recommended for late-stage tapeouts.

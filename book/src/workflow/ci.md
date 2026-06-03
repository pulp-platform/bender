# Continuous Integration

Integrating Bender into your CI/CD pipeline ensures that your hardware project is consistently built, linted, and simulated using the exact same dependency versions as your local environment.

## Reproducibility

In a CI environment, you should never run `bender update`. Instead, your pipeline should rely on the [`Bender.lock`](../lockfile.md) file to fetch the exact revisions of your dependencies.

1.  **Commit** [`Bender.lock`](../lockfile.md): Ensure the lockfile is checked into your repository.
2.  **Use** `bender checkout`: This command reads the lockfile and reconstructs the dependency tree without changing any versions.

## GitHub Actions

If you are using GitHub Actions, the [`pulp-platform/pulp-actions`](https://github.com/pulp-platform/pulp-actions) repository provides a dedicated action to simplify the setup.

### `bender-install`

The `bender-install` action handles downloading the Bender binary and adding it to your `PATH`.

```yaml
steps:
  - uses: actions/checkout@v4
  
  - name: Install Bender
    uses: pulp-platform/pulp-actions/bender-install@v2
    with:
      version: 0.31.0 # Optional: specify a version

  - name: Checkout dependencies
    run: bender checkout
```

## GitLab CI

For GitLab CI, you can use the standard [installation script](../installation.md) to fetch the Bender binary at the start of your job if bender is not already installed in the system.

### Example Workflow

```yaml
variables:
  BENDER_VERSION: "0.31.0"

before_script:
  # Install Bender locally so the binary lands in the current directory across
  # all versions, then add CWD to PATH. (For v0.32.0+ the installer would
  # otherwise default to a global install; --local pins the project-local
  # behavior. For pre-v0.32.0 versions --local is a no-op.)
  - curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh -s -- --local $BENDER_VERSION
  - export PATH=$PATH:$(pwd)

sim_job:
  stage: test
  script:
    - bender checkout
    - bender script vsim > compile.tcl
    - # Run your simulation tool here...
```

## Caching

Since [`Bender.lock`](../lockfile.md) uniquely identifies the state of all dependencies, it is theoretically possible to cache the `.bender` directory to speed up pipelines. However, for most projects, the overhead of managing the cache (uploading/downloading) might outweigh the time saved by `bender checkout`, especially with fast network connections to Git remotes.

Caching is only recommended for projects with exceptionally large dependency trees or slow network access.

### Caching Examples

#### GitHub Actions
```yaml
  - name: Cache Bender dependencies
    uses: actions/cache@v4
    with:
      path: .bender
      key: bender-${{ hashFiles('**/Bender.lock') }}
```

#### GitLab CI
```yaml
cache:
  key:
    files:
      - Bender.lock
  paths:
    - .bender/
```

> **Note on Cache Keys:** We use the hash of [`Bender.lock`](../lockfile.md) as the cache key. This ensures the cache is only reused when the dependencies haven't changed.

## Sharing the Database Across Runs and Projects

When jobs run on a persistent runner, the bare git repositories Bender clones can be shared across CI runs and even across projects to drastically reduce fetch times and disk usage. The recommended way is the [`db_dir`](../configuration.md#db_dir) setting, which relocates *only* the bare repos and lock files; per-project working-tree checkouts stay under each project's own `.bender/` directory and cannot collide.

Place a [`Bender.local`](../local.md) in a parent directory of your projects so it is picked up automatically:

```yaml
db_dir: /var/cache/bender_shared
```

Every job on the runner now reuses the already-fetched Git data and serializes safely against concurrent jobs via per-dependency filesystem locks living next to the bare repos (`<db_dir>/git/locks/`).

If you'd rather not place a `Bender.local` on the runner at all, exporting `BENDER_DB_DIR=/var/cache/bender_shared` in the job environment has the same effect — any project that explicitly sets `db_dir` in its own configuration overrides the env var.

> **Bender versions before 0.32** silently ignore both `db_dir` and `BENDER_DB_DIR` and fall back to their normal per-project `.bender/` cache, so the shared config above is safe to deploy on a runner that still hosts pinned-to-old-bender projects — those projects simply won't benefit from the shared cache, but they won't misbehave either.

### Sharing working-tree checkouts too

If you want to share the bare repos *and* the per-dependency working-tree checkouts (uncommon), use [`database`](../configuration.md#database) instead of `db_dir`:

```yaml
database: /var/cache/bender_shared
```

Caveat: two top-level projects with the same name will collide on the same `<database>/git/checkouts/<name>-<hash>/` directory. Give each top-level project its own `workspace.checkout_dir` in [`Bender.yml`](../manifest.md) so the checkouts remain isolated.

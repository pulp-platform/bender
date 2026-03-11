# Continuous Integration

Integrating Bender into your CI/CD pipeline ensures that your hardware project is consistently built, linted, and simulated using the exact same dependency versions as your local environment.

## Reproducibility

In a CI environment, you should never run `bender update`. Instead, your pipeline should rely on the `Bender.lock` file to fetch the exact revisions of your dependencies.

1.  **Commit** `Bender.lock`: Ensure the lockfile is checked into your repository.
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

For GitLab CI, you can use the standard [installation script](../installation.md) to fetch the Bender binary at the start of your job.

### Example Workflow

```yaml
variables:
  BENDER_VERSION: "0.31.0"

before_script:
  # Install Bender using the init script
  - curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh -s -- $BENDER_VERSION
  - export PATH=$PATH:$(pwd)

sim_job:
  stage: test
  script:
    - bender checkout
    - bender script vsim > compile.tcl
    - # Run your simulation tool here...
```

## Caching

Since `Bender.lock` uniquely identifies the state of all dependencies, it is theoretically possible to cache the `.bender` directory to speed up pipelines. However, for most projects, the overhead of managing the cache (uploading/downloading) might outweigh the time saved by `bender checkout`, especially with fast network connections to Git remotes.

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

> **Note on Cache Keys:** We use the hash of `Bender.lock` as the cache key. This ensures the cache is only reused when the dependencies haven't changed.

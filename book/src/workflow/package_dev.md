# Package Development

Bender makes it easy to develop multiple packages simultaneously. If you find yourself needing to modify a dependency, you don't have to manually manage local paths and Git remotes. Instead, you can use the `clone` and `snapshot` workflow.

## The Development Workflow

### 1. Clone the Dependency
Use the `clone` command to move a dependency from Bender's internal cache into a local directory where you can modify it:

```sh
bender clone <PKG_NAME>
```

By default, the package is checked out into a `working_dir` folder (you can change this with `-p/--path`). Bender automatically:
1.  Performs a `git clone` of the dependency into that folder.
2.  Adds a `path` override to your **`Bender.local`** file.

Now, any changes you make in that folder are immediately reflected in your top-level project when you run Bender commands.

### 2. Modify and Commit
You can now work on the cloned package as if it were a normal Git repository. You can add files, fix bugs, and commit your changes within that directory.

### 3. Snapshot the State
Once you have committed changes in your cloned dependency and want to record that specific state (for sharing with others or for CI), use the `snapshot` command:

```sh
bender snapshot
```

Bender will:
1.  Detect all dependencies that are currently overridden by a local path.
2.  Check the current Git commit hash of those local repositories.
3.  Update **`Bender.local`** to use a `git` override with that specific `rev` (commit hash).
4.  Automatically update **`Bender.lock`** to include these exact revisions.

**Why use Snapshot?**
The main benefit of a snapshot is portability. Because the lockfile is updated with the specific commit hashes, you can commit `Bender.lock` and share it with colleagues or run it in CI. The other environments will download the exact revisions you were working on from the Git remotes, without needing access to your local development paths.

## Finalizing Changes

Once your changes are stable and you are ready to "release" them:

1.  **Tag the Dependency:** Push your changes to the remote repository and create a new version tag (e.g., `v1.2.2`).
2.  **Update Manifest:** Update the version requirement in your top-level `Bender.yml` to include the new version.
3.  **Clean Up:** Remove the local overrides from your `Bender.local` file.
4.  **Resolve:** Run `bender update` to re-resolve the dependency tree and update `Bender.lock` to point to the new official version.

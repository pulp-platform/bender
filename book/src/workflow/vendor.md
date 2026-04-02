# Vendorizing External Code

Bender's `vendor` command allows you to manage external dependencies that aren't natively packaged for Bender. It works by copying a subset of files from an upstream Git repository into a local directory within your project, allowing you to track changes and maintain local patches.

This flow is heavily inspired by the `vendor.py` script used in the [OpenTitan](https://opentitan.org/) project.

## Configuration

Vendorized packages are defined in the `vendor_package` section of your `Bender.yml`:

```yaml
vendor_package:
  - name: my_ip
    target_dir: "deps/my_ip"
    upstream: { git: "https://github.com/external/my_ip.git", rev: "abcd123" }
    include_from_upstream:
      - "src/*.sv"
      - "include/*.svh"
    exclude_from_upstream:
      - "src/deprecated/*"
    patch_dir: "deps/patches/my_ip"
    mapping:
      - {from: 'src/old_name.sv', to: 'src/new_name.sv' }
```

### Key Fields
- **`name`**: A unique identifier for the vendorized package.
- **`target_dir`**: Where the files should be copied to in your repository.
- **`upstream`**: The Git repository and specific revision (commit/tag/branch) to pull from.
- **`include_from_upstream`**: (Optional) Glob patterns of files to copy.
- **`exclude_from_upstream`**: (Optional) Glob patterns of files to ignore.
- **`patch_dir`**: (Optional) A directory containing `.patch` files to apply after copying.
- **`mapping`**: (Optional) A list of specific file renames or movements during the copy process.

## The Vendor Workflow

Using the configuration above, here is how you manage a vendorized IP:

### 1. Initialize (`init`)
To download the upstream code and copy it into your `target_dir`:

```sh
bender vendor init
```

Bender clones the upstream repository, filters the files based on your rules, and copies them to your project. If a `patch_dir` is specified, any existing patches are applied automatically.

### 2. Make Local Changes and Diff
Assume you need to fix a bug in `deps/my_ip/src/top.sv`. You edit the file directly in your workspace. You can see how your local code differs from the upstream source (plus existing patches) by running:

```sh
bender vendor diff
```

### 3. Create a Patch (`patch`)
To make your fix permanent and shareable, stage the change and generate a patch:

```sh
# Stage the change in your main repository
git add deps/my_ip/src/top.sv

# Generate the patch file
bender vendor patch
```

Bender will prompt for a commit message and create a numbered patch file in your `patch_dir` (e.g., `0001-fix-bug.patch`).

### 4. Commit Everything
Now you can create an atomic commit in your repository that contains both the modified source and the new patch file:

```sh
git add deps/patches/my_ip/0001-fix-bug.patch
git commit -m "Update my_ip with local bugfix"
```

The next time you (or a teammate) run `bender vendor init`, Bender will pull the fresh upstream code and automatically apply your patch.

## Upstreaming Patches

If you want to contribute your fix back to the upstream repository:
1. Clone the upstream repository separately.
2. Check out the same revision (`abcd123`).
3. Apply your patch: `git am /path/to/your_repo/deps/patches/my_ip/0001-fix-bug.patch`.
4. The fix is now a proper Git commit in the upstream repo, with all metadata (author, timestamp) preserved.

## When to use Vendor?

Use the vendor flow when:
- The external IP does not have its own `Bender.yml`.
- You need to include only a small subset of a massive repository.
- You must maintain local modifications (bug fixes or tool workarounds) that haven't been merged upstream yet.

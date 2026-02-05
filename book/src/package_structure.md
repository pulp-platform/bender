# Package Structure

Bender looks for the following three files in a package:

- `Bender.yml`: This is the main **package manifest**, and the only required file for a directory to be recognized as a Bender package. It contains metadata, dependencies, and source file lists.

- `Bender.lock`: The **lock file** is generated once all dependencies have been successfully resolved. It contains the exact revision of each dependency. This file *may* be put under version control to allow for reproducible builds. This is handy for example upon taping out a design. If the lock file is missing or a new dependency has been added, it is regenerated.

- `Bender.local`: This optional file contains **local configuration overrides**. It should be ignored in version control, i.e. added to `.gitignore`. This file can be used to override dependencies with local variants. It is also used when the user asks for a local working copy of a dependency.

[Relevant code](https://github.com/pulp-platform/bender/blob/master/src/cli.rs)

# Initialization

To start a new Bender project, use the `init` command to set up a basic manifest:

```sh
bender init
```

## How it Works

Bender attempts to intelligently pre-fill your `Bender.yml` with the following information:

- **Package Name:** Set to the name of the current working directory.
- **Authors:** Pulled automatically from your global Git configuration (`git config user.name` and `git config user.email`).

## The Generated Manifest

The command creates a `Bender.yml` file with a structure similar to this:

```yaml
package:
  name: my_new_ip
  authors:
    - "John Doe <john@doe.com>"

dependencies:

sources:
  # Source files grouped in levels. 
  # Level 0
```

Once the manifest is created, you can begin adding your [dependencies](../dependencies.md) and [source files](../sources.md) to the respective sections.

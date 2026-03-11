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

# Workflow

The workflow of bender is based on a configuration and a lock file. The configuration file lists the sources, dependencies, and tests of the package at hand. The lock file is used by the tool to track which exact version of a package is being used. Adding this file to version control, e.g. for chips that will be taped out, makes it easy to reconstruct the exact IPs that were used during a simulation, synthesis, or tapeout.

Upon executing any command, bender checks to see if dependencies have been added to the configuration file that are not in the lock file. It then tries to find a revision for each added dependency that is compatible with the other dependencies and add that to the lock file. In a second step, bender tries to ensure that the checked out revisions match the ones in the lock file. If not possible, appropriate errors are generated.

The update command reevaluates all dependencies in the configuration file and tries to find for each a revision that satisfies all recursive constraints. If semantic versioning is used, this will update the dependencies to newer versions within the bounds of the version requirement provided in the configuration file.

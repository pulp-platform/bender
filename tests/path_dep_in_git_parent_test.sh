#!/bin/bash
# Regression test for parent-relative path dependencies.
#
# A path dependency declared inside a git dependency must be recorded in the
# lockfile relative to its parent (with a `parent:` field) rather than as a
# baked-in absolute path, and its on-disk location must be derived from the
# parent's checkout. Nested path-in-path-in-git dependencies must accumulate the
# relative path while keeping the git dependency as their parent.

set -e

cd "$(dirname "${BASH_SOURCE[0]}")"
BENDER=${BENDER:-"cargo run --"}
DIR="$(pwd)"/tmp/"$(basename "$0" _test.sh)"
[ ! -d "$DIR" ] || rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

git config --global init.defaultBranch master >/dev/null 2>&1 || true

# --- Build a git dependency "gitdep" that contains a path dependency "mid",
#     which itself contains a nested path dependency "leaf". ---
mkdir -p gitdep/src gitdep/libs/mid/deep/leaf/src
cat > gitdep/Bender.yml <<EOF
package:
  name: gitdep
dependencies:
  mid: { path: "libs/mid" }
sources:
  - src/gitdep.sv
EOF
echo "module gitdep; endmodule" > gitdep/src/gitdep.sv
cat > gitdep/libs/mid/Bender.yml <<EOF
package:
  name: mid
dependencies:
  leaf: { path: "deep/leaf" }
sources:
  - mid.sv
EOF
echo "module mid; endmodule" > gitdep/libs/mid/mid.sv
cat > gitdep/libs/mid/deep/leaf/Bender.yml <<EOF
package:
  name: leaf
sources:
  - src/leaf.sv
EOF
echo "module leaf; endmodule" > gitdep/libs/mid/deep/leaf/src/leaf.sv

cd "$DIR"/gitdep
git init -q
git add -A
git -c user.name=test -c user.email=test@test commit -q -m "init"
REV=$(git rev-parse HEAD)

# --- Root package depending on gitdep via git. ---
mkdir -p "$DIR"/root
cd "$DIR"/root
cat > Bender.yml <<EOF
package:
  name: root
dependencies:
  gitdep: { git: "file://$DIR/gitdep", rev: "$REV" }
EOF

$BENDER update &> log || { cat log; echo "update failed" >&2; exit 1; }

# The lockfile must record the path deps relative to the parent, not absolutely.
if ! grep -qE '^      Path: libs/mid$' Bender.lock; then
	cat Bender.lock
	echo "expected 'mid' to be locked relative to parent (libs/mid)" >&2
	exit 2
fi
# 'leaf' is nested inside 'mid', so its parent is the immediate dependency
# 'mid' and its path is relative to 'mid' (not the git root).
if ! grep -qE '^      Path: deep/leaf$' Bender.lock; then
	cat Bender.lock
	echo "expected nested 'leaf' to be relative to its parent 'mid' (deep/leaf)" >&2
	exit 3
fi
if ! grep -qE '^    parent: gitdep$' Bender.lock; then
	cat Bender.lock
	echo "expected 'mid' to have parent 'gitdep'" >&2
	exit 4
fi
if ! grep -qE '^    parent: mid$' Bender.lock; then
	cat Bender.lock
	echo "expected 'leaf' to have parent 'mid'" >&2
	exit 5
fi

# The derived paths must point inside the parent's checkout and exist.
GITDEP_PATH=$($BENDER path gitdep)
LEAF_PATH=$($BENDER path leaf)
case "$LEAF_PATH" in
	"$GITDEP_PATH"/libs/mid/deep/leaf) ;;
	*)
		echo "leaf path '$LEAF_PATH' is not nested under gitdep checkout '$GITDEP_PATH'" >&2
		exit 6
		;;
esac
if [ ! -f "$LEAF_PATH/src/leaf.sv" ]; then
	echo "leaf source not found at derived path '$LEAF_PATH'" >&2
	exit 7
fi

# The lockfile must be portable: copying the project tree elsewhere and
# resolving from the lock alone must derive paths under the new location, i.e.
# no absolute path was baked into the lock.
cp -r "$DIR"/root "$DIR"/moved
cd "$DIR"/moved
rm -rf .bender
MOVED_LEAF=$($BENDER path leaf)
case "$MOVED_LEAF" in
	"$DIR"/moved/*) ;;
	*)
		echo "moved leaf path '$MOVED_LEAF' is not under the moved project; absolute path was baked in" >&2
		exit 8
		;;
esac

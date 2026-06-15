#!/bin/bash
# Regression test for path dependencies nested inside another path dependency.
#
# A relative path dependency declared inside another path dependency must be
# recorded relative to that parent (with a `parent:` field naming it) rather
# than relative to the root package, and `bender path` must still report the
# correct absolute location. This keeps the lockfile portable even when the
# parent path dependency lives outside the root project tree.

set -e

cd "$(dirname "${BASH_SOURCE[0]}")"
BENDER=${BENDER:-"cargo run --"}
DIR="$(pwd)"/tmp/"$(basename "$0" _test.sh)"
[ ! -d "$DIR" ] || rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

# A path dependency "mid" that lives OUTSIDE the root tree and itself contains a
# nested path dependency "leaf".
mkdir -p ext/mid/deep/leaf/src
cat > ext/mid/Bender.yml <<EOF
package:
  name: mid
dependencies:
  leaf: { path: "deep/leaf" }
sources:
  - mid.sv
EOF
echo "module mid; endmodule" > ext/mid/mid.sv
cat > ext/mid/deep/leaf/Bender.yml <<EOF
package:
  name: leaf
sources:
  - src/leaf.sv
EOF
echo "module leaf; endmodule" > ext/mid/deep/leaf/src/leaf.sv

# The root package depends on "mid" via a relative path that escapes the root.
mkdir -p "$DIR"/root
cd "$DIR"/root
cat > Bender.yml <<EOF
package:
  name: root
dependencies:
  mid: { path: "../ext/mid" }
EOF

$BENDER update &> log || { cat log; echo "update failed" >&2; exit 1; }

# 'leaf' must be locked relative to its parent 'mid', with a parent field.
if ! grep -qE '^      Path: deep/leaf$' Bender.lock; then
	cat Bender.lock
	echo "expected 'leaf' to be locked relative to its parent 'mid' (deep/leaf)" >&2
	exit 2
fi
if ! grep -qE '^    parent: mid$' Bender.lock; then
	cat Bender.lock
	echo "expected 'leaf' to have parent 'mid'" >&2
	exit 3
fi

# 'bender path' must report the correct absolute location for both deps.
MID_PATH=$($BENDER path mid)
LEAF_PATH=$($BENDER path leaf)
case "$LEAF_PATH" in
	"$MID_PATH"/deep/leaf) ;;
	*)
		echo "leaf path '$LEAF_PATH' is not nested under mid '$MID_PATH'" >&2
		exit 4
		;;
esac
if [ ! -f "$LEAF_PATH/src/leaf.sv" ]; then
	echo "leaf source not found at derived path '$LEAF_PATH'" >&2
	exit 5
fi

# Portability: moving the whole tree (root + ext together) and resolving from
# the lock alone must still derive 'leaf' under the moved 'mid', proving the
# path was recorded relative to its parent rather than as an absolute path.
MOVED="$DIR"_moved
cd "$(dirname "$DIR")"
mv "$DIR" "$MOVED"
cd "$MOVED"/root
rm -rf .bender Bender.lock
$BENDER update &> log || { cat log; echo "update after move failed" >&2; exit 6; }
MOVED_MID=$($BENDER path mid)
MOVED_LEAF=$($BENDER path leaf)
case "$MOVED_LEAF" in
	"$MOVED_MID"/deep/leaf) ;;
	*)
		echo "moved leaf '$MOVED_LEAF' not under moved mid '$MOVED_MID'" >&2
		exit 7
		;;
esac
case "$MOVED_MID" in
	"$MOVED"/*) ;;
	*)
		echo "moved mid '$MOVED_MID' not under moved tree '$MOVED'; absolute path baked in" >&2
		exit 8
		;;
esac

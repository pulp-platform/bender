#!/bin/bash
# Test re-exporting include directories of dependencies via `export_include_dirs`.
set -e

cd "$(dirname "${BASH_SOURCE[0]}")"
BENDER=${BENDER:-"cargo run --"}
DIR="$(pwd)"/tmp/"$(basename $0 _test.sh)"
[ ! -d "$DIR" ] || rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

fail() {
	echo "$1" >&2
	exit 1
}

# c exports `inc` and `other`; b re-exports `inc` of c; a re-exports all of b.
mkdir -p c/inc c/other c/sub/inc c/src b/inc b/src a/src top/src
touch c/src/c.sv b/src/b.sv a/src/a.sv top/src/top.sv
cat >c/Bender.yml <<YML
package:
  name: c
export_include_dirs:
  - inc
  - other
  - ./sub/inc/
sources:
  - src/c.sv
YML
cat >b/Bender.yml <<YML
package:
  name: b
dependencies:
  c: { path: "../c" }
export_include_dirs:
  - inc
  - dep: c
    dir: inc
sources:
  - src/b.sv
YML
cat >a/Bender.yml <<YML
package:
  name: a
dependencies:
  b: { path: "../b" }
export_include_dirs:
  - dep: b
sources:
  - src/a.sv
YML
cat >top/Bender.yml <<YML
package:
  name: top
dependencies:
  a: { path: "../a" }
sources:
  - src/top.sv
YML
cd top

# Include directories passed to the compile command of the given source file.
incdirs_of() {
	$BENDER script vsim "${@:2}" | awk -v RS='vlog ' -v f="$1" 'index($0, f)' | grep incdir || true
}

# The root sees the include directories of a, b and (through b) c, but not
# the `other` directory of c, which b does not re-export.
out=$(incdirs_of src/top.sv)
for d in b/inc c/inc; do
	echo "$out" | grep -q "+incdir+.*/$d\"" || fail "missing $d in include directories"
done
echo "$out" | grep -q "c/other" && fail "c/other must not be re-exported"
# `dir: inc` is an exact match and does not pick up `sub/inc`.
echo "$out" | grep -q "sub/inc" && fail "c/sub/inc must not be re-exported"

# A nested directory can be selected, also with a different spelling.
cp ../b/Bender.yml ../b/Bender.yml.orig
sed -i.bak 's|dir: inc|dir: ./sub/inc|' ../b/Bender.yml
out=$(incdirs_of src/top.sv)
echo "$out" | grep -q "+incdir+.*/c/\./sub/inc/\"" || fail "missing c/sub/inc"
echo "$out" | grep -q "c/inc" && fail "c/inc must not be re-exported"
cp ../b/Bender.yml.orig ../b/Bender.yml

# Only exact matches count: `nc/inc` or `..` do not select anything.
for d in 'sub' '../c/inc' '/inc'; do
	sed -i.bak "s|dir: inc|dir: $d|" ../b/Bender.yml
	if $BENDER script vsim &>log; then fail "dir $d should fail"; fi
	grep -q "does not match any of its" log || { cat log; fail "wrong error for dir $d"; }
	cp ../b/Bender.yml.orig ../b/Bender.yml
done

# An empty `dir` is rejected.
sed -i.bak 's|dir: inc|dir: ""|' ../b/Bender.yml
if $BENDER script vsim &>log; then fail "empty dir should fail"; fi
cp ../b/Bender.yml.orig ../b/Bender.yml

# Target filtering of a re-export is honored.
sed -i.bak 's|  - dep: c|  - dep: c\n    target: sim|' ../b/Bender.yml
incdirs_of src/top.sv | grep -q "c/inc" && fail "c/inc must be filtered without target sim"
incdirs_of src/top.sv -t sim | grep -q "+incdir+.*/c/inc\"" || fail "missing c/inc with target sim"
cp ../b/Bender.yml.orig ../b/Bender.yml

# A directory the dependency does not export is an error.
sed -i.bak 's|dir: inc|dir: nope|' ../b/Bender.yml
if $BENDER script vsim &>log; then fail "unknown dir should fail"; fi
grep -q "does not match any of its" log || { cat log; fail "wrong error for unknown dir"; }
cp ../b/Bender.yml.orig ../b/Bender.yml

# A package that is not a dependency is an error.
sed -i.bak 's|dep: c|dep: z|' ../b/Bender.yml
if $BENDER script vsim &>log; then fail "unknown dep should fail"; fi
grep -q "not one of its dependencies" log || { cat log; fail "wrong error for unknown dep"; }

# Diamond: top depends on b and e, which both re-export c; no duplicates.
cp ../b/Bender.yml.orig ../b/Bender.yml
mkdir -p ../e/src
touch ../e/src/e.sv
cat >../e/Bender.yml <<YML
package:
  name: e
dependencies:
  c: { path: "../c" }
export_include_dirs:
  - dep: c
    dir: inc
sources:
  - src/e.sv
YML
cat >Bender.yml <<YML
package:
  name: top
dependencies:
  b: { path: "../b" }
  e: { path: "../e" }
sources:
  - src/top.sv
YML
$BENDER update &>/dev/null
n=$(incdirs_of src/top.sv | grep -c "/c/inc\"")
[ "$n" = 1 ] || fail "c/inc listed $n times in diamond"

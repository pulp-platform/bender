#!/bin/bash
set -e

cd "$(dirname "${BASH_SOURCE[0]}")"
BENDER="cargo run --"
DIR="$(pwd)"/tmp/"$(basename $0 _test.sh)"
[ ! -d "$DIR" ] || rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

mkdir foo
mkdir bar

cd "$DIR"/foo
git init

cd "$DIR"/bar
echo "
package:
  name: bar

dependencies:
  foo: { git: \"file://$DIR/foo\", rev: master }
" > Bender.yml
if $BENDER path foo &> log; then # this fails according to issue #2
	cat log
	echo "should fail" >&2
	exit 1
fi

if ! grep 'Dependency `foo` cannot satisfy requirement `master`' log; then
	cat log
	echo "should fail differently" >&2
	exit 2
fi

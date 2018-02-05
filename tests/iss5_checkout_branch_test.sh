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
touch README
git add .
git commit -m "Hello"

cd "$DIR"/bar
echo "
package:
  name: bar

dependencies:
  foo: { git: \"file://$DIR/foo\", rev: master }
" > Bender.yml
$BENDER path foo # this fails according to issue #5

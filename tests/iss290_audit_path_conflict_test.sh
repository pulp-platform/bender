#!/bin/bash
# Regression test for issue #290:
# bender audit should not report false path conflicts when multiple packages
# reference the same local directory via different relative paths.
#
# Setup:
#   my-chip/
#   ├── Bender.yml        → common: path: "ip/common"
#   ├── ip/
#   │   ├── common/
#   │   │   └── Bender.yml
#   │   ├── foo/
#   │   │   └── Bender.yml → common: path: "../common"
#   │   └── bar/
#   │       └── Bender.yml → common: path: "../common"
#
# All three references resolve to the same ip/common/ directory.
# Before fix: bender audit reports "common has a Conflict"
# After fix:  no conflict reported

set -e

cd "$(dirname "${BASH_SOURCE[0]}")"
BENDER=${BENDER:-"cargo run --"}
DIR="$(pwd)"/tmp/"$(basename $0 _test.sh)"
[ ! -d "$DIR" ] || rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

# Create directory structure
mkdir -p ip/common ip/foo ip/bar

# Root Bender.yml - references common via "ip/common"
cat > Bender.yml <<'EOF'
package:
  name: my-chip

dependencies:
  common: { path: "ip/common" }
  foo:    { path: "ip/foo" }
  bar:    { path: "ip/bar" }

sources:
  - target: rtl
    files: []
EOF

# ip/common/Bender.yml - the shared dependency
cat > ip/common/Bender.yml <<'EOF'
package:
  name: common

sources:
  - target: rtl
    files: []
EOF

# ip/foo/Bender.yml - references common via "../common" (relative to foo)
cat > ip/foo/Bender.yml <<'EOF'
package:
  name: foo

dependencies:
  common: { path: "../common" }

sources:
  - target: rtl
    files: []
EOF

# ip/bar/Bender.yml - references common via "../common" (relative to bar)
cat > ip/bar/Bender.yml <<'EOF'
package:
  name: bar

dependencies:
  common: { path: "../common" }

sources:
  - target: rtl
    files: []
EOF

# Run bender audit and capture output
if ! $BENDER audit > log 2>&1; then
	# bender audit may return non-zero on conflicts, check output
	true
fi

# The key assertion: "Conflict" should NOT appear for common
if grep -q "Conflict" log; then
	echo "FAIL: bender audit falsely reports a path conflict:" >&2
	cat log >&2
	echo "" >&2
	echo "All three paths (ip/common, ../common, ../common) point to the same directory." >&2
	echo "This is a false positive. See https://github.com/pulp-platform/bender/issues/290" >&2
	exit 1
fi

echo "PASS: No false path conflict detected for equivalent relative paths."

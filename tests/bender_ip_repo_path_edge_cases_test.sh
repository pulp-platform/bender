#!/bin/bash
# Test: BENDER_IP_REPO_PATH edge cases
# Tests various edge cases and error conditions

set -e

# Create a temporary directory for testing
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

cleanup() {
    cd /
    rm -rf "$TEST_DIR"
    unset BENDER_IP_REPO_PATH
}
trap cleanup EXIT

# Test 1: Malformed environment variable (multiple colons, empty components)
export BENDER_IP_REPO_PATH=":/non/existent::/another/path:"

mkdir -p edge_test1/src
cat > edge_test1/Bender.yml << 'EOF'
package:
  name: edge_test1
  authors: ["Test <test@example.com>"]
sources:
  - src/test.sv
EOF

cat > edge_test1/src/test.sv << 'EOF'
module edge_test1;
endmodule
EOF

cd edge_test1
# Should handle malformed path gracefully
/workspaces/bender/target/debug/bender packages > /dev/null
cd ..

# Test 2: Conflicting dependency names (same name in multiple search paths)
mkdir -p conflict_test/{path1/shared_dep,path2/shared_dep,main}/src

# First version
cat > conflict_test/path1/shared_dep/Bender.yml << 'EOF'
package:
  name: shared_dep
  authors: ["Test <test@example.com>"]
sources:
  - src/version1.sv
EOF

cat > conflict_test/path1/shared_dep/src/version1.sv << 'EOF'
module shared_dep_v1;
  // Version 1
endmodule
EOF

# Second version (same name, different content)
cat > conflict_test/path2/shared_dep/Bender.yml << 'EOF'
package:
  name: shared_dep
  authors: ["Test <test@example.com>"]
sources:
  - src/version2.sv
EOF

cat > conflict_test/path2/shared_dep/src/version2.sv << 'EOF'
module shared_dep_v2;
  // Version 2
endmodule
EOF

# Main package
cat > conflict_test/main/Bender.yml << 'EOF'
package:
  name: main_conflict_test
  authors: ["Test <test@example.com>"]
dependencies:
  shared_dep: { git: "https://fake.url/shared_dep.git", version: "1.0.0" }
sources:
  - src/main.sv
EOF

cat > conflict_test/main/src/main.sv << 'EOF'
module main_conflict;
endmodule
EOF

export BENDER_IP_REPO_PATH="$TEST_DIR/conflict_test/path1:$TEST_DIR/conflict_test/path2"

cd conflict_test/main
# Should use the first one found and not fail
OUTPUT=$(/workspaces/bender/target/debug/bender packages 2>&1)
if ! echo "$OUTPUT" | grep -q "shared_dep"; then
    echo "Error: shared_dep not found in conflicting dependencies test"
    exit 1
fi
cd ../..

# Test 3: Fallback to Git when IP not found in search path
mkdir -p fallback_test/main_pkg/src

cat > fallback_test/main_pkg/Bender.yml << 'EOF'
package:
  name: fallback_test_pkg
  authors: ["Test Author <test@example.com>"]

dependencies:
  non_existent_ip: { git: "https://fake.git.url/non_existent.git", version: "1.0.0" }

sources:
  - src/main.sv
EOF

cat > fallback_test/main_pkg/src/main.sv << 'EOF'
module fallback_main;
  // Fallback test module
endmodule
EOF

export BENDER_IP_REPO_PATH="$TEST_DIR/conflict_test/path1"  # Path doesn't contain non_existent_ip

cd fallback_test/main_pkg

# This should attempt Git and fail (expected behavior)
if /workspaces/bender/target/debug/bender packages > /dev/null 2>&1; then
    echo "Error: Should have failed trying to fetch from fake Git URL"
    exit 1
fi

cd ../..

# Test 4: Special characters in paths
export BENDER_IP_REPO_PATH="/path with spaces:/path-with-dashes:/path_with_underscores"

mkdir -p special_test/src
cat > special_test/Bender.yml << 'EOF'
package:
  name: special_test
  authors: ["Test <test@example.com>"]
sources:
  - src/test.sv
EOF

cat > special_test/src/test.sv << 'EOF'
module special_test;
endmodule
EOF

cd special_test
# Should handle special characters gracefully
/workspaces/bender/target/debug/bender packages > /dev/null
cd ..

# Test 5: Very long path string
LONG_PATH=""
for i in {1..20}; do  # Reduced from 50 to 20 for faster testing
    LONG_PATH="${LONG_PATH}:/very/long/path/component/number/$i"
done
export BENDER_IP_REPO_PATH="$LONG_PATH"

mkdir -p long_path_test/src
cat > long_path_test/Bender.yml << 'EOF'
package:
  name: long_path_test
  authors: ["Test <test@example.com>"]
sources:
  - src/test.sv
EOF

cat > long_path_test/src/test.sv << 'EOF'
module long_path_test;
endmodule
EOF

cd long_path_test
# Should handle long path string gracefully
/workspaces/bender/target/debug/bender packages > /dev/null
cd ..

echo "All edge case tests passed"

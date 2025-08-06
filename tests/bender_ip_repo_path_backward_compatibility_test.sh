#!/bin/bash
# Test: BENDER_IP_REPO_PATH backward compatibility
# Ensures the new feature doesn't break existing functionality

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

# Test 1: Basic functionality without BENDER_IP_REPO_PATH
unset BENDER_IP_REPO_PATH

# Create a simple test package
mkdir -p test_pkg/src
cat > test_pkg/Bender.yml << 'EOF'
package:
  name: test_pkg
  authors: ["Test Author <test@example.com>"]

dependencies: {}

sources:
  - src/test.sv
EOF

cat > test_pkg/src/test.sv << 'EOF'
module test_module;
  // Simple test module
endmodule
EOF

cd test_pkg

# Test basic commands work
/workspaces/bender/target/debug/bender packages > /dev/null
/workspaces/bender/target/debug/bender sources > /dev/null
/workspaces/bender/target/debug/bender config > /dev/null

cd ..

# Test 2: Functionality with empty BENDER_IP_REPO_PATH
export BENDER_IP_REPO_PATH=""

cd test_pkg
/workspaces/bender/target/debug/bender packages > /dev/null
/workspaces/bender/target/debug/bender sources > /dev/null
cd ..

# Test 3: Functionality with non-existent paths in BENDER_IP_REPO_PATH
export BENDER_IP_REPO_PATH="/non/existent/path1:/another/fake/path"

cd test_pkg
/workspaces/bender/target/debug/bender packages > /dev/null
/workspaces/bender/target/debug/bender sources > /dev/null
cd ..

# Test 4: Path dependencies still work normally
mkdir -p path_dep_test/{main_pkg,dep_pkg}/src

cat > path_dep_test/dep_pkg/Bender.yml << 'EOF'
package:
  name: dep_pkg
  authors: ["Test Author <test@example.com>"]

sources:
  - src/dep.sv
EOF

cat > path_dep_test/dep_pkg/src/dep.sv << 'EOF'
module dep_module;
  // Dependency module
endmodule
EOF

cat > path_dep_test/main_pkg/Bender.yml << 'EOF'
package:
  name: main_pkg
  authors: ["Test Author <test@example.com>"]

dependencies:
  dep_pkg: { path: "../dep_pkg" }

sources:
  - src/main.sv
EOF

cat > path_dep_test/main_pkg/src/main.sv << 'EOF'
module main_module;
  // Main module
endmodule
EOF

cd path_dep_test/main_pkg
/workspaces/bender/target/debug/bender packages > /dev/null
/workspaces/bender/target/debug/bender sources > /dev/null
cd ../..

echo "All backward compatibility tests passed"

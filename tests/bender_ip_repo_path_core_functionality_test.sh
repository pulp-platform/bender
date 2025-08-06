#!/bin/bash
# Test: BENDER_IP_REPO_PATH core functionality
# Tests the main search path override functionality

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

# Test 1: Standard layout - path/ip_name/Bender.yml
mkdir -p repo_test/repo_path/test_ip/src
mkdir -p repo_test/main_pkg/src

# Create an IP in the repo path with standard layout
cat > repo_test/repo_path/test_ip/Bender.yml << 'EOF'
package:
  name: test_ip
  authors: ["Test Author <test@example.com>"]

sources:
  - src/test_ip.sv
EOF

cat > repo_test/repo_path/test_ip/src/test_ip.sv << 'EOF'
module test_ip_module;
  // Test IP module found via search path
endmodule
EOF

# Create main package that depends on the test IP
cat > repo_test/main_pkg/Bender.yml << 'EOF'
package:
  name: main_pkg_with_test_ip
  authors: ["Test Author <test@example.com>"]

dependencies:
  test_ip: { git: "https://fake.git.url/test_ip.git", version: "1.0.0" }

sources:
  - src/main.sv
EOF

cat > repo_test/main_pkg/src/main.sv << 'EOF'
module main_with_test_ip;
  // Main module with test IP dependency
endmodule
EOF

export BENDER_IP_REPO_PATH="$TEST_DIR/repo_test/repo_path"

cd repo_test/main_pkg

# Verify the search path override works
OUTPUT=$(/workspaces/bender/target/debug/bender packages 2>&1)
if ! echo "$OUTPUT" | grep -q "test_ip"; then
    echo "Error: test_ip not found in packages output"
    exit 1
fi

if ! echo "$OUTPUT" | grep -q "path"; then
    echo "Error: Expected path dependency but got different type"
    exit 1
fi

# Verify sources command works
/workspaces/bender/target/debug/bender sources > /dev/null

cd ../..

# Test 2: Direct layout - path/Bender.yml
mkdir -p direct_test/direct_ip/src
mkdir -p direct_test/main_pkg/src

# Create an IP directly in the search path
cat > direct_test/direct_ip/Bender.yml << 'EOF'
package:
  name: direct_ip
  authors: ["Test Author <test@example.com>"]

sources:
  - src/direct.sv
EOF

cat > direct_test/direct_ip/src/direct.sv << 'EOF'
module direct_ip_module;
  // Direct IP module
endmodule
EOF

# Create main package that depends on the direct IP
cat > direct_test/main_pkg/Bender.yml << 'EOF'
package:
  name: main_pkg_with_direct_ip
  authors: ["Test Author <test@example.com>"]

dependencies:
  direct_ip: { git: "https://fake.git.url/direct_ip.git", version: "1.0.0" }

sources:
  - src/main.sv
EOF

cat > direct_test/main_pkg/src/main.sv << 'EOF'
module main_with_direct_ip;
  // Main module with direct IP dependency
endmodule
EOF

export BENDER_IP_REPO_PATH="$TEST_DIR/direct_test"

cd direct_test/main_pkg

# Verify the direct layout search works
OUTPUT=$(/workspaces/bender/target/debug/bender packages 2>&1)
if ! echo "$OUTPUT" | grep -q "direct_ip"; then
    echo "Error: direct_ip not found in packages output"
    exit 1
fi

# Verify sources command works
/workspaces/bender/target/debug/bender sources > /dev/null

cd ../..

# Test 3: Multiple search paths (colon-separated)
mkdir -p multi_test/{path1/ip1,path2/ip2,main}/src

# First IP in path1
cat > multi_test/path1/ip1/Bender.yml << 'EOF'
package:
  name: ip1
  authors: ["Test Author <test@example.com>"]

sources:
  - src/ip1.sv
EOF

cat > multi_test/path1/ip1/src/ip1.sv << 'EOF'
module ip1_module;
endmodule
EOF

# Second IP in path2
cat > multi_test/path2/ip2/Bender.yml << 'EOF'
package:
  name: ip2
  authors: ["Test Author <test@example.com>"]

sources:
  - src/ip2.sv
EOF

cat > multi_test/path2/ip2/src/ip2.sv << 'EOF'
module ip2_module;
endmodule
EOF

# Main package depending on both
cat > multi_test/main/Bender.yml << 'EOF'
package:
  name: main_multi
  authors: ["Test Author <test@example.com>"]

dependencies:
  ip1: { git: "https://fake.url/ip1.git", version: "1.0.0" }
  ip2: { git: "https://fake.url/ip2.git", version: "1.0.0" }

sources:
  - src/main.sv
EOF

cat > multi_test/main/src/main.sv << 'EOF'
module main_multi;
endmodule
EOF

export BENDER_IP_REPO_PATH="$TEST_DIR/multi_test/path1:$TEST_DIR/multi_test/path2"

cd multi_test/main

# Verify both IPs are found
OUTPUT=$(/workspaces/bender/target/debug/bender packages 2>&1)
if ! echo "$OUTPUT" | grep -q "ip1"; then
    echo "Error: ip1 not found in multi-path test"
    exit 1
fi

if ! echo "$OUTPUT" | grep -q "ip2"; then
    echo "Error: ip2 not found in multi-path test"
    exit 1
fi

# Verify sources command works
/workspaces/bender/target/debug/bender sources > /dev/null

cd ../..

echo "All core functionality tests passed"

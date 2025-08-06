#!/bin/bash
# Test: BENDER_IP_REPO_PATH comprehensive integration
# Comprehensive end-to-end test demonstrating the complete workflow

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

# Scenario: Hardware design team with local IP repository
echo "=== Test 1: Basic IP repository functionality ==="

mkdir -p company_ips/{axi_lib,uart_ip}/src
mkdir -p project/cpu_design/src

# Create AXI library IP
cat > company_ips/axi_lib/Bender.yml << 'EOF'
package:
  name: axi_lib
  authors: ["Company IP Team <ip@company.com>"]

sources:
  - src/axi_pkg.sv
EOF

cat > company_ips/axi_lib/src/axi_pkg.sv << 'EOF'
package axi_pkg;
  // AXI package definitions
endpackage
EOF

# Create UART IP that depends on AXI
cat > company_ips/uart_ip/Bender.yml << 'EOF'
package:
  name: uart_ip
  authors: ["Company IP Team <ip@company.com>"]

dependencies:
  axi_lib: { git: "https://company.git/axi_lib.git", version: "2.1.0" }

sources:
  - src/uart_core.sv
EOF

cat > company_ips/uart_ip/src/uart_core.sv << 'EOF'
module uart_core;
  // UART implementation
endmodule
EOF

# Create main CPU design project
cat > project/cpu_design/Bender.yml << 'EOF'
package:
  name: cpu_design
  authors: ["CPU Team <cpu@company.com>"]

dependencies:
  axi_lib: { git: "https://company.git/axi_lib.git", version: "2.1.0" }
  uart_ip: { git: "https://company.git/uart_ip.git", version: "1.0.0" }

sources:
  - src/cpu_top.sv
EOF

cat > project/cpu_design/src/cpu_top.sv << 'EOF'
module cpu_top;
  // CPU top level with peripherals
endmodule
EOF

# Test with local repository
export BENDER_IP_REPO_PATH="$TEST_DIR/company_ips"

cd project/cpu_design

# Verify that IPs are resolved from local repository
OUTPUT=$(/workspaces/bender/target/debug/bender packages 2>&1)

if ! echo "$OUTPUT" | grep -q "axi_lib"; then
    echo "Error: axi_lib should be found"
    exit 1
fi

if ! echo "$OUTPUT" | grep -q "uart_ip"; then
    echo "Error: uart_ip should be found"  
    exit 1
fi

# Verify sources command works
/workspaces/bender/target/debug/bender sources > /dev/null
echo "✓ Local IP repository resolution works"

echo "=== Test 2: Script generation ==="

# Test script generation
/workspaces/bender/target/debug/bender script flist > flist_output.txt
if [ ! -s flist_output.txt ]; then
    echo "Error: Script generation failed"
    exit 1
fi

if ! grep -q "axi_pkg.sv" flist_output.txt; then
    echo "Error: Local IP sources not included in script output"
    exit 1
fi

echo "✓ Script generation works with local dependencies"

echo "=== Test 3: Search path priority ==="

# Test search path priority
mkdir -p "$TEST_DIR/priority_test/path1/priority_ip" "$TEST_DIR/priority_test/path2/priority_ip"

# Create different versions in different paths
cat > "$TEST_DIR/priority_test/path1/priority_ip/Bender.yml" << 'EOF'
package:
  name: priority_ip
  authors: ["Team 1"]
sources:
  - priority1.sv
EOF

cat > "$TEST_DIR/priority_test/path2/priority_ip/Bender.yml" << 'EOF'
package:
  name: priority_ip
  authors: ["Team 2"]
sources:
  - priority2.sv
EOF

echo 'module priority1; endmodule' > "$TEST_DIR/priority_test/path1/priority_ip/priority1.sv"
echo 'module priority2; endmodule' > "$TEST_DIR/priority_test/path2/priority_ip/priority2.sv"

# Create consumer project
mkdir -p "$TEST_DIR/priority_test/consumer/src"
cat > "$TEST_DIR/priority_test/consumer/Bender.yml" << 'EOF'
package:
  name: consumer
dependencies:
  priority_ip: { git: "https://fake.url/priority_ip.git", version: "1.0.0" }
sources:
  - src/consumer.sv
EOF

echo 'module consumer; endmodule' > "$TEST_DIR/priority_test/consumer/src/consumer.sv"

# Test with path1 first
export BENDER_IP_REPO_PATH="$TEST_DIR/priority_test/path1:$TEST_DIR/priority_test/path2"
cd "$TEST_DIR/priority_test/consumer"

SOURCES_OUTPUT=$(/workspaces/bender/target/debug/bender sources --flatten 2>&1)
if ! echo "$SOURCES_OUTPUT" | grep -q "priority1.sv"; then
    echo "Error: Should use first path in search order"
    exit 1
fi

echo "✓ Search path priority works correctly"

echo ""
echo "=== All comprehensive integration tests passed! ==="

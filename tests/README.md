# Test Documentation: BENDER_IP_REPO_PATH Feature

This directory contains comprehensive tests for the `BENDER_IP_REPO_PATH` feature, which allows Bender to search for IP dependencies in local directories before falling back to Git repositories.

## Test Structure

### Existing Tests
- `iss2_empty_dep_test.sh` - Original test for empty dependency handling
- `iss5_checkout_branch_test.sh` - Original test for branch checkout functionality

### New BENDER_IP_REPO_PATH Tests

#### 1. `bender_ip_repo_path_backward_compatibility_test.sh`
**Purpose**: Ensures the new feature doesn't break existing functionality.

**Coverage**:
- Basic Bender operations without environment variable set
- Functionality with empty `BENDER_IP_REPO_PATH`
- Functionality with non-existent paths
- Existing path dependencies continue to work

#### 2. `bender_ip_repo_path_core_functionality_test.sh`
**Purpose**: Tests the main search path override functionality.

**Coverage**:
- Standard layout: `path/ip_name/Bender.yml`
- Direct layout: `path/Bender.yml`
- Multiple search paths (colon-separated)
- Dependency resolution from search paths

#### 3. `bender_ip_repo_path_edge_cases_test.sh`
**Purpose**: Tests various edge cases and error conditions.

**Coverage**:
- Malformed environment variables (multiple colons, empty components)
- Conflicting dependency names in multiple search paths
- Fallback to Git when IP not found in search paths
- Special characters in paths
- Very long path strings

#### 4. `bender_ip_repo_path_cli_integration_test.sh`
**Purpose**: Ensures all CLI commands work properly with the new feature.

**Coverage**:
- `bender packages` command
- `bender sources` command with various flags
- `bender config` command
- `bender script` commands with different formats
- Target filtering functionality

#### 5. `bender_ip_repo_path_comprehensive_integration_test.sh`
**Purpose**: End-to-end workflow demonstration and integration testing.

**Coverage**:
- Realistic hardware design scenario
- Local IP repository management
- Script generation with local dependencies
- Search path priority handling
- Mixed local/external dependency scenarios

## Running Tests

### Run All Tests
```bash
cd tests/
./run_all.sh
```

### Run Individual Tests
```bash
cd tests/
./bender_ip_repo_path_core_functionality_test.sh
```

## Test Framework

The tests use the existing Bender test framework (`run_all.sh`) which:
- Automatically discovers all `*_test.sh` files
- Runs each test in isolation
- Reports pass/fail status
- Provides summary statistics

## Test Design Principles

1. **Isolation**: Each test creates its own temporary directory and cleans up after itself
2. **Robustness**: Tests handle expected failures gracefully
3. **Coverage**: Tests cover normal cases, edge cases, and error conditions
4. **Realistic**: Tests use realistic IP structures and dependency patterns
5. **Fast**: Tests are optimized for quick execution in CI/CD pipelines

## Environment Variables Used in Tests

- `BENDER_IP_REPO_PATH`: The main feature being tested
- `TEST_DIR`: Temporary directory for test isolation
- Various cleanup traps to ensure proper test isolation

## Expected Test Behavior

All tests should:
- Pass consistently when run individually or as part of the suite
- Clean up their temporary files and directories
- Not interfere with each other
- Complete within reasonable time limits
- Provide clear error messages when they fail

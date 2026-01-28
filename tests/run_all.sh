#!/bin/bash
# Copyright (c) 2017-2018 ETH Zurich
# Fabian Schuiki <fschuiki@iis.ee.ethz.ch>


set -e
# CRST=`tput sgr0`
# CNAME=`tput bold`
# CFAIL=`tput setaf 1`
# CPASS=`tput setaf 2`

TEST_OUTPUT=`mktemp`
TESTS_DIR="$(dirname "${BASH_SOURCE[0]}")"
PROJECT_ROOT="$(cd "$TESTS_DIR/.." && pwd)"

# Build bender
(cd "$PROJECT_ROOT" && cargo build)

# Find the bender binary
if [ -z "$BENDER" ]; then
    DEBUG_BIN="$PROJECT_ROOT/target/debug/bender"
    if [ -f "$DEBUG_BIN" ]; then
        export BENDER="$DEBUG_BIN"
    elif [ -f "$DEBUG_BIN.exe" ]; then
        export BENDER="$DEBUG_BIN.exe"
    fi
fi

NUM_PASS=0
NUM_FAIL=0
while read -d $'\0' TEST; do
	echo -n "running ${CNAME}$TEST${CRST} ..."
	if ! $TEST &> $TEST_OUTPUT; then
		NUM_FAIL=$((NUM_FAIL+1))
		echo " ${CFAIL}failed${CRST}"
		cat $TEST_OUTPUT
	else
		NUM_PASS=$((NUM_PASS+1))
		echo " ${CPASS}passed${CRST}"
	fi
done < <(find $TESTS_DIR -name "*_test.sh" -print0)

echo
if [ $NUM_FAIL -gt 0 ]; then
    echo "  ${CNAME}result: ${CFAIL}$NUM_FAIL/$((NUM_FAIL+NUM_PASS)) failed${CRST}"
else
    echo "  ${CNAME}result: ${CPASS}$NUM_PASS passed${CRST}"
fi
echo
[ $NUM_FAIL = 0 ] # return non-zero exit code if anything failed

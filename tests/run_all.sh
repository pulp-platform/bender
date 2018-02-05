#!/bin/bash
# Copyright (c) 2017-2018 ETH Zurich
# Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

set -e
CRST=`tput sgr0`
CNAME=`tput bold`
CFAIL=`tput setaf 1`
CPASS=`tput setaf 2`

TMP=`mktemp`
TESTS_DIR="$(dirname "${BASH_SOURCE[0]}")"

NUM_PASS=0
NUM_FAIL=0
while read -d $'\0' TEST; do
	echo -n "running ${CNAME}$TEST${CRST} ..."
	if ! $TEST &> $TMP; then
		NUM_FAIL=$((NUM_FAIL+1))
		echo " ${CFAIL}failed${CRST}"
		cat $TMP
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

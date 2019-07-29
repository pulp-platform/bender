#!/usr/bin/env bash

if [ "$TRAVIS_RUST_VERSION" = "stable" -a -n "$TRAVIS_TAG" ]; then
    "$@"
fi

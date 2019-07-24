#!/usr/bin/env bash

if [ "$TRAVIS_RUST_VERSION" = "1.35.0" -a -n "$TRAVIS_TAG" ]; then
    "$@"
fi

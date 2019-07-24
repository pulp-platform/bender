#!/usr/bin/env bash

if [ -z "$TRAVIS_TAG" ]; then
    readonly pkgver="$TRAVIS_BRANCH"
else
    if [[ "$TRAVIS_TAG" =~ ^v.*$ ]]; then
        readonly pkgver="$(echo $TRAVIS_TAG | sed -n 's/^v//p')"
    else
        readonly pkgver="$TRAVIS_TAG"
    fi
fi

if [ -z "$1" ]; then
    readonly release_dir="target/release"
    readonly tar_suffix=""
else
    readonly release_dir="target/$1/release"
    readonly tar_suffix="-$1"
fi

tar -czf "bender-$pkgver-x86_64-linux-gnu$tar_suffix.tar.gz" \
        -C "./$release_dir" \
        --owner=0 --group=0 \
        bender

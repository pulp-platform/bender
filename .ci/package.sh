#!/usr/bin/env bash

readonly pkgver="$(git tag -l --points-at HEAD | grep '^v.*$' | sed -n 's/^v//p')"

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

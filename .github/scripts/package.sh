#!/usr/bin/env bash

if [[ "$GITHUB_REF" =~ ^refs/tags/v.*$ ]]; then
    readonly pkgver="$(echo $GITHUB_REF | sed -n 's/^refs\/tags\/v//p')"
else
    readonly pkgver="$(echo $GITHUB_REF | sed -n 's/^refs\/tags\///p')"
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

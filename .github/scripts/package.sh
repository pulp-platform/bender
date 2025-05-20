#!/usr/bin/env bash

if [[ "$GITHUB_REF" =~ ^refs/tags/v.*$ ]]; then
    readonly pkgver="$(echo $GITHUB_REF | sed -n 's/^refs\/tags\/v//p')"
else
    readonly pkgver="$(echo $GITHUB_REF | sed -n 's/^refs\/tags\///p')"
fi

if [ -n "$pkgver" ]; then
    pkgver="-$pkgver"
fi

if [ -z "$1" ] && [ -z "$2" ]; then # no arguments
    readonly release_dir="target/release"
    readonly tar_suffix=""
    readonly tar_prefix=""
elif [ -n "$1" ] && [ -n "$2" ]; then # both arguments
    readonly release_dir="target/$2/$1/release"
    readonly tar_suffix="-$1"
    readonly tar_prefix="-$2"
elif [ -n "$1" ] && [ -z "$2" ]; then # only first argument
    readonly release_dir="target/$1/release"
    readonly tar_suffix="-$1"
    readonly tar_prefix=""
fi

tar -czf "bender$pkgver$tar_prefix-linux-gnu$tar_suffix.tar.gz" \
        -C "./$release_dir" \
        --owner=0 --group=0 \
        bender

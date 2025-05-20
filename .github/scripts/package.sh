#!/usr/bin/env bash

if [[ "$GITHUB_REF" =~ ^refs/tags/v.*$ ]]; then
    pkgver="$(echo $GITHUB_REF | sed -n 's/^refs\/tags\/v//p')"
else
    pkgver="$(echo $GITHUB_REF | sed -n 's/^refs\/tags\///p')"
fi

if [ -z "$pkgver" ]; then
    pkgver="latest"
fi

if [ -z "$1" ] && [ -z "$2" ]; then # no arguments
    release_dir="target/release"
    tar_suffix=""
    tar_prefix="-x86_64"
elif [ -n "$1" ] && [ -n "$2" ]; then # both arguments
    release_dir="target/$2/$1/release"
    tar_suffix="-$1"
    tar_prefix="-$2"
elif [ -n "$1" ] && [ -z "$2" ]; then # only first argument
    release_dir="target/$1/release"
    tar_suffix="-$1"
    tar_prefix="-x86_64"
fi

# WIESEP: Change amd64 to x86_64 to keep release names compatible with previous releases
if [ "$tar_prefix" == "-amd64" ]; then
    tar_prefix="-x86_64"
fi

tar -czf "bender-$pkgver$tar_prefix-linux-gnu$tar_suffix.tar.gz" \
        -C "./$release_dir" \
        --owner=0 --group=0 \
        bender

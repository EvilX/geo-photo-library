#!/usr/bin/env bash
# Сборка RPM: исходники (git, включая незакоммиченные изменения отслеживаемых файлов)
# + вендоринг зависимостей Cargo → rpmbuild. Результат в target/rpmbuild/RPMS.
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=geo-photo-library
SPEC=packaging/$NAME.spec
VERSION=$(rpmspec -q --srpm --qf '%{version}' "$SPEC")
TOP=$PWD/target/rpmbuild
mkdir -p "$TOP"/{SOURCES,SPECS}

REV=$(git stash create || true)
git archive --format=tar.gz --prefix="$NAME-$VERSION/" -o "$TOP/SOURCES/$NAME-$VERSION.tar.gz" "${REV:-HEAD}"

VENDOR_TMP=$(mktemp -d)
trap 'rm -rf "$VENDOR_TMP"' EXIT
cargo vendor --locked --versioned-dirs "$VENDOR_TMP/vendor" > /dev/null
tar -C "$VENDOR_TMP" -cJf "$TOP/SOURCES/$NAME-$VERSION-vendor.tar.xz" vendor

rpmbuild -bb --define "_topdir $TOP" "$SPEC"
find "$TOP/RPMS" -name "$NAME-*.rpm" -newer "$SPEC"

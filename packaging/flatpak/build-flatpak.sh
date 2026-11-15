#!/usr/bin/env bash
# Сборка Flatpak: исходники (git, включая незакоммиченные изменения отслеживаемых файлов)
# + вендоринг зависимостей Cargo → flatpak-builder → бандл target/flatpak/geo-photo-library.flatpak.
# SDK GNOME и расширение rust-stable ставятся в пользовательскую установку (без root).
set -euo pipefail
cd "$(dirname "$0")/../.."

NAME=geo-photo-library
APP_ID=io.github.EvilX.GeoPhotoLibrary
MANIFEST=packaging/flatpak/$APP_ID.yml
OUT=$PWD/target/flatpak
mkdir -p "$OUT"

REV=$(git stash create || true)
git archive --format=tar.gz --prefix="$NAME/" -o "$OUT/$NAME-src.tar.gz" "${REV:-HEAD}"

VENDOR_TMP=$(mktemp -d)
trap 'rm -rf "$VENDOR_TMP"' EXIT
cargo vendor --locked --versioned-dirs "$VENDOR_TMP/vendor" > /dev/null
tar -C "$VENDOR_TMP" -cJf "$OUT/$NAME-vendor.tar.xz" vendor

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
# flatpak-builder из системы или из Flathub (org.flatpak.Builder).
if command -v flatpak-builder > /dev/null; then
    BUILDER=(flatpak-builder)
else
    flatpak install --user -y --noninteractive flathub org.flatpak.Builder
    BUILDER=(flatpak run org.flatpak.Builder)
fi

"${BUILDER[@]}" --user --force-clean --install-deps-from=flathub \
    --state-dir="$OUT/state" --repo="$OUT/repo" "$OUT/build" "$MANIFEST"
flatpak build-bundle "$OUT/repo" "$OUT/$NAME.flatpak" "$APP_ID"
echo "$OUT/$NAME.flatpak"

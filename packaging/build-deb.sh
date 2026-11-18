#!/usr/bin/env bash
# Сборка пакета deb для Ubuntu (24.04, 26.04): release-бинарник из текущего дерева,
# файлы data/, зависимости библиотек — через dpkg-shlibdeps. Результат в target/deb.
# Нужны: cargo (rustup — в Ubuntu 24.04 Rust слишком старый), build-essential, pkg-config,
# libgtk-4-dev libshumate-dev libsoup-3.0-dev libheif-dev, dpkg-dev,
# desktop-file-utils, appstream.
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=geo-photo-library
APP_ID=io.github.EvilX.GeoPhotoLibrary
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# Выпуск Ubuntu — в версии пакета: пакеты для 24.04 и 26.04 различаются, и при обновлении
# системы ~ubuntu24.04 < ~ubuntu26.04 — пакет заменится на собранный для новой.
# os-release читается в подоболочке: в нём есть своя переменная NAME.
DISTRO=$(. /etc/os-release && echo "$ID$VERSION_ID")
DEB_VERSION="$VERSION-1~$DISTRO"
ARCH=$(dpkg --print-architecture)
OUT=$PWD/target/deb
ROOT=$OUT/root
rm -rf "$ROOT"
mkdir -p "$ROOT/DEBIAN"

# .cargo/config.toml задаёт PKG_CONFIG=/usr/bin/pkg-config — в Ubuntu это и есть системный.
cargo build --release --locked
cargo test --release --locked

install -Dm0755 target/release/$NAME "$ROOT/usr/bin/$NAME"
strip --strip-unneeded "$ROOT/usr/bin/$NAME"
install -Dm0644 data/$APP_ID.desktop "$ROOT/usr/share/applications/$APP_ID.desktop"
install -Dm0644 data/$APP_ID.metainfo.xml "$ROOT/usr/share/metainfo/$APP_ID.metainfo.xml"
install -Dm0644 data/icons/$APP_ID.svg "$ROOT/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm0644 LICENSE "$ROOT/usr/share/doc/$NAME/copyright"
install -Dm0644 README.md "$ROOT/usr/share/doc/$NAME/README.md"

desktop-file-validate "$ROOT/usr/share/applications/$APP_ID.desktop"
appstreamcli validate --no-net "$ROOT/usr/share/metainfo/$APP_ID.metainfo.xml"

# dpkg-shlibdeps ищет debian/control в текущем каталоге — даём ему заготовку.
SHLIBS_TMP=$(mktemp -d)
trap 'rm -rf "$SHLIBS_TMP"' EXIT
mkdir -p "$SHLIBS_TMP/debian"
printf 'Source: %s\n\nPackage: %s\nArchitecture: any\n' "$NAME" "$NAME" > "$SHLIBS_TMP/debian/control"
DEPENDS=$(cd "$SHLIBS_TMP" && dpkg-shlibdeps -O "$ROOT/usr/bin/$NAME" | sed -n 's/^shlibs:Depends=//p')

SIZE=$(du -sk --exclude=DEBIAN "$ROOT" | cut -f1)
cat > "$ROOT/DEBIAN/control" <<EOF
Package: $NAME
Version: $DEB_VERSION
Architecture: $ARCH
Maintainer: Leonid Nasedkin <leonidnasedkin@gmail.com>
Installed-Size: $SIZE
Depends: $DEPENDS, hicolor-icon-theme
Recommends: libheif-plugin-libde265
Section: graphics
Priority: optional
Homepage: https://github.com/EvilX/geo-photo-library
Description: Photo catalog with map search by EXIF GPS coordinates
 GeoPhotoLibrary indexes a folder of photos (JPEG, HEIC, PNG, WebP, TIFF),
 shows them in a year/month/day tree and places them on an OpenStreetMap map
 using GPS coordinates from EXIF metadata.
EOF

DEB=$OUT/${NAME}_${DEB_VERSION}_${ARCH}.deb
dpkg-deb --build --root-owner-group "$ROOT" "$DEB" > /dev/null
dpkg-deb --info "$DEB" | sed -n '/Package:/,$p'
echo "$DEB"

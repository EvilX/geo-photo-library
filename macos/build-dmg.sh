#!/bin/sh
# Образ диска с приложением macOS: target/macos/Geo-photo-library-<версия>-<архитектура>.dmg.
# Внутри — приложение и ссылка на «Программы» для установки перетаскиванием.
# Подпись ad-hoc, без нотаризации: скачанное из сети приложение Gatekeeper сам не откроет
# (см. README, раздел про macOS).
set -eu
cd "$(dirname "$0")/.."

./macos/build-app.sh

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="target/macos/Geo photo library.app"
DMG="target/macos/Geo-photo-library-$VERSION-$(uname -m).dmg"
STAGE=target/macos/dmg
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "Geo photo library" -srcfolder "$STAGE" -format UDZO -ov "$DMG"
rm -rf "$STAGE"
echo "$DMG"

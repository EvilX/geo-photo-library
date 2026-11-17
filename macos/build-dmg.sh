#!/bin/sh
# Образ диска с приложением macOS: target/macos/GeoPhotoLibrary-<версия>-<архитектура>.dmg.
# Внутри — приложение, ссылка на «Программы» для установки перетаскиванием и лицензия.
# Подпись ad-hoc, без нотаризации: скачанное из сети приложение Gatekeeper сам не откроет
# (см. README, раздел про macOS).
set -eu
cd "$(dirname "$0")/.."

./macos/build-app.sh

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="target/macos/GeoPhotoLibrary.app"
DMG="target/macos/GeoPhotoLibrary-$VERSION-$(uname -m).dmg"
STAGE=target/macos/dmg
# Все прежние образы, не только этой версии: в CI каталог восстанавливается из кэша,
# и под маску *.dmg попали бы образы старых версий.
rm -rf "$STAGE" target/macos/*.dmg
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp LICENSE "$STAGE/LICENSE.txt"
hdiutil create -quiet -volname "GeoPhotoLibrary" -srcfolder "$STAGE" -format UDZO -ov "$DMG"
rm -rf "$STAGE"
echo "$DMG"

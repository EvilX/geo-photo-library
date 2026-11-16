#!/bin/sh
# Сборка приложения macOS: ядро на Rust (статическая библиотека) + интерфейс на Swift.
# Результат — target/macos/Geo photo library.app (подпись ad-hoc, для этой машины).
#   ./macos/build-app.sh          собрать
#   ./macos/build-app.sh --run    собрать и запустить
set -eu
cd "$(dirname "$0")/.."

# Макросы SwiftUI есть только в полном Xcode: если выбран Command Line Tools — берём Xcode.
if [ -z "${DEVELOPER_DIR:-}" ] && [ "$(xcode-select -p)" = /Library/Developer/CommandLineTools ] \
    && [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi
# Компилятор, а не `xcodebuild -version`: тот работает и без принятой лицензии.
if ! cc --version > /dev/null 2>&1; then
    echo "Нужен Xcode с принятой лицензией: sudo xcodebuild -license accept" >&2
    exit 1
fi
command -v cargo > /dev/null || . "$HOME/.cargo/env"

# Та же минимальная версия системы, что в Package.swift, — иначе компоновщик предупреждает.
export MACOSX_DEPLOYMENT_TARGET=14.0
# Без GTK и libheif: HEIC на macOS декодирует ImageIO (HeifDecoder.swift).
cargo rustc --release --locked --lib --no-default-features --features ffi --crate-type staticlib
swift build -c release --package-path macos

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="target/macos/Geo photo library.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp macos/.build/release/GeoPhotoLibrary "$APP/Contents/MacOS/"
sed "s/@VERSION@/$VERSION/" macos/Info.plist > "$APP/Contents/Info.plist"
# Переводы строк Info.plist (запрос доступа к геолокации).
cp -R macos/Resources/*.lproj "$APP/Contents/Resources/"

# Иконка из того же SVG, что и на Linux.
ICONSET=target/macos/AppIcon.iconset
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    for scale in 1 2; do
        px=$((size * scale))
        name="icon_${size}x${size}"
        [ $scale = 2 ] && name="${name}@2x"
        sips -s format png -z $px $px data/icons/io.github.EvilX.GeoPhotoLibrary.svg \
            --out "$ICONSET/$name.png" > /dev/null
    done
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

codesign --force --sign - "$APP"
echo "$APP"
if [ "${1:-}" = --run ]; then
    open "$APP"
fi

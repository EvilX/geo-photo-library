#!/bin/sh
# Сборка приложения macOS: ядро на Rust (статическая библиотека) + интерфейс на Swift.
# Результат — target/macos/GeoPhotoLibrary.app (подпись ad-hoc, для этой машины).
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
APP="target/macos/GeoPhotoLibrary.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp macos/.build/release/GeoPhotoLibrary "$APP/Contents/MacOS/"
sed "s/@VERSION@/$VERSION/" macos/Info.plist > "$APP/Contents/Info.plist"
# Переводы строк Info.plist (запрос доступа к геолокации).
cp -R macos/Resources/*.lproj "$APP/Contents/Resources/"

# Иконка для macOS 14–15 (.icns): свой SVG на весь квадрат (рисунок тот же, что на Linux).
ICONSET=target/macos/AppIcon.iconset
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    for scale in 1 2; do
        px=$((size * scale))
        name="icon_${size}x${size}"
        [ $scale = 2 ] && name="${name}@2x"
        sips -s format png -z $px $px macos/AppIcon.svg \
            --out "$ICONSET/$name.png" > /dev/null
    done
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

# Тёмный вариант иконки (macOS 26+): только в формате Icon Composer (.icon), собранном
# actool в Assets.car (в обычном каталоге .appiconset тёмные картинки для macOS
# actool выбрасывает). Один слой на весь квадрат, для тёмного оформления — своя картинка.
# Обе картинки — только в image-name-specializations: при отдельном image-name тёмная
# молча пропадает.
# Старый Xcode .icon не знает — тогда без тёмного варианта, остаётся .icns.
ICON=target/macos/AppIcon.icon
rm -rf "$ICON" target/macos/actool
mkdir -p "$ICON/Assets" target/macos/actool
sips -s format png -z 1024 1024 macos/AppIcon.svg --out "$ICON/Assets/light.png" > /dev/null
sips -s format png -z 1024 1024 macos/AppIcon-dark.svg --out "$ICON/Assets/dark.png" > /dev/null
cat > "$ICON/icon.json" <<'JSON'
{
  "fill" : "automatic",
  "groups" : [
    {
      "layers" : [
        {
          "image-name-specializations" : [
            { "value" : "light.png" },
            { "appearance" : "dark", "value" : "dark.png" }
          ],
          "name" : "icon"
        }
      ],
      "shadow" : { "kind" : "none", "opacity" : 0 },
      "translucency" : { "enabled" : false, "value" : 0 }
    }
  ],
  "supported-platforms" : { "squares" : [ "macOS" ] }
}
JSON
if xcrun actool --compile target/macos/actool --platform macosx --minimum-deployment-target 14.0 \
        --app-icon AppIcon --output-partial-info-plist target/macos/actool/partial.plist \
        "$ICON" > /dev/null 2>&1 && [ -f target/macos/actool/Assets.car ]; then
    cp target/macos/actool/Assets.car "$APP/Contents/Resources/"
else
    echo "actool не собрал AppIcon.icon (нужен Xcode 26+) — иконка без тёмного варианта" >&2
fi

codesign --force --sign - "$APP"
echo "$APP"
if [ "${1:-}" = --run ]; then
    open "$APP"
fi

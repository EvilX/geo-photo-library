#!/usr/bin/env bash
# Сборка для Windows в оболочке MSYS2 (UCRT64 — x64, CLANGARM64 — ARM64):
#   ./packaging/windows/build-windows.sh [--no-test]
# → target/windows/GeoPhotoLibrary/                       — программа с GTK, готовая к запуску
#   target/windows/GeoPhotoLibrary-<версия>-<арх>.zip      — переносная версия
#   target/windows/GeoPhotoLibrary-<версия>-<арх>-setup.exe — установщик (если есть Inno Setup)
#
# Пакеты MSYS2 (P=mingw-w64-ucrt-x86_64 или mingw-w64-clang-aarch64):
#   $P-toolchain $P-rust $P-pkgconf $P-gtk4 $P-libshumate $P-libheif $P-libsoup3
#   $P-glib-networking $P-librsvg $P-ntldd zip
# Установщик собирает ISCC.exe из Inno Setup 6 или 7 (ищется в PATH и в Program Files).
set -euo pipefail

: "${MINGW_PREFIX:?Запускать в оболочке MSYS2 UCRT64 или CLANGARM64}"
cd "$(dirname "$(realpath "$0")")/../.."
ROOT=$PWD
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
case "$MSYSTEM_CARCH" in
    x86_64) ARCH=x64 ;;
    aarch64) ARCH=arm64 ;;
    *) echo "Неизвестная архитектура $MSYSTEM_CARCH" >&2; exit 1 ;;
esac
OUT=target/windows
APP=$OUT/GeoPhotoLibrary
NAME=GeoPhotoLibrary-$VERSION-$ARCH

# .cargo/config.toml задаёт PKG_CONFIG=/usr/bin/pkg-config для Fedora — здесь свой.
export PKG_CONFIG="$MINGW_PREFIX/bin/pkgconf.exe"
cargo build --release --locked
[ "${1:-}" = --no-test ] || cargo test --release --locked

rm -rf "$OUT"
mkdir -p "$APP/bin" "$APP/lib" "$APP/share"
cp target/release/geo-photo-library.exe "$APP/bin/"
# gdbus.exe: через него GApplication поднимает сессию D-Bus — без неё второй запуск
# не находит уже открытое окно; gspawn-*-helper — запуск программ из GLib.
cp "$MINGW_PREFIX/bin/gdbus.exe" "$APP/bin/"
cp "$MINGW_PREFIX"/bin/gspawn-win*-helper*.exe "$APP/bin/" 2>/dev/null || true

# Модули GLib и GDK: TLS для тайлов OSM и поиска местности (glib-networking),
# загрузчики картинок gdk-pixbuf (SVG — librsvg).
GIO_MODULES=lib/gio/modules
PIXBUF=lib/gdk-pixbuf-2.0/2.10.0
mkdir -p "$APP/$GIO_MODULES" "$APP/$PIXBUF/loaders"
cp "$MINGW_PREFIX/$GIO_MODULES/libgiognutls.dll" "$APP/$GIO_MODULES/"
cp "$MINGW_PREFIX/$PIXBUF"/loaders/*.dll "$APP/$PIXBUF/loaders/"
rm -f "$APP/$PIXBUF"/loaders/*.dll.a

# Все DLL из MSYS2, от которых зависят exe и модули. Не ldd: на ARM64 он работает под
# эмуляцией x64 и теряет часть зависимостей; ntldd -R разбирает PE сам, по всей цепочке.
PREFIX_W=$(cygpath -w "$MINGW_PREFIX/bin")
ntldd -R "$APP"/bin/*.exe "$APP/$GIO_MODULES"/*.dll "$APP/$PIXBUF"/loaders/*.dll \
    | P="$PREFIX_W\\" awk '{ f = $3; sub(/\r$/, "", f); if (index(tolower(f), tolower(ENVIRON["P"])) == 1) print f }' \
    | sort -u | cygpath -u -f - | xargs -r cp -n -t "$APP/bin/"

# Кэши модулей. Пути в loaders.cache MSYS2 уже относительные (от каталога программы).
"$MINGW_PREFIX/bin/gio-querymodules.exe" "$APP/$GIO_MODULES"
cp "$MINGW_PREFIX/$PIXBUF/loaders.cache" "$APP/$PIXBUF/"

# Схемы GSettings (диалоги GTK) и темы значков; своя иконка — в hicolor.
mkdir -p "$APP/share/glib-2.0/schemas"
cp "$MINGW_PREFIX"/share/glib-2.0/schemas/org.gtk.gtk4.*.gschema.xml "$APP/share/glib-2.0/schemas/"
"$MINGW_PREFIX/bin/glib-compile-schemas.exe" "$APP/share/glib-2.0/schemas"
mkdir -p "$APP/share/icons"
cp -r "$MINGW_PREFIX/share/icons/Adwaita" "$MINGW_PREFIX/share/icons/hicolor" "$APP/share/icons/"
install -Dm644 data/icons/io.github.EvilX.GeoPhotoLibrary.svg \
    "$APP/share/icons/hicolor/scalable/apps/io.github.EvilX.GeoPhotoLibrary.svg"
"$MINGW_PREFIX/bin/gtk4-update-icon-cache.exe" -q -t -f "$APP/share/icons/hicolor"
"$MINGW_PREFIX/bin/gtk4-update-icon-cache.exe" -q -t -f "$APP/share/icons/Adwaita"

# Переводы самого GTK (кнопки диалогов) — только языки интерфейса программы.
for l in ru el; do
    for d in gtk40 glib20; do
        f="$MINGW_PREFIX/share/locale/$l/LC_MESSAGES/$d.mo"
        [ -f "$f" ] && install -Dm644 "$f" "$APP/share/locale/$l/LC_MESSAGES/$d.mo"
    done
done

cp LICENSE "$APP/LICENSE.txt"
echo "Каталог программы: $APP ($(du -sh "$APP" | cut -f1))"

# Переносная версия.
(cd "$OUT" && rm -f "$NAME.zip" && zip -qr9 "$NAME.zip" GeoPhotoLibrary)
echo "$OUT/$NAME.zip"

# Установщик.
ISCC=$(command -v ISCC.exe || true)
for d in "/c/Program Files (x86)"/Inno\ Setup\ * "/c/Program Files"/Inno\ Setup\ * "$LOCALAPPDATA"/Programs/Inno\ Setup\ *; do
    [ -z "$ISCC" ] && [ -x "$d/ISCC.exe" ] && ISCC="$d/ISCC.exe"
done
if [ -z "$ISCC" ]; then
    echo "Inno Setup не найден — установщик не собран" >&2
    exit 0
fi
"$ISCC" -Q "-DAppVersion=$VERSION" "-DAppArch=$ARCH" "-DSourceDir=$(cygpath -w "$ROOT/$APP")" \
    "-DOutputDir=$(cygpath -w "$ROOT/$OUT")" "-DOutputName=$NAME-setup" \
    "$(cygpath -w "$ROOT/packaging/windows/geo-photo-library.iss")"
echo "$OUT/$NAME-setup.exe"

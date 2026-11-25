#!/usr/bin/env bash
# Репозитории пакетов для dnf (Fedora) и apt (Ubuntu) — статические файлы для GitHub Pages.
#   ./packaging/build-repo.sh <каталог с новыми .rpm и .deb> <каталог сайта>
# Новые пакеты добавляются к уже лежащим на сайте (ветка gh-pages), у каждого выпуска
# системы и архитектуры остаются последние KEEP версий; новые RPM и метаданные
# подписываются ключом GPG_KEY_ID (ключ уже импортирован в gpg, без пароля).
# Нужны: createrepo_c (в Ubuntu — пакет createrepo-c), apt-ftparchive (apt-utils),
# rpmsign (пакет rpm), gpg.
#
# Раскладка сайта:
#   rpm/fedora/<выпуск>/<архитектура>/*.rpm + repodata/   — выпуск из .fc45 в имени
#   deb/pool/<кодовое имя>/*.deb, deb/dists/<кодовое имя>/ — выпуск из ~ubuntu24.04 в версии
#   geo-photo-library.repo, key.asc, key.gpg
#   index.html (лендинг packaging/site), icon.svg, screenshots/
set -euo pipefail

IN=$(realpath "$1")
SITE=$(realpath "$2")
KEEP=${KEEP:-3}
URL=${REPO_URL:-https://evilx.github.io/geo-photo-library}
NAME=geo-photo-library
: "${GPG_KEY_ID:?Нужен GPG_KEY_ID — ключ для подписи репозиториев}"

GPG=(gpg --batch --yes --local-user "$GPG_KEY_ID")

# Кодовые имена выпусков Ubuntu — для строки `deb … <имя> main` у пользователя.
codename() {
    case "$1" in
        24.04) echo noble ;;
        26.04) echo resolute ;;
        *) echo "Неизвестный выпуск Ubuntu $1 — добавить его кодовое имя в build-repo.sh" >&2; exit 1 ;;
    esac
}

# Оставить в каталоге последние KEEP файлов по версии (в каталоге — по файлу на версию).
prune() {
    local dir=$1 pattern=$2
    find "$dir" -maxdepth 1 -name "$pattern" -printf '%f\n' | sort -V | head -n -"$KEEP" \
        | while read -r f; do rm -v "$dir/$f"; done
}

# --- RPM ---
shopt -s nullglob
for rpm in "$IN"/*.rpm; do
    f=$(basename "$rpm")
    [[ $f == *.src.rpm ]] && continue
    # geo-photo-library-1.3.1-1.fc45.x86_64.rpm
    [[ $f =~ \.fc([0-9]+)\.([a-z0-9_]+)\.rpm$ ]] || { echo "Не разобрать имя $f" >&2; exit 1; }
    dir=$SITE/rpm/fedora/${BASH_REMATCH[1]}/${BASH_REMATCH[2]}
    mkdir -p "$dir"
    cp "$rpm" "$dir/"
    # Подпись самого пакета (gpgcheck) — подписываются только новые, старые уже подписаны.
    # Путь к gpg — явно: rpm из Ubuntu ищет его не там, где он лежит.
    rpmsign --addsign --define "_gpg_name $GPG_KEY_ID" --define "__gpg $(command -v gpg)" \
        "$dir/$f" > /dev/null 2>&1
done
for dir in "$SITE"/rpm/fedora/*/*/; do
    prune "$dir" "*.rpm"
    createrepo_c --quiet --update --no-database "$dir"
    # И подпись метаданных (repo_gpgcheck): без неё подменённый список версий не заметить.
    "${GPG[@]}" --armor --detach-sign --output "$dir/repodata/repomd.xml.asc" "$dir/repodata/repomd.xml"
done

# --- DEB ---
for deb in "$IN"/*.deb; do
    f=$(basename "$deb")
    # geo-photo-library_1.3.1-1~ubuntu24.04_amd64.deb
    [[ $f =~ ~ubuntu([0-9]+\.[0-9]+)_ ]] || { echo "Не разобрать имя $f" >&2; exit 1; }
    dir=$SITE/deb/pool/$(codename "${BASH_REMATCH[1]}")
    mkdir -p "$dir"
    cp "$deb" "$dir/"
done
if [ -d "$SITE/deb/pool" ]; then
    cd "$SITE/deb"
    for pool in pool/*/; do
        suite=$(basename "$pool")
        archs=$(find "$pool" -name '*.deb' -printf '%f\n' | sed 's/.*_\(.*\)\.deb$/\1/' | sort -u)
        for arch in $archs; do
            prune "$pool" "*_$arch.deb"
        done
        archs=$(find "$pool" -name '*.deb' -printf '%f\n' | sed 's/.*_\(.*\)\.deb$/\1/' | sort -u)
        rm -rf "dists/$suite"
        for arch in $archs; do
            out=dists/$suite/main/binary-$arch
            mkdir -p "$out"
            # Пути в Packages — от корня репозитория (deb/), как их ищет apt.
            apt-ftparchive --arch "$arch" packages "$pool" > "$out/Packages"
            gzip -9kn "$out/Packages"
        done
        apt-ftparchive \
            -o APT::FTPArchive::Release::Origin="$NAME" \
            -o APT::FTPArchive::Release::Label="$NAME" \
            -o APT::FTPArchive::Release::Suite="$suite" \
            -o APT::FTPArchive::Release::Codename="$suite" \
            -o APT::FTPArchive::Release::Components=main \
            -o APT::FTPArchive::Release::Architectures="${archs//$'\n'/ }" \
            release "dists/$suite" > "dists/$suite/Release"
        "${GPG[@]}" --clearsign --output "dists/$suite/InRelease" "dists/$suite/Release"
        "${GPG[@]}" --armor --detach-sign --output "dists/$suite/Release.gpg" "dists/$suite/Release"
    done
    cd - > /dev/null
fi

# --- Ключ, файл репозитория dnf, страница ---
gpg --batch --yes --armor --export "$GPG_KEY_ID" > "$SITE/key.asc"
gpg --batch --yes --export "$GPG_KEY_ID" > "$SITE/key.gpg"
cat > "$SITE/$NAME.repo" <<EOF
[$NAME]
name=GeoPhotoLibrary
baseurl=$URL/rpm/fedora/\$releasever/\$basearch/
enabled=1
repo_gpgcheck=1
gpgcheck=1
gpgkey=$URL/key.asc
EOF
# Без .nojekyll GitHub Pages пропускает файлы и каталоги, начинающиеся с «_».
touch "$SITE/.nojekyll"
# Страница проекта: лендинг из packaging/site, скриншоты и иконка — из репозитория.
SRC=$(dirname "$(realpath "$0")")/..
sed "s|@URL@|$URL|g" "$SRC/packaging/site/index.html" > "$SITE/index.html"
cp "$SRC/data/icons/io.github.EvilX.GeoPhotoLibrary.svg" "$SITE/icon.svg"
install -d "$SITE/screenshots"
cp "$SRC/docs/screenshots/macos-en.png" "$SRC/docs/screenshots/map-en.png" "$SITE/screenshots/"
find "$SITE" -name '*.rpm' -o -name '*.deb' | sed "s|$SITE/||" | sort

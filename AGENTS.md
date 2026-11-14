# AGENTS.md

Инструкции для ИИ-агентов, работающих с репозиторием **Geo photo library** —
каталогизатором фотографий на Rust + GTK4 + SQLite с поиском снимков на карте OSM
по GPS из EXIF. Пользовательское описание — в `README.md`.

## Язык и стиль

- Комментарии, строки интерфейса, сообщения об ошибках и коммиты — **на русском**.
- Коммиты: заголовок-суть, пустая строка, тело «что и почему» (см. `git log`).
- Перед коммитом: `cargo clippy --release --all-targets` без предупреждений и `cargo test --release`.

## Сборка и окружение (Fedora)

```sh
sudo dnf install gcc gtk4-devel libshumate-devel libheif-devel   # системные зависимости
cargo build --release
cargo test --release
```

- Rust установлен через rustup (`source ~/.cargo/env`), а не через dnf.
- Homebrew-овский `pkg-config` стоит в PATH раньше системного и не видит `.pc` файлы Fedora,
  поэтому `.cargo/config.toml` задаёт `PKG_CONFIG=/usr/bin/pkg-config`. Не удалять.
- `rusqlite` собирается с `bundled`, `mozjpeg` собирает libjpeg-turbo из исходников
  (без `nasm` — без SIMD, это нормально).
- Фича `heic` (по умолчанию) требует libheif; `--no-default-features` отключает HEIC.

## Структура

| Файл | Назначение |
| --- | --- |
| `src/main.rs` | запуск GTK-приложения; CLI-режим `--scan <папка>` без GUI |
| `src/scanner.rs` | инкрементальное сканирование (size+mtime), rayon, пачки по 64, отмена через `AtomicBool` |
| `src/metadata.rs` | EXIF (kamadak-exif): дата, GPS, ориентация, камера |
| `src/thumbs.rs` | квадратные миниатюры 320×320: JPEG через mozjpeg с IDCT-масштабом, HEIC — встроенная миниатюра, DNG — встроенное JPEG-превью |
| `src/dng.rs` | разбор IFD в DNG/TIFF и поиск JPEG-превью |
| `src/db.rs` | SQLite: схема, миграции (`PRAGMA user_version`), запросы |
| `src/geo.rs` | Web Mercator, видимая область, кластеризация по сетке |
| `src/trace.rs` | замеры времени (`GEO_PHOTO_LIBRARY_TRACE`) |
| `src/ui/window.rs` | окно, вкладки, настройки, фоновое сканирование, отладочные хуки |
| `src/ui/dates_tab.rs` | дерево год → месяц → день (`TreeListModel`) + сетка |
| `src/ui/map_tab.rs` | карта libshumate, маркеры-кластеры, список фото в видимой области |
| `src/ui/photo_grid.rs` | общая сетка миниатюр (`GridView`) |
| `src/ui/textures.rs` | асинхронная загрузка текстур, LRU-кэш 128 МБ |
| `examples/bench_thumbs.rs` | бенчмарк этапов генерации миниатюр |
| `packaging/` | RPM spec и `build-rpm.sh` |
| `data/` | иконка, `.desktop`, metainfo (ID `io.github.EvilX.GeoPhotoLibrary`) |

Данные пользователя: `~/.config/geo-photo-library/config.toml`,
`~/.local/share/geo-photo-library/library.db`, `~/.cache/geo-photo-library/thumbs/`.

## Важные инварианты и грабли

- **Миграции БД.** Формат данных версионируется `SCHEMA_VERSION` в `db.rs`. Чтобы сканер
  переобработал файлы, миграция ставит им `mtime = -1`. Новая логика миниатюр/метаданных,
  которая должна примениться к уже отсканированным файлам, — это новая версия схемы
  с миграцией и тестом.
- **Пути в БД абсолютные и канонические** (`root.canonicalize()` в сканере). Иначе
  относительный и абсолютный путь к одной папке дают полное пересканирование.
- **Недоступная папка не очищает БД:** если корня нет (диск не подключён), сканер
  возвращает ошибку до удаления «пропавших» записей. Не ломать.
- **mozjpeg сообщает об ошибках паникой** — декодирование обёрнуто в `catch_unwind`
  с откатом на крейт `image`.
- **HEIC:** libheif сам применяет поворот (`irot`), EXIF Orientation к HEIC не применять.
  Встроенная миниатюра берётся, только если её пропорции совпадают с основным снимком.
- **`GridView::set_max_columns`:** GTK держит виджеты примерно для `max_columns × 30`
  элементов. Завышенное значение (было 32) отключает виртуализацию — создаются ячейки
  для всех фото, интерфейс тормозит. Сейчас 12 (даты) и 4 (карта).
- **Всё тяжёлое — вне главного потока:** сканирование в `std::thread`, текстуры через
  `gio::spawn_blocking`. Ячейки сетки переиспользуются — перед установкой текстуры
  проверять, что ячейка показывает тот же элемент (`set_thumbnail(.., still_valid)`).
- **Смена папки во время сканирования** отменяет текущее и перезапускает
  (`rescan_pending`). Во время сканирования UI перечитывает БД каждые 3 с
  (`reload(state, false)` сохраняет раскрытые узлы и выбор).
- Зум карты при автоподгонке округляется до целого — на дробном видны швы тайлов.

## Проверка интерфейса без участия человека

GTK-приложение уникально по ID: если у пользователя оно уже открыто, повторный запуск
просто активирует его окно. Для автотестов запускать в отдельной сессии D-Bus и с
отдельными XDG-каталогами, чтобы не трогать данные пользователя:

```sh
X=/tmp/gpl-test
mkdir -p $X/cfg/geo-photo-library
echo "library_path = \"$PWD/testdata/library\"" > $X/cfg/geo-photo-library/config.toml
XDG_DATA_HOME=$X/data XDG_CACHE_HOME=$X/cache XDG_CONFIG_HOME=$X/cfg \
  GEO_PHOTO_LIBRARY_SCREENSHOT=/tmp/shot.png GEO_PHOTO_LIBRARY_TAB=map \
  dbus-run-session -- ./target/release/geo-photo-library
```

Переменные окружения:

| Переменная | Действие |
| --- | --- |
| `GEO_PHOTO_LIBRARY_TRACE=1` | печатать длительность операций (`[trace] …`) |
| `GEO_PHOTO_LIBRARY_SCREENSHOT=out.png` | через `GEO_PHOTO_LIBRARY_DELAY` с (по умолчанию 6) сохранить окно в PNG и выйти |
| `GEO_PHOTO_LIBRARY_TAB=map` | открыть вкладку карты |
| `GEO_PHOTO_LIBRARY_BENCH=1` | прогнать зумы карты 18…2 вокруг первого фото и выйти |
| `GEO_PHOTO_LIBRARY_DEBUG_SWITCH=<папка>` | через 3 с сменить папку библиотеки, как из настроек |

Скан без GUI: `geo-photo-library --scan <папка>`.
Бенчмарк миниатюр: `cargo run --release --example bench_thumbs -- <папка> [N]`.

## Тестовые данные

`testdata/` в `.gitignore`: `testdata/library` — 1000 синтетических 12-Мп снимков
(100 мест × 10, 17% HEIC со встроенными миниатюрами) с подставленным EXIF. Генератор
в репозиторий не входит. Реальную галерею пользователя (`~/OneDrive/Pictures`)
только читать и сканировать в отдельные XDG-каталоги.

Ориентиры производительности на этой выборке (i5-10210U, 8 потоков): первое
сканирование ~18 с, повторное ~0,1 с; JPEG 12 Мп ~45 мс, HEIC со встроенной миниатюрой
~12 мс на файл.

## Пакет RPM

```sh
./packaging/build-rpm.sh   # → target/rpmbuild/RPMS/x86_64/geo-photo-library-<версия>.rpm
```

Скрипт берёт исходники из git (включая незакоммиченные изменения отслеживаемых файлов),
вендорит зависимости Cargo (сборка без сети) и запускает `rpmbuild`; при Rust из rustup —
с `--nodeps`. `%check` гоняет тесты и валидирует `.desktop`/metainfo. Установка требует
root — агент её не выполняет, а даёт пользователю команду
(`pkexec dnf install -y ./target/rpmbuild/RPMS/x86_64/*.rpm`).

При выпуске версии синхронно менять: `version` в `Cargo.toml`, `Version`/`Release` и
`%changelog` в spec, `<releases>` в metainfo.

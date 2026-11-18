# AGENTS.md

Инструкции для ИИ-агентов, работающих с репозиторием **GeoPhotoLibrary** —
каталогизатором фотографий с поиском снимков на карте по GPS из EXIF: ядро на Rust + SQLite,
интерфейс GTK4 с картой OSM на Linux и нативный SwiftUI + MapKit на macOS. Пользовательское описание — в `README.md` (русский)
и `README.en.md` (английский).

## Язык и стиль

- Комментарии, отладочный вывод (`eprintln!`, `[trace]`) и коммиты — **на русском**.
- Интерфейс локализован: английский (по умолчанию), русский, греческий. Все строки,
  которые видит пользователь (окно, статусы, ошибки сканера, вывод `--scan`), — только
  через `src/i18n.rs`, сразу на трёх языках. Язык определяется один раз по `LANGUAGE`,
  `LC_ALL`, `LC_MESSAGES`, `LANG`; проверить: `LANG=el_GR.UTF-8 ./target/release/geo-photo-library`.
  Переводы `.desktop` и metainfo тоже держать для `ru` и `el`.
- Даты в интерфейсе — только через `i18n::datetime`: формат берётся из локали `LC_TIME`
  (`nl_langinfo`, без секунд), при локали C/POSIX — по языку интерфейса.
- Не запускать `cargo fmt`: код не отформатирован rustfmt (длинные строки), и он
  переформатирует весь крейт. Править оформление вручную.
- Коммиты: заголовок-суть, пустая строка, тело «что и почему» (см. `git log`).
- Перед коммитом: `cargo clippy --release --all-targets --features ffi` без предупреждений
  и `cargo test --release --features ffi`.

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
| `src/lib.rs` | ядро как библиотека: всё, кроме `main.rs`, `geocode.rs`, `location.rs` и `ui/` |
| `src/main.rs` | запуск GTK-приложения; CLI-режим `--scan <папка>` без GUI |
| `src/ffi.rs` | C-интерфейс ядра для приложения macOS (фича `ffi`) |
| `src/paths.rs` | каталоги данных: XDG через GLib, без GTK — `~/Library` |
| `src/scanner.rs` | инкрементальное сканирование (size+mtime), rayon, пачки по 64, отмена через `AtomicBool` |
| `src/metadata.rs` | EXIF (kamadak-exif): дата, GPS, ориентация, камера; подробности для панели «Информация» |
| `src/thumbs.rs` | квадратные миниатюры 320×320: JPEG через mozjpeg с IDCT-масштабом, HEIC — встроенная миниатюра, DNG — встроенное JPEG-превью |
| `src/dng.rs` | разбор IFD в DNG/TIFF и поиск JPEG-превью |
| `src/db.rs` | SQLite: схема, миграции (`PRAGMA user_version`), запросы |
| `src/geo.rs` | Web Mercator, видимая область, кластеризация по сетке |
| `src/location.rs` | текущее местоположение через портал `org.freedesktop.portal.Location` (GeoClue) по D-Bus |
| `src/geocode.rs` | поиск местности через Nominatim (libsoup): URL запроса, разбор ответа |
| `src/trace.rs` | замеры времени (`GEO_PHOTO_LIBRARY_TRACE`) |
| `src/i18n.rs` | локализация en/ru/el: выбор языка по локали, строки, названия месяцев, формат дат |
| `src/ui/window.rs` | окно, вкладки, главное меню, фоновое сканирование, отладочные хуки |
| `src/ui/settings.rs` | диалог настроек (папка библиотеки) |
| `src/ui/about.rs` | окно «О программе» |
| `src/ui/date_tree.rs` | дерево год → месяц → день (`TreeListModel`) со счётчиками: вкладка дат и фильтр карты («Всё время» первой строкой) |
| `src/ui/dates_tab.rs` | дерево дат + сетка фото выбранного периода |
| `src/ui/map_tab.rs` | карта libshumate, строка поиска местности, фильтр по датам (плавающая панель), маркеры-кластеры, список фото в видимой области |
| `src/ui/photo_grid.rs` | общая сетка миниатюр (`GridView`) и контекстное меню фото |
| `src/ui/menu.rs` | меню с иконками у пунктов (`icon_menu`) — главное и меню фото |
| `src/ui/info_panel.rs` | панель «Информация» справа: файл и EXIF выбранного фото (чтение в фоне) |
| `src/ui/textures.rs` | асинхронная загрузка текстур, LRU-кэш 128 МБ |
| `macos/` | нативный интерфейс macOS: пакет SwiftPM (SwiftUI + MapKit), `build-app.sh`, `build-dmg.sh`, `Info.plist`, `AppIcon.svg` (иконка на весь квадрат, рисунок как в `data/icons`) |
| `examples/bench_thumbs.rs` | бенчмарк этапов генерации миниатюр |
| `packaging/` | RPM spec и `build-rpm.sh`; `flatpak/` — манифест Flatpak и `build-flatpak.sh` |
| `data/` | иконка, `.desktop`, metainfo (ID `io.github.EvilX.GeoPhotoLibrary`) |
| `docs/screenshots/` | скриншоты для README: `map-ru.png`, `map-en.png` |

Данные пользователя: `~/.config/geo-photo-library/config.toml`,
`~/.local/share/geo-photo-library/library.db`, `~/.cache/geo-photo-library/thumbs/`.

## macOS

Крейт разделён на библиотеку-ядро и бинарник GTK. Фичи: `gtk-ui` (по умолчанию; GTK,
libshumate, libsoup — бинарник требует её), `heic` (libheif), `ffi` (C-интерфейс).
Приложение macOS — `macos/`: Swift вызывает ядро через `src/ffi.rs`.

```sh
./macos/build-app.sh [--run]   # → target/macos/GeoPhotoLibrary.app
./macos/build-dmg.sh           # → target/macos/GeoPhotoLibrary-<версия>-<архитектура>.dmg
cargo test --release --no-default-features --features ffi --lib   # тесты ядра без GTK
```

- Нужен полный Xcode с принятой лицензией (в Command Line Tools нет макросов SwiftUI);
  скрипт сам берёт `/Applications/Xcode.app`, если выбран CLT. Rust — из rustup.
- **`src/ffi.rs` и `macos/Sources/CGeoPhotoCore/include/gpl.h` править вместе:** заголовок
  написан вручную, несовпадение сигнатур компилятор не заметит.
- Интерфейс C без общего состояния: каждый запрос сам открывает БД, поэтому функции можно
  звать из любого потока; запросы и `gpl_scan` — не из главного.
- Строки интерфейса и на macOS — только из `i18n.rs` (`gpl_text*`, в Swift — `Core.text`).
  Язык Swift берёт из системных настроек и передаёт в `gpl_init` (`i18n::set_lang`).
  Проверить другой язык: `open -n "target/macos/GeoPhotoLibrary.app" --args -AppleLanguages "(ru)"`.
  Даты под фото Swift форматирует сам по системной локали.
- HEIC: libheif не используется, ядро вызывает декодер ImageIO из Swift
  (`thumbs::set_heif_decoder`, `HeifDecoder.swift`); поворот применяет ImageIO.
- Карта — MapKit. Зум Web Mercator считается по ширине видимой области
  (`20 + log2(ширина вида / ширина visibleMapRect)`), поэтому наклон и поворот карты выключены.
  Кластеры считает ядро (`gpl_map_view`), маркеры пересоздаются только изменившиеся —
  те же правила, что у `MapTab::refresh` в GTK.
- Данные: `~/Library/Application Support/geo-photo-library/` (настройки, БД),
  `~/Library/Caches/geo-photo-library/thumbs/`. В `viewer`/`editor` конфига на macOS лежат
  идентификаторы пакетов приложений, а не id `.desktop`.
- Поиск местности — `MKLocalSearch` (не Nominatim), местоположение — разовый запрос
  CoreLocation; точка рисуется своей аннотацией, `showsUserLocation` не включать — он держит
  геолокацию постоянно. Текст системного запроса доступа — в `macos/Info.plist` и
  `macos/Resources/*.lproj/InfoPlist.strings`, согласован с `i18n::location_purpose`.
- Панель «Информация»: содержимое готовит ядро (`gpl_photo_info`, JSON), Swift только рисует.
- **Чтения БД идут параллельно и могут ждать сканер** — результат применяется, только если
  он от последнего запрошенного чтения (`reloadGeneration` в `AppModel`), иначе устаревший
  ответ затирает свежие данные.
- MapKit не отдаляет карту дальше примерно зума 3 при обычном размере окна: подгонка под фото
  со всего мира показывает только часть.
- Проверка без человека — запускать бинарник из `.app` напрямую с `GEO_PHOTO_LIBRARY_HOME=<каталог>`
  (подменяет каталоги данных). Работают `GEO_PHOTO_LIBRARY_SCREENSHOT`, `_DELAY`, `_TAB`,
  `_ACTION`, `_INFO`, `_SEARCH`, `_LOCATE` с тем же смыслом, что на Linux, и
  `GEO_PHOTO_LIBRARY_PHOTO_ACTION=copy|show-on-map|trash` — пункт меню для первого фото вкладки
  (`trash` действительно перемещает файл в Корзину). `GEO_PHOTO_LIBRARY_DATE_FILTER=open|<год>` —
  открыть фильтр карты по датам (и выбрать год). `GEO_PHOTO_LIBRARY_APPEARANCE=dark|light` — тёмное
  или светлое оформление независимо от системного (аргумент `-AppleInterfaceStyle` не действует). `_LOCATE` покажет пользователю системный
  запрос доступа — без спроса не запускать. Язык: аргумент `-AppleLanguages "(ru)"`.
- **Не использовать `HSplitView`:** вкладки лежат друг на друге и живут постоянно, и
  разделитель скрытой вкладки продолжает управлять курсором (после перетаскивания он
  оставался «стрелками»). Панели делит свой `SplitPane` (`SplitPane.swift`).
- **Иконка macOS — `macos/AppIcon.svg`, без полей и скругления:** форму накладывает система,
  а иконку, не заполняющую квадрат, macOS 26 сажает на серую подложку. Меняя рисунок
  в `data/icons/…svg` (Linux, со скруглением и полями), менять и его.
- **Тёмный вариант иконки** — `macos/AppIcon-dark.svg` (тот же рисунок, ночная карта). `build-app.sh`
  собирает из обеих картинок `AppIcon.icon` (формат Icon Composer) и через `actool` кладёт
  `Assets.car` в пакет (`CFBundleIconName`); `.icns` остаётся для macOS 14–15. В `.appiconset`
  тёмные картинки для macOS `actool` молча выбрасывает. Обе картинки — только в
  `image-name-specializations`: при отдельном `image-name` тёмная тоже пропадает. Проверка:
  `xcrun assetutil --info …/Assets.car` — должны быть `AppIcon_Assets/light` и `…/dark`.
- Enter в сетке фото не открывает просмотр (в GTK открывает) — только двойной щелчок.
- **Фильтр карты по датам:** `AppModel.mapPeriod` (`nil` — всё время), фото карты —
  `gpl_db_geotagged(год, месяц, день)`. В дереве — только даты фото в видимой области карты:
  счётчики по всем фото с координатами (без учёта фильтра) пересчитываются при каждом
  обновлении карты (`gpl_visible_date_counts`, в GTK — `MapTab::refresh`). Выбранный период
  при этом не сбрасывается, даже если его узел пропал из дерева. Смена периода только перечитывает
  список: положение и масштаб карты не меняются (так решил пользователь — без подгонки).
  Панель плавает над картой (overlay) — её открытие не меняет размер карты.
- **Меню «Правка»** — действия с выбранным фото, без пунктов правки текста; ⌘X/⌘V/⌘Z/⌘A, а в поле
  ввода и ⌘C/⌘⌫ отдаёт строке поиска `TextEditKeys` (локальный монитор клавиш).
- Изменения ядра проверять и для GTK: на Mac — в контейнере
  (`docker run --rm -v "$PWD":/src:ro -e CARGO_TARGET_DIR=/target fedora:45 …`, зависимости как в CI).

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
  для всех фото, интерфейс тормозит. Сейчас 12 (даты) и 4 (карта) при масштабе
  миниатюр 100%; слайдер масштаба (50…200%) меняет предел обратно пропорционально,
  но не выше 16 (`MAX_COLUMNS_LIMIT` в `photo_grid.rs`).
- **Всё тяжёлое — вне главного потока:** сканирование в `std::thread`, текстуры через
  `gio::spawn_blocking`. Ячейки сетки переиспользуются — перед установкой текстуры
  проверять, что ячейка показывает тот же элемент (`set_thumbnail(.., still_valid)`).
- **Смена папки во время сканирования** отменяет текущее и перезапускает
  (`rescan_pending`). Во время сканирования UI перечитывает БД каждые 3 с
  (`reload(state, false)` сохраняет раскрытые узлы и выбор).
- **Никаких покадровых обработчиков** (`add_tick_callback` с `Continue`): они будят
  главный поток 60 раз в секунду даже в простое и на скрытой вкладке. За размером карты
  следит невидимая `DrawingArea` (сигнал `resize`) поверх неё.
- **Обновление карты (`MapTab::refresh`) идёт по всем фото на каждое движение**, поэтому
  в нём нет тригонометрии и копирования: проекция на зум 0 считается один раз в `reload`
  (`points`), объекты фото для списка создаются там же и передаются как есть
  (`PhotoGrid::set_objects`), маркеры хранятся по ключу кластера и пересоздаются только
  изменившиеся. Ориентир на 50 000 фото: без изменений на экране — доли миллисекунды.
- **Владение состоянием окна:** обработчики на виджетах, которыми владеет `App`, держат
  его только слабой ссылкой (`Rc::downgrade`); единственная сильная — в обработчике
  `close-request`, она отпускается при закрытии окна, и БД закрывается чисто.
- **Циклы ссылок в окнах:** GTK4 при закрытии окна не разрывает цикл «окно → дочерний
  виджет → обработчик → окно». Обработчики внутри диалога держат его через
  `downgrade()` — иначе диалог (и всё захваченное, например `on_save` с состоянием)
  остаётся в памяти после каждого открытия. Проверка: слабая ссылка на окно после `close()`.
- **Popover с `set_parent`** отвязывать в `connect_destroy` родителя (`unparent()`), иначе
  при закрытии GTK пишет «Finalizing … but it still has children left».
- Зум карты при автоподгонке округляется до целого — на дробном видны швы тайлов.
- **Моё местоположение:** портал геолокации, без новых зависимостей (`gio` D-Bus) и без
  прав Flatpak. Подписка на `Response` — до вызова `Start` (путь запроса известен заранее).
  В изолированной `dbus-run-session` портал отвечает «Location services disabled» —
  так проверяется ветка ошибки. Успешный путь в настоящей сессии покажет пользователю
  системный запрос доступа, поэтому без спроса его не запускать.
- **Иконки в меню** (главное меню и меню фото): `PopoverMenu` в GTK4 не рисует иконки
  обычных пунктов, поэтому пункты — свои кнопки через атрибут `custom` (`ui/menu.rs`,
  `icon_menu`). Сочетание клавиш в таком пункте рисуется отдельно (`Item::accel`).
- **Положение карты** (центр и зум) сохраняется в `[map_view]` конфига при закрытии окна
  и при запуске заменяет подгонку под все фото. Пока карта не показана (окно без размера),
  положение не сохраняется — иначе запомнилась бы нулевая точка.
- **Поиск местности (Nominatim):** правила сервиса — не чаще 1 запроса в секунду и
  User-Agent с названием программы; запрос только по Enter, без автодополнения.
  HTTP — через libsoup (`soup3` 0.8: версия должна использовать тот же `glib`, что и `gtk4`).

## Проверка интерфейса без участия человека

GTK-приложение уникально по ID: если у пользователя оно уже открыто, повторный запуск
просто активирует его окно. Для автотестов запускать в отдельной сессии D-Bus и с
отдельными XDG-каталогами, чтобы не трогать данные пользователя:

```sh
X=/tmp/gpl-test
mkdir -p $X/cfg/geo-photo-library
echo "library_path = \"$PWD/testdata/library\"" > $X/cfg/geo-photo-library/config.toml
XDG_DATA_HOME=$X/data XDG_CACHE_HOME=$X/cache XDG_CONFIG_HOME=$X/cfg \
  GEO_PHOTO_LIBRARY_SCREENSHOT=/tmp/shot.png GEO_PHOTO_LIBRARY_TAB=dates \
  dbus-run-session -- ./target/release/geo-photo-library
```

Переменные окружения:

| Переменная | Действие |
| --- | --- |
| `GEO_PHOTO_LIBRARY_TRACE=1` | печатать длительность операций (`[trace] …`) |
| `GEO_PHOTO_LIBRARY_SCREENSHOT=out.png` | через `GEO_PHOTO_LIBRARY_DELAY` с (по умолчанию 6) сохранить окно в PNG, закрыть его (как пользователь — сохраняется положение карты) и выйти; если открыт диалог — снимается он |
| `GEO_PHOTO_LIBRARY_ACTION=about` | при запуске выполнить пункт главного меню (`settings`, `about`) |
| `GEO_PHOTO_LIBRARY_TAB=dates` | открыть вкладку дат (по умолчанию — карта, она первая) |
| `GEO_PHOTO_LIBRARY_BENCH=1` | прогнать зумы карты 18…2 вокруг первого фото и выйти |
| `GEO_PHOTO_LIBRARY_DEBUG_SWITCH=<папка>` | через 3 с сменить папку библиотеки, как из настроек |
| `GEO_PHOTO_LIBRARY_INFO=1` | через 3 с выделить первое фото на открытой вкладке и открыть панель «Информация» |
| `GEO_PHOTO_LIBRARY_LOCATE=1` | через 2 с нажать «Моё местоположение» на карте |
| `GEO_PHOTO_LIBRARY_SEARCH=<запрос>` | через 2 с найти место на карте (одно найденное — переход, несколько — список) |
| `GEO_PHOTO_LIBRARY_DATE_FILTER=open\|<год>` | через 3 с открыть фильтр карты по датам (и выбрать год) |

Скан без GUI: `geo-photo-library --scan <папка>`.
Бенчмарк миниатюр: `cargo run --release --example bench_thumbs -- <папка> [N]`.

## Документация

- `README.md` (русский) и `README.en.md` (английский) держать согласованными: новая
  возможность — в оба сразу. Вверху каждого — ссылка на другой.
- Скриншоты в `docs/screenshots/` снимаются на синтетической `testdata/library` (не на
  галерее пользователя — репозиторий публичный), без панели «Информация» (в ней виден
  домашний путь), с чистым конфигом без `[map_view]` — карта показывает все фото:

  ```sh
  X=/tmp/gpl-shot; mkdir -p $X/cfg/geo-photo-library
  for l in ru_RU:ru en_US:en; do
    echo "library_path = \"$PWD/testdata/library\"" > $X/cfg/geo-photo-library/config.toml
    LANG=${l%:*}.UTF-8 XDG_DATA_HOME=$X/data XDG_CACHE_HOME=$X/cache XDG_CONFIG_HOME=$X/cfg \
      GEO_PHOTO_LIBRARY_SCREENSHOT=docs/screenshots/map-${l#*:}.png GEO_PHOTO_LIBRARY_DELAY=8 \
      dbus-run-session -- ./target/release/geo-photo-library
  done
  ```

  Первый запуск сканирует выборку; если к моменту снимка скан не закончился, повторить.

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
./packaging/build-rpm.sh   # → target/rpmbuild/RPMS/<архитектура>/geo-photo-library-<версия>.rpm
```

Скрипт берёт исходники из git (включая незакоммиченные изменения отслеживаемых файлов),
вендорит зависимости Cargo (сборка без сети) и запускает `rpmbuild`; при Rust из rustup —
с `--nodeps`. `%check` гоняет тесты и валидирует `.desktop`/metainfo. Установка требует
root — агент её не выполняет, а даёт пользователю команду
(`pkexec dnf install -y ./target/rpmbuild/RPMS/*/*.rpm`).

При выпуске версии синхронно менять: `version` в `Cargo.toml` (и запись пакета в
`Cargo.lock` — сборки идут с `--locked`), `Version`/`Release` и `%changelog` в spec,
`<releases>` в metainfo. В Cargo версия всегда трёхчастная (`1.1.0` для выпуска 1.1), в spec
и metainfo — как у выпуска (`1.1`, `1.1.1`). Версия приложения macOS и имя DMG берутся из `Cargo.toml`.

## GitHub Actions

`.github/workflows/ci.yml` — на push в `main`, теги `v*` и pull request, в контейнере
`fedora:45` с Rust из dnf: задача `test` (clippy с `-D warnings` и `cargo test`), затем
`rpm` (`packaging/build-rpm.sh`) — матрицей для x86_64 (`ubuntu-latest`, артефакт
`geo-photo-library-fc45-rpm`) и aarch64 (`ubuntu-24.04-arm`, артефакт
`geo-photo-library-fc45-aarch64-rpm`); сборка нативная, без эмуляции.
Независимо от них — задача `dmg` на `macos-latest`: тесты ядра без GTK и `macos/build-dmg.sh`
(артефакт `geo-photo-library-macos-dmg`, только arm64; подпись ad-hoc, без нотаризации —
для неё нужны сертификат Developer ID и секреты в репозитории).
На теге `v*` задача `release` (после `rpm` и `dmg`) проверяет, что тег равен версии из
`Cargo.toml` (`v1.1.2` ↔ `1.1.2`, `v1.1` ↔ `1.1.0`), и прикладывает оба RPM и DMG к релизу
GitHub: если релиз уже создан в интерфейсе — догружает файлы, если запушен только тег —
создаёт релиз с автоописанием. Выпуск: поднять версию (см. «Пакет RPM»), запушить в `main`,
затем создать релиз с тегом `v<версия>` на GitHub или `git tag v<версия> && git push origin v<версия>`.
Локально то же самое можно повторить через `podman run fedora:45`.

## Flatpak

```sh
./packaging/flatpak/build-flatpak.sh   # → target/flatpak/geo-photo-library.flatpak
flatpak install --user target/flatpak/geo-photo-library.flatpak
```

Манифест `packaging/flatpak/io.github.EvilX.GeoPhotoLibrary.yml`: рантайм GNOME 51 +
rust-stable. Скрипт, как и для RPM, готовит архив исходников из git и вендоренные
зависимости Cargo в `target/flatpak/` (сборка внутри flatpak-builder без сети;
`cargo-vendor.toml` подменяет `.cargo/config.toml`), ставит SDK в пользовательскую
установку (`--user`, без root) и делает бандл.

- В рантайме нет libshumate (собирается вместе с protobuf-c), а его libheif — без HEVC,
  поэтому собираются свои libde265 и libheif. При обновлении источников менять `sha256`.
- Права: `--filesystem=home` (библиотека где угодно в домашнем каталоге), сеть для
  тайлов OSM. Корзина, открытие файлов и диалоги — через порталы.
- В песочнице программы хоста не видны: выбор просмотрщика/редактора в настройках
  скрыт, «Редактирование» открывает системный выбор программы.
- Данные приложения в Flatpak — в `~/.var/app/io.github.EvilX.GeoPhotoLibrary/`, отдельно
  от версии из RPM.

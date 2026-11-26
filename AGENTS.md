# AGENTS.md

Инструкции для ИИ-агентов, работающих с репозиторием **GeoPhotoLibrary** —
каталогизатором фотографий с поиском снимков на карте по GPS из EXIF: ядро на Rust + SQLite,
интерфейс GTK4 с картой OSM на Linux и Windows и нативный SwiftUI + MapKit на macOS. Пользовательское описание — в `README.md` (английский, по умолчанию)
и `README.ru.md` (русский).

## Язык и стиль

- Комментарии, отладочный вывод (`eprintln!`, `[trace]`) и коммиты — **на русском**.
  Исключение — `Cargo.toml` и RPM spec (`packaging/geo-photo-library.spec`, включая `%changelog`):
  они попадают в метаданные пакетов, поэтому на английском.
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
| `src/paths.rs` | каталоги данных: XDG через GLib, на Windows — `%LOCALAPPDATA%`, без GTK — `~/Library` |
| `src/win.rs` | Windows (kernel32): языки интерфейса, формат дат, консоль для `--scan` |
| `build.rs` | на Windows вкомпоновывает в exe иконку и версию (`packaging/windows/*.rc`, `windres`) |
| `src/scanner.rs` | инкрементальное сканирование (size+mtime), rayon, пачки по 64, отмена через `AtomicBool` |
| `src/geotag.rs` | редактирование EXIF файла: поля тегов (`fields`, чтение kamadak-exif), запись координат и изменённых тегов (little_exif, `Changes`) — только JPEG, HEIC/HEIF, WebP; результат проверяется до замены файла |
| `src/metadata.rs` | EXIF (kamadak-exif): дата, GPS, ориентация, камера; подробности для панели «Информация» |
| `src/thumbs.rs` | квадратные миниатюры 320×320: JPEG через mozjpeg с IDCT-масштабом, HEIC — встроенная миниатюра, DNG — встроенное JPEG-превью |
| `src/dng.rs` | разбор IFD в DNG/TIFF и поиск JPEG-превью |
| `src/db.rs` | SQLite: схема, миграции (`PRAGMA user_version`), запросы |
| `src/geo.rs` | Web Mercator, видимая область, кластеризация по сетке |
| `src/location.rs` | текущее местоположение через портал `org.freedesktop.portal.Location` (GeoClue) по D-Bus |
| `src/geocode.rs` | поиск местности через Nominatim (libsoup): URL запроса, разбор ответа |
| `src/update.rs` | проверка обновлений: разбор ответа GitHub о последнем выпуске, сравнение версий (запрос делает интерфейс) |
| `src/trace.rs` | замеры времени (`GEO_PHOTO_LIBRARY_TRACE`) |
| `src/i18n.rs` | локализация en/ru/el: выбор языка по локали, строки, названия месяцев, формат дат |
| `src/ui/window.rs` | окно, вкладки, главное меню, фоновое сканирование, отладочные хуки |
| `src/ui/settings.rs` | диалог настроек (папка библиотеки) |
| `src/ui/about.rs` | окно «О программе» |
| `src/ui/date_tree.rs` | дерево год → месяц → день (`TreeListModel`) со счётчиками: вкладка дат и фильтр карты («Всё время» первой строкой) |
| `src/ui/dates_tab.rs` | дерево дат + сетка фото выбранного периода; над деревом — фильтр по координатам (`db::GeoFilter`: все, с координатами, без) |
| `src/ui/map_tab.rs` | карта libshumate, строка поиска местности, фильтр по датам (плавающая панель), маркеры-кластеры, список фото в видимой области |
| `src/ui/exif_dialog.rs` | окно «Редактирование EXIF»: вкладка «Координаты» (метка в центре карты, поля широты/долготы) и «EXIF» (все теги, поиск), подтверждение |
| `src/ui/place_search.rs` | поиск местности (Nominatim) и «Моё местоположение» — общие для вкладки карты и редактора EXIF |
| `src/ui/photo_grid.rs` | общая сетка миниатюр (`GridView`) и контекстное меню фото |
| `src/ui/menu.rs` | меню с иконками у пунктов (`icon_menu`) — главное и меню фото |
| `src/ui/info_panel.rs` | панель «Информация» справа: файл и EXIF выбранного фото (чтение в фоне) |
| `src/ui/update_banner.rs` | полоса «Доступна новая версия» над вкладками: запрос к GitHub (libsoup), ссылка на сайт |
| `src/ui/textures.rs` | асинхронная загрузка текстур, LRU-кэш 128 МБ |
| `macos/` | нативный интерфейс macOS: пакет SwiftPM (SwiftUI + MapKit), `build-app.sh`, `build-dmg.sh`, `Info.plist`, `AppIcon.svg` (иконка на весь квадрат, рисунок как в `data/icons`) |
| `examples/bench_thumbs.rs` | бенчмарк этапов генерации миниатюр |
| `packaging/` | `windows/` — `build-windows.sh` (MSYS2), установщик Inno Setup (`.iss`), иконка `.ico`; RPM spec и `build-rpm.sh`, `build-deb.sh` (DEB для Ubuntu), `build-repo.sh` (репозитории dnf/apt для GitHub Pages); `flatpak/` — манифест Flatpak и `build-flatpak.sh` |
| `data/` | иконка, `.desktop`, metainfo (ID `io.github.EvilX.GeoPhotoLibrary`) |
| `docs/screenshots/` | скриншоты для README: `map-ru.png`, `map-en.png` (Linux), `macos-*.png`, `windows-*.png` |

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
- **Liquid Glass:** macOS включает новый интерфейс, только если в бинарнике записана версия SDK 26+.
  Движок Swift Build (по умолчанию в SwiftPM с Xcode 27) пишет туда минимальную версию системы
  (14.0), поэтому `build-app.sh` передаёт компоновщику `-platform_version` с настоящей версией SDK.
  Проверка: `otool -l target/macos/GeoPhotoLibrary.app/Contents/MacOS/GeoPhotoLibrary | grep sdk`.
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
  `GEO_PHOTO_LIBRARY_PHOTO_ACTION=copy|show-on-map|trash|edit-exif` — пункт меню для первого фото вкладки
  (`trash` спрашивает подтверждение, как из меню). `GEO_PHOTO_LIBRARY_DATE_FILTER=open|<год>[-<месяц>[-<день>]]` —
  выбрать период и открыть фильтр карты по датам (дерево раскрывается до выбранной строки). `GEO_PHOTO_LIBRARY_APPEARANCE=dark|light` — тёмное
  или светлое оформление независимо от системного (аргумент `-AppleInterfaceStyle` не действует). `_LOCATE` покажет пользователю системный
  запрос доступа — без спроса не запускать. Язык: аргумент `-AppleLanguages "(ru)"`.
- **Панель «Информация» — не `.inspector`:** с Liquid Glass системный инспектор занимает правую
  часть заголовка окна, и вкладки по центру сдвигались влево. Панель — боковая часть `SplitPane`
  (`sideVisible`) под заголовком; основная часть при этом остаётся тем же видом.
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
- **Клавиатура в сетке** (`PhotoGrid`, `onKeyPress`): стрелки, Page Up/Down (на число видимых
  рядов), Home/End, с ⇧ — диапазон от опорного фото до `cursor`. Фокус сетка получает по щелчку.
  Карту клавиши не центрируют — как в GTK, где это всё делает сам `GridView`.
- **Фильтр карты по датам:** `AppModel.mapPeriod` (`nil` — всё время), фото карты —
  `gpl_db_geotagged(год, месяц, день)`. В дереве — только даты фото в видимой области карты:
  счётчики по всем фото с координатами (без учёта фильтра) пересчитываются при каждом
  обновлении карты (`gpl_visible_date_counts`, в GTK — `MapTab::refresh`). Выбранный период
  при этом не сбрасывается, даже если его узел пропал из дерева. Смена периода только перечитывает
  список: положение и масштаб карты не меняются (так решил пользователь — без подгонки).
  Панель плавает над картой (overlay) — её открытие не меняет размер карты.
  При закрытии панель убирается целиком, поэтому раскрытые узлы хранит модель (`mapExpanded`),
  а при открытии раскрываются год и месяц выбранного периода и список прокручивается к нему.
  В GTK то же делает `DateTree::set_counts`: предки выбранного узла раскрыты всегда.
- **Редактор EXIF** — лист `ExifEditor` (`AppModel.exifEdit`) с вкладками `TabView`; теги — `gpl_exif_fields`
  (JSON), проверка поля — `gpl_exif_check`, запись — `gpl_save_edit` в фоне; разбор координат — ядро
  (`gpl_parse_coord*`). `TabView` пересоздаёт карту при возврате на вкладку — начальный вид берётся из полей. Метка — вид поверх `MKMapView`; при смене размера листа
  MapKit сохраняет угол области, поэтому `resized` возвращает точку в центр. `PlaceFinder` переводит
  карту через `target` (`PlaceTarget`), ошибки местоположения — в своём `alert`: окно главной вкладки
  поверх листа не показывается.
- **Фильтр вкладки дат по координатам** (`AppModel.dateFilter`, код — `GeoFilter::index`) передаётся
  в `gpl_db_date_counts` и `gpl_db_photos_by_date`; не сохраняется — при запуске «Все фото».
- **Меню «Правка»** — действия с выбранными фото, без пунктов правки текста; ⌘X/⌘V/⌘Z, а в поле
  ввода и ⌘C/⌘⌫/⌘A отдаёт строке поиска `TextEditKeys` (локальный монитор клавиш).
- **Выделение нескольких фото** (`PhotoSelection` в `AppModel`): ⌘/⇧ + щелчок и рамка мышью
  (`PhotoGrid`). LazyVGrid создаёт только видимые ячейки, поэтому рамка находит фото не по
  их кадрам, а по колонкам и высоте ячейки. Над несколькими фото — только копирование и удаление,
  панель «Информация» пуста. Щелчок с ⌘/⇧ и рамка карту не центрируют (`onPick` — только обычный щелчок).
- Изменения ядра проверять и для GTK: на Mac — в контейнере
  (`docker run --rm -v "$PWD":/src:ro -e CARGO_TARGET_DIR=/target fedora:45 …`, зависимости как в CI).

## Windows

Тот же интерфейс GTK, собирается в MSYS2: UCRT64 (x64) или CLANGARM64 (ARM64), пакеты — в
шапке `packaging/windows/build-windows.sh`. Скрипт собирает exe, кладёт рядом DLL, модули GIO
(TLS — `libgiognutls`), загрузчики gdk-pixbuf, схемы GSettings, темы значков, `gdbus.exe` и делает
ZIP и установщик Inno Setup (6 или 7) → `target/windows/GeoPhotoLibrary-<версия>-<арх>-setup.exe`.

- `.cargo/config.toml` задаёт `PKG_CONFIG=/usr/bin/pkg-config` для Fedora; в MSYS2 его перекрывает
  `PKG_CONFIG=$MINGW_PREFIX/bin/pkgconf.exe` (скрипт ставит сам).
- **DLL — через `ntldd -R`, не `ldd`:** на ARM64 `ldd` MSYS2 работает под эмуляцией x64 и теряет
  зависимости (так пропадал `libgnutls` — без него нет тайлов OSM). Фильтр по выводу — одним `awk`:
  запуск процесса на строку в MSYS2 медленный (было 10 минут вместо 30 с).
- Пути: `canonicalize()` даёт `\\?\C:\…` — сканер отрезает префикс (`scanner::strip_verbatim`), иначе он
  виден в панели «Информация» и в буфере обмена.
- Язык — `GetUserPreferredUILanguages`, если не заданы `LANG`/`LC_*`; даты — `GetLocaleInfoEx`
  (короткие форматы) → strftime GLib (`win::convert_pattern`). Кнопки «Моё местоположение» нет
  (портал D-Bus). `DesktopAppInfo` только для Unix: программы ищутся среди `gio::AppInfo::all()`,
  редактор по категориям не ищется — сразу окно выбора.
- Данные — `%LOCALAPPDATA%\geo-photo-library\` (кэш GLib на Windows — `INetCache`, туда не класть);
  `XDG_*_HOME` GLib учитывает и на Windows — так изолируются тесты.
- exe с `windows_subsystem = "windows"`: без консоли; `--scan` подключается к консоли родителя.
- Иконка `packaging/windows/geo-photo-library.ico` сделана из `data/icons/…svg` (rsvg-convert + magick,
  16…256) — менять вместе с ней.
- Проверка на ВМ: GUI из сессии SSH не виден — запускать через `schtasks /create … /it` в сессии
  вошедшего пользователя, с `GEO_PHOTO_LIBRARY_SCREENSHOT` и отдельными `XDG_*_HOME`.
  Установщик ставится тихо: `setup.exe /VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER`.
- CI: задача `windows` (матрица `windows-latest` UCRT64 и `windows-11-arm` CLANGARM64, `msys2/setup-msys2`),
  артефакты `geo-photo-library-windows-<x64|arm64>`, на теге — в релиз.

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
- **Выделение фото — `MultiSelection`** с рамкой (`set_enable_rubberband`): Ctrl/Shift + щелчок,
  Ctrl+A и Shift + стрелки даёт GridView. Удаление выделенных фото из списка `MultiSelection`
  сообщает только `items-changed`, без `selection-changed` — `PhotoGrid::connect_selection_changed`
  слушает оба. Карта центрируется только по обычному щелчку (`connect_picked`), не по смене
  выделения: иначе рамка, задев одно фото, сдвигала бы карту и список под ней. Меню над
  несколькими выделенными — только «Копировать» и «Удалить»; перемещение в корзину сообщает
  окну все результаты разом (`TrashResults`), БД перечитывается один раз.
- **Редактирование EXIF** пишет в сами файлы (`geotag::write`, `Changes`: координаты и изменённые теги):
  little_exif пересобирает блок EXIF, поэтому новый файл собирается в памяти, проверяется (координаты и
  изменённые теги читаются kamadak-exif, ни один прежний тег, кроме GPS и удалённых, не пропал) и атомарно заменяет старый (временный `.<имя>.gpl-tmp`
  рядом). PNG (little_exif пишет EXIF в текстовый блок, сканер его не видит) и DNG/TIFF (данные
  переносятся внутри файла) не поддерживаются. Поле тега идентифицирует ключ `блок:номер:формат`
  (`exif:9003:2`); формат — little_exif для известных ему тегов (значение он приводит к нему), иначе из
  файла. Пишутся только изменённые поля, и у пачки фото остальные теги каждого не трогаются;
  тег с разными значениями — пустое поле с подсказкой. Не редактируются служебные теги (смещения
  блоков, миниатюра, MakerNote), байтовые (`undefined`) и массивы длиннее 64 значений; координаты
  GPS — только на своей вкладке. После записи файл перечитывается как сканером
  (`geotag::apply` → `scanner::process_file`, миниатюра заново) и запись в БД обновляется
  (`Db::update`): дата, ориентация, размер и mtime — сканер не обработает файл повторно; карту перечитывать с `force`
  (GTK `reload(state, true)`, macOS `resetMarkers`): маркеры хранятся по составу кластера и
  остались бы на старом месте. В окне метка закреплена в центре карты, точка — центр карты;
  сдвиг центра, совпадающий с полями (карту двигает ввод), поля не трогает. Окно держит состояние
  сильной ссылкой в `connect_destroy` — без неё обработчики со слабыми ссылками перестают работать
  сразу после `show()`.
- **Иконки в меню** (главное меню и меню фото): `PopoverMenu` в GTK4 не рисует иконки
  обычных пунктов, поэтому пункты — свои кнопки через атрибут `custom` (`ui/menu.rs`,
  `icon_menu`). Сочетание клавиш в таком пункте рисуется отдельно (`Item::accel`).
- **Положение карты** (центр и зум) сохраняется в `[map_view]` конфига при закрытии окна
  и при запуске заменяет подгонку под все фото. Пока карта не показана (окно без размера),
  положение не сохраняется — иначе запомнилась бы нулевая точка.
- **Проверка обновлений** (`check_updates` в конфиге, по умолчанию включена): при запуске
  `releases/latest` GitHub; ошибки сети — только в stderr, полоса над вкладками появляется,
  лишь если выпуск новее `CARGO_PKG_VERSION`. Ссылка — на сайт (`update::SITE_URL`), не на
  файл: там выбор пакета под систему. На macOS запрос — `URLSession`, разбор — ядро (`gpl_update_newer`).
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
| `GEO_PHOTO_LIBRARY_UPDATE=<версия>` | считать её последним выпуском вместо запроса к GitHub (баннер — если новее текущей; и на macOS) |
| `GEO_PHOTO_LIBRARY_SELECT=<n>` | через 4 с выделить первые n фото на открытой вкладке (и на macOS) |
| `GEO_PHOTO_LIBRARY_EDIT_EXIF=1\|exif` | через 5 с открыть редактор EXIF для выделенных фото (без выделения — для первого; `exif` — сразу на вкладке «EXIF»; и на macOS) |
| `GEO_PHOTO_LIBRARY_GEO_FILTER=all\|with\|without` | фильтр вкладки дат по координатам при запуске (и на macOS) |

Скан без GUI: `geo-photo-library --scan <папка>`.
Бенчмарк миниатюр: `cargo run --release --example bench_thumbs -- <папка> [N]`.

## Документация

- `README.md` (английский) и `README.ru.md` (русский) держать согласованными: новая
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
- Скриншоты macOS (`macos-<язык>.png`) — так же, на синтетической выборке, светлое оформление,
  окно по умолчанию; снимок Retina уменьшить вдвое (`sips --resampleWidth 1427`):

  ```sh
  X=/tmp/gpl-mac; mkdir -p $X/config/geo-photo-library
  for l in ru en; do
    echo "library_path = \"$PWD/testdata/library\"" > $X/config/geo-photo-library/config.toml
    GEO_PHOTO_LIBRARY_HOME=$X GEO_PHOTO_LIBRARY_APPEARANCE=light GEO_PHOTO_LIBRARY_SCREENSHOT=$X/$l.png \
      GEO_PHOTO_LIBRARY_DELAY=30 target/macos/GeoPhotoLibrary.app/Contents/MacOS/GeoPhotoLibrary -AppleLanguages "($l)"
    sips --resampleWidth 1427 $X/$l.png --out docs/screenshots/macos-$l.png
  done
  ```

## Тестовые данные

`testdata/` в `.gitignore`: `testdata/library` — 1000 синтетических 12-Мп снимков
(100 мест × 10, 17% HEIC со встроенными миниатюрами) с подставленным EXIF. Генератор
в репозиторий не входит. Реальную галерею пользователя (`~/OneDrive/Pictures`)
только читать и сканировать в отдельные XDG-каталоги.

Ориентиры производительности на этой выборке (i5-10210U, 8 потоков): первое
сканирование ~18 с, повторное ~0,1 с; JPEG 12 Мп ~45 мс, HEIC со встроенной миниатюрой
~12 мс на файл.

## Пакет DEB (Ubuntu 24.04, 26.04)

`./packaging/build-deb.sh` → `target/deb/geo-photo-library_<версия>-1~ubuntu<выпуск>_<архитектура>.deb`.
Скрипт собирает бинарник из текущего дерева (`cargo build/test --locked`), раскладывает файлы
`data/`, проверяет `.desktop` и metainfo, зависимости библиотек считает `dpkg-shlibdeps`
и собирает пакет `dpkg-deb` (без debhelper). Собирать в системе того же выпуска, для
которого пакет (CI — контейнеры `ubuntu:24.04`, `ubuntu:26.04`). Rust в Ubuntu 24.04 (1.75)
слишком старый — нужен rustup.

- **libheif:** `libheif-rs` подключён с `default-features = false, features = ["v1_17"]` — по
  умолчанию он требует новейшую libheif (1.21), а в Ubuntu 24.04 — 1.17. Новые функции libheif
  в коде — только вместе с поднятием этой планки (и без Ubuntu 24.04).
- Декодер HEVC в Ubuntu — отдельный пакет `libheif-plugin-libde265`, он в `Recommends`.
- `build-deb.sh` удаляет старые `target/deb/*.deb` перед сборкой: в CI `target/` восстанавливается
  из кэша целиком (исключение `!target/deb` в `actions/cache` не действует на подкаталог), и
  иначе в артефакт и релиз попадали пакеты всех прошлых версий.
- Проверка без человека: в контейнере `ubuntu:<выпуск>` поставить пакет и `xvfb`, запустить
  с `GEO_PHOTO_LIBRARY_SCREENSHOT` под `dbus-run-session` (как в разделе о проверке интерфейса).

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
`geo-photo-library-fc45-aarch64-rpm`); сборка нативная, без эмуляции. После `test` — и `deb`
(`packaging/build-deb.sh`) матрицей Ubuntu 24.04/26.04 × amd64/arm64 в контейнерах `ubuntu:<выпуск>`
с Rust из rustup; пакет ставится в тот же контейнер (`apt-get install ./…deb`), артефакты
`geo-photo-library-ubuntu<выпуск>-<архитектура>-deb`.
Независимо от них — задача `windows` (см. «Windows») и задача `dmg` на `macos-latest`: тесты ядра без GTK и `macos/build-dmg.sh`
(артефакт `geo-photo-library-macos-dmg`, только arm64; подпись ad-hoc, без нотаризации —
для неё нужны сертификат Developer ID и секреты в репозитории).
На теге `v*` задача `release` (после `rpm`, `deb`, `dmg` и `windows`) проверяет, что тег равен версии из
`Cargo.toml` (`v1.1.2` ↔ `1.1.2`, `v1.1` ↔ `1.1.0`), и прикладывает RPM, DEB, DMG и установщики Windows (с ZIP) к релизу
GitHub: если релиз уже создан в интерфейсе — догружает файлы, если запушен только тег —
создаёт релиз с автоописанием. Затем задача `repo` обновляет репозитории пакетов (см. ниже). Выпуск: поднять версию (см. «Пакет RPM»), запушить в `main`,
затем создать релиз с тегом `v<версия>` на GitHub или `git tag v<версия> && git push origin v<версия>`.
Локально то же самое можно повторить через `podman run fedora:45`.

## Репозитории dnf и apt (GitHub Pages)

На теге `v*` после `release` задача `repo` кладёт RPM и DEB в ветку `gh-pages`
(сайт `https://evilx.github.io/geo-photo-library/`) скриптом `packaging/build-repo.sh <пакеты> <сайт>`:
`rpm/fedora/<выпуск>/<архитектура>/` с метаданными `createrepo_c`, `deb/pool/<кодовое имя>/` и
`deb/dists/<кодовое имя>/` (`apt-ftparchive`), файл `geo-photo-library.repo`, ключи `key.asc`/`key.gpg`
и `index.html` — лендинг проекта из `packaging/site/index.html` (тёмная/светлая тема по системной,
языки en/ru/el — по браузеру, переключатель в шапке, `?lang=ru`; английский текст — в разметке, ru и el — в словаре `T`
в скрипте страницы, менять все три сразу; скриншоты `macos-*`/`map-*`/`windows-*` для en и ru (у греческого — английские) и иконка копируются из репозитория, `@URL@` заменяется адресом сайта;
версию и прямую ссылку на DMG страница берёт из API GitHub) и `sitemap.xml` из `packaging/site/sitemap.xml` (три языка — `?lang=` с `hreflang`, `@DATE@` — дата выпуска; новый язык лендинга — добавить и туда). Лендинг обновляется только с выпуском. Скрипт берёт уже лежащие на сайте пакеты, добавляет новые и оставляет
последние `KEEP` (3) версий на выпуск и архитектуру; ветка `gh-pages` каждый раз — один коммит
(`push --force`), иначе удалённые пакеты копились бы в истории.

- Подпись: секрет `GPG_PRIVATE_KEY` — закрытый ключ без пароля в ASCII. Подписываются сами RPM
  (`rpmsign`, `gpgcheck=1`), `repomd.xml` (`repo_gpgcheck=1`) и `Release`/`InRelease` для apt.
  Сменить ключ — значит заставить всех пользователей заново импортировать `key.asc`/`key.gpg`.
- Новый выпуск Ubuntu — добавить его кодовое имя в `codename()` в `build-repo.sh` (24.04 noble,
  26.04 resolute); новый выпуск Fedora подхватывается сам по `.fc<N>` в имени RPM.
- Pages включён в настройках репозитория: источник — ветка `gh-pages`, корень.
- Проверка без GitHub: прогнать скрипт в `ubuntu:24.04` (пакеты `createrepo-c apt-utils gnupg rpm`)
  с одноразовым ключом, затем поставить пакет из `file://` в контейнерах `fedora:45` и `ubuntu:24.04`.

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

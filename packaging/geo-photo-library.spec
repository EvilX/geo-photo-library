# Release-сборка Rust без отладочной информации — debuginfo-пакет был бы пустым.
%global debug_package %{nil}
%global app_id io.github.EvilX.GeoPhotoLibrary

Name:           geo-photo-library
Version:        1.3
Release:        1%{?dist}
Summary:        Photo catalog with map search by EXIF GPS coordinates
License:        Apache-2.0
URL:            https://github.com/EvilX/geo-photo-library
Source0:        %{name}-%{version}.tar.gz
# Зависимости Cargo (cargo vendor), чтобы собирать без сети.
Source1:        %{name}-%{version}-vendor.tar.xz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4)
BuildRequires:  pkgconfig(shumate-1.0)
BuildRequires:  pkgconfig(libheif)
BuildRequires:  pkgconfig(libsoup-3.0)
BuildRequires:  desktop-file-utils
BuildRequires:  appstream
Requires:       hicolor-icon-theme

%description
GeoPhotoLibrary indexes a folder of photos (JPEG, HEIC, PNG, WebP, TIFF),
shows them in a year/month/day tree and places them on an OpenStreetMap map
using GPS coordinates from EXIF metadata.

%prep
%autosetup -n %{name}-%{version} -a 1
cat >> .cargo/config.toml <<'CFG'

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CFG

%build
cargo build --release --offline --locked

%install
install -Dm0755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dm0644 data/%{app_id}.desktop %{buildroot}%{_datadir}/applications/%{app_id}.desktop
install -Dm0644 data/%{app_id}.metainfo.xml %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
install -Dm0644 data/icons/%{app_id}.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{app_id}.desktop
appstreamcli validate --no-net %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
cargo test --release --offline --locked

%files
%license LICENSE
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{app_id}.desktop
%{_metainfodir}/%{app_id}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg

%changelog
* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3-1
- Месяцы в дереве дат — от новых к старым (декабрь сверху), как годы

* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2.3-1
- macOS: интерфейс Liquid Glass и в локальной сборке (версия SDK в бинарнике)
- macOS: панель «Информация» не сдвигает вкладки в заголовке окна
- Для Linux изменений нет

* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2.2-1
- Выделение нескольких фото: Ctrl/⌘ + щелчок, Shift + щелчок, рамка мышью, выделить все;
  над несколькими фото — копирование и удаление, панель «Информация» пуста
- Подтверждение перед перемещением фото в корзину

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2.1-1
- Выбор периода в фильтре карты по датам не меняет положение и масштаб карты

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2-1
- Фильтр карты по датам: плавающая панель «Всё время» и дерево год → месяц → день
  с датами фото в видимой области карты (Linux и macOS)
- Общее дерево дат для вкладки «По датам» и фильтра карты (GTK)

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.6-1
- macOS: годы в дереве дат — от новых к старым, как в версии GTK

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.5-1
- Стопка фотографий на иконке крупнее и выше
- macOS: меню «Правка» — действия с выбранным фото (копировать, удалить,
  просмотр, редактирование, показать в Finder)
- macOS: заголовок у панели «Информация»

* Tue Nov 17 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.4-1
- macOS: в меню «Правка» только нужные пункты, убрано меню справки
- В релиз попадает только DMG текущей версии

* Tue Nov 17 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.3-1
- Лицензия Apache 2.0
- Пакеты RPM и образ DMG прикладываются к релизам GitHub
- macOS: дерево дат в стиле боковой панели Finder, слайдер масштаба без отметок,
  тёмный вариант иконки

* Mon Nov 16 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.2-1
- Название программы в одно слово: GeoPhotoLibrary
- Пункт «Показать на диске» в меню фото: файловый менеджер открывает папку и выделяет файл

* Mon Nov 16 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.1-1
- Версия для macOS: кнопки «Пересканировать» и «Информация» — у правого края заголовка окна
- Версия для macOS: выбор сортировки над сеткой больше не сжимается до одной буквы
- Для Linux изменений нет

* Mon Nov 16 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1-1
- Ядро (сканер, база, миниатюры, EXIF) выделено в библиотеку, интерфейс GTK — её клиент
- Нативная версия для macOS на том же ядре (SwiftUI + MapKit), распространяется отдельно (DMG)
- Для Linux возможности и формат данных не изменились

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.0.1-1
- Исправлено определение местоположения (кнопка «Моё местоположение»)
- Исправлена утечка памяти в диалоге настроек
- Убраны предупреждения GTK при закрытии окна

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.0-1
- Даты под фото — в формате системной локали
- Ускорение карты на больших библиотеках, программа не тратит CPU в простое
- Исправлена утечка памяти при закрытии окна

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.7-1
- Кнопка «Моё местоположение» на карте
- Кнопки «−»/«+» у слайдера масштаба миниатюр
- Иконки в главном меню

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.6-1
- Кнопка «i» в заголовке открывает и закрывает панель «Информация»
- Положение и масштаб карты запоминаются между запусками
- Пункт «Найти на карте» в меню фото
- Иконки у пунктов меню фото

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.5-1
- Панель «Информация» с данными файла и EXIF выбранного фото

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.4-1
- Поиск местности на карте (геокодер Nominatim)
- CI: тесты и сборка RPM для Fedora 45 в GitHub Actions

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.3-1
- Локализация интерфейса: английский, русский, греческий
- Вкладка «Карта» открывается первой
- Сортировка фото и масштаб миниатюр 50…200%
- Главное меню: настройки и окно «О программе»
- Контекстное меню фото: копирование, просмотр, редактирование, удаление в корзину
- Выбор просмотрщика и редактора в настройках
- Сборка Flatpak

* Sat Nov 14 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.2-1
- Ускорение сканирования: масштабированный JPEG, встроенные миниатюры HEIC
- Асинхронная загрузка миниатюр в интерфейсе
- Смена папки во время сканирования отменяет его и запускает заново
- Дерево дат и карта обновляются по ходу длинного сканирования
- Миниатюры для DNG (iPhone ProRAW)

* Sat Nov 14 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.1-1
- Первый выпуск

# Release-сборка Rust без отладочной информации — debuginfo-пакет был бы пустым.
%global debug_package %{nil}
%global app_id io.github.EvilX.GeoPhotoLibrary

Name:           geo-photo-library
Version:        0.2
Release:        1%{?dist}
Summary:        Photo catalog with map search by EXIF GPS coordinates
# TODO: лицензия проекта не выбрана.
License:        LicenseRef-Unspecified
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
BuildRequires:  desktop-file-utils
BuildRequires:  appstream
Requires:       hicolor-icon-theme

%description
Geo photo library indexes a folder of photos (JPEG, HEIC, PNG, WebP, TIFF),
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
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{app_id}.desktop
%{_metainfodir}/%{app_id}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg

%changelog
* Sat Nov 14 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.2-1
- Ускорение сканирования: масштабированный JPEG, встроенные миниатюры HEIC
- Асинхронная загрузка миниатюр в интерфейсе
- Смена папки во время сканирования отменяет его и запускает заново
- Дерево дат и карта обновляются по ходу длинного сканирования
- Миниатюры для DNG (iPhone ProRAW)

* Sat Nov 14 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.1-1
- Первый выпуск

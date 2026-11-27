# Rust release build without debug info: the debuginfo package would be empty.
%global debug_package %{nil}
%global app_id io.github.EvilX.GeoPhotoLibrary

Name:           geo-photo-library
Version:        1.4.1
Release:        1%{?dist}
Summary:        Photo catalog with map search by EXIF GPS coordinates
License:        Apache-2.0
URL:            https://github.com/EvilX/geo-photo-library
Source0:        %{name}-%{version}.tar.gz
# Cargo dependencies (cargo vendor), to build without network access.
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
# Video thumbnails (a frame from the start of the video).
Recommends:     /usr/bin/ffmpeg

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
* Fri Nov 27 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.4.1-1
- The scanner no longer drops records when a folder cannot be read or the library mount point
  is empty; missing thumbnails are recreated
- EXIF writes: progress, no concurrent writes to the same files, symlinks are followed,
  failures are reported as "saved N, failed M"
- The date tab keeps the selection and scroll position on reload; collapsing a year in the map
  date filter keeps the period; "Show on map" resets the filter when the photo is outside it
- Keyboard in the photo grid: Delete, Ctrl+C, Menu/Shift+F10, F5
- The EXIF editor highlights an invalid coordinate and warns about files it cannot write
- Thumbnails that are no longer needed are not decoded while scrolling; nothing is reloaded
  after a scan that changed nothing

* Fri Nov 27 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.4.0-1
- Videos (MP4, MOV, M4V, 3GP, MKV, WebM, AVI, MTS) in the library: shown by date with a frame
  thumbnail and duration; they have no coordinates and open in the default player
- The filter above the date tree gets a "Videos" item; "With/Without coordinates" show photos only
- Video thumbnails are made with ffmpeg (recommended dependency)

* Thu Nov 26 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.9-1
- EXIF editing works with files where a tag has a signed instead of an unsigned type
  (for example ExifImageWidth in some older cameras)

* Thu Nov 26 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.8-1
- "Edit EXIF" window: a new EXIF tab to edit all tags, with a filter by tag name or value;
  only changed tags are written, also for several photos
- After editing, the photo date, orientation and thumbnail are updated without a rescan

* Thu Nov 26 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.7-1
- Edit photo coordinates on a map; they are written into the file EXIF (JPEG, HEIC, WebP)
- "By date" tab: filter photos with or without coordinates

* Wed Nov 25 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.6-1
- Check for a new version at startup (can be turned off in Settings)

* Wed Nov 25 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.5-1
- Windows version (x64 and ARM64): installer and portable version

* Wed Nov 25 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.4-1
- Fixed "database is locked" on the first scan of a new library

* Tue Nov 24 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.3-1
- macOS: keyboard navigation in the photo list (arrows, Page Up/Down, Home/End, Shift)

* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.2-1
- dnf and apt package repositories on GitHub Pages

* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3.1-1
- Map date filter: when reopened, the tree is expanded down to the selected row

* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.3-1
- Months in the date tree go from newest to oldest (December on top), like years

* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2.3-1
- macOS: Liquid Glass interface in local builds too (SDK version in the binary)
- macOS: the Information panel no longer shifts the tabs in the window title bar
- No changes for Linux

* Thu Nov 19 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2.2-1
- Selecting several photos: Ctrl/⌘ + click, Shift + click, rubber band, select all;
  for several photos — copy and delete, the Information panel is empty
- Confirmation before moving photos to the trash

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2.1-1
- Choosing a period in the map date filter keeps the map position and zoom

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.2-1
- Map date filter: a floating panel with "All time" and a year → month → day tree
  with the dates of photos in the visible map area (Linux and macOS)
- Shared date tree for the "By date" tab and the map filter (GTK)

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.6-1
- macOS: years in the date tree go from newest to oldest, as in the GTK version

* Wed Nov 18 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.5-1
- Larger and higher photo stack on the icon
- macOS: the Edit menu acts on the selected photo (copy, delete,
  view, edit, show in Finder)
- macOS: title for the Information panel

* Tue Nov 17 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.4-1
- macOS: only relevant items in the Edit menu, Help menu removed
- Only the DMG of the current version goes into the release

* Tue Nov 17 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.3-1
- Apache 2.0 license
- RPM packages and the DMG image are attached to GitHub releases
- macOS: Finder-style sidebar date tree, zoom slider without tick marks,
  dark icon variant

* Mon Nov 16 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.2-1
- The app name is one word: GeoPhotoLibrary
- "Show in folder" in the photo menu: the file manager opens the folder and selects the file

* Mon Nov 16 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1.1-1
- macOS version: the Rescan and Information buttons are at the right edge of the title bar
- macOS version: the sort picker above the grid no longer shrinks to one letter
- No changes for Linux

* Mon Nov 16 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.1-1
- The core (scanner, database, thumbnails, EXIF) is a library; the GTK interface is its client
- Native macOS version on the same core (SwiftUI + MapKit), distributed separately (DMG)
- No feature or data format changes for Linux

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.0.1-1
- Fixed location lookup (the "My location" button)
- Fixed a memory leak in the settings dialog
- Removed GTK warnings when closing the window

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 1.0-1
- Photo dates use the system locale format
- Faster map on large libraries; no CPU use when idle
- Fixed a memory leak when closing the window

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.7-1
- "My location" button on the map
- "−"/"+" buttons next to the thumbnail size slider
- Icons in the main menu

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.6-1
- The "i" button in the title bar opens and closes the Information panel
- Map position and zoom are remembered between launches
- "Show on map" in the photo menu
- Icons in the photo menu

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.5-1
- Information panel with file and EXIF data of the selected photo

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.4-1
- Place search on the map (Nominatim geocoder)
- CI: tests and RPM builds for Fedora 45 on GitHub Actions

* Sun Nov 15 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.3-1
- Interface localization: English, Russian, Greek
- The Map tab opens first
- Photo sorting and thumbnail size 50…200%
- Main menu: settings and the About window
- Photo context menu: copy, view, edit, move to trash
- Choice of viewer and editor in settings
- Flatpak build

* Sat Nov 14 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.2-1
- Faster scanning: scaled JPEG decoding, embedded HEIC thumbnails
- Asynchronous thumbnail loading in the interface
- Changing the folder during a scan cancels it and starts over
- The date tree and map update during a long scan
- Thumbnails for DNG (iPhone ProRAW)

* Sat Nov 14 2026 Leonid Nasedkin <leonidnasedkin@gmail.com> - 0.1-1
- First release

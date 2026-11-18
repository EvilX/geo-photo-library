# <img src="data/icons/io.github.EvilX.GeoPhotoLibrary.svg" width="48" height="48" align="center" alt=""> GeoPhotoLibrary

[Русский](README.md) | **English**

[![Latest release](https://img.shields.io/github/v/release/EvilX/geo-photo-library)](https://github.com/EvilX/geo-photo-library/releases/latest)

A photo catalog for Linux and macOS that places your photos on a map using GPS coordinates
from EXIF. The Rust core (scanner, SQLite database, thumbnails) is shared; the interface is
GTK4 with an OpenStreetMap map on Linux and a native SwiftUI one with a MapKit map on macOS
(see [Building (macOS)](#building-macos)).

The macOS version was added in 1.1; nothing has changed feature-wise on Linux since 1.0.

![Map tab: photo previews on a world map and the photos in the visible area](docs/screenshots/map-en.png)

## Features

The description below is for Linux; the differences of the macOS version are listed in
[Building (macOS)](#building-macos).

- **Map** — photo previews on the map, clustered; on the right, the photos in the visible area
  sorted by date (newest or oldest first, the choice is remembered). The map position and zoom
  are remembered between launches.
- **Place search** — a search box above the map: type a city, address or landmark and press
  Enter (Nominatim geocoder, requires internet). Several matches are shown as a list.
- **My location** — a button next to the search box: the map jumps to your current location and
  marks it with a dot (via the location portal; location services must be enabled in the system
  settings).
- **Date filter** — the calendar button left of the search box opens a panel over the map with
  "All time" and a year → month → day tree; the map keeps only the photos of the chosen period
  while its position and zoom stay as they are. The chosen period is shown on the button; a click
  on the map closes the panel.
- **By date** — a year → month → day tree and a thumbnail grid.
- **Information** — a panel on the right with file and EXIF data: camera, lens, shutter speed,
  aperture, ISO, focal length, coordinates, altitude and all tags. It follows the selected photo.
  Open it with the “i” button in the window header or from the photo menu; close it with the same
  button or the cross.
- **Photo menu** (right click): copy the file, view, edit, show in folder (the file manager opens
  the folder with the file selected), information, show on map (jumps to
  where the photo was taken; warns if EXIF has no coordinates), move to trash.
- Double click opens the photo in a viewer, Ctrl + double click in an editor. Both are chosen
  in Settings (by default, the app for the file type and a detected image editor).
- A slider under the photo grids scales thumbnails from 50 to 200%, with −/+ buttons (10% steps).
- On startup the library is rescanned incrementally (new, changed and deleted files).
- Formats: JPEG, HEIC/HEIF (iPhone, Samsung), PNG, WebP, TIFF, DNG (metadata only).
- Interface languages: English, Russian, Greek — from the system locale (`LANG`, `LC_MESSAGES`,
  `LANGUAGE`), English by default. Photo dates follow the system date format (`LC_TIME`),
  e.g. `12/21/2025 01:03 PM` in en_US and `21/12/25 13:03` in en_GB.

## Installation

Ready-made packages are in the [latest release](https://github.com/EvilX/geo-photo-library/releases/latest):
RPMs for Fedora 45 (x86_64 and aarch64), DEBs for Ubuntu 24.04 and 26.04 (amd64 and arm64) and
a disk image for macOS.

```sh
sudo dnf install ./geo-photo-library-*.fc45.*.rpm                # Fedora
sudo apt install ./geo-photo-library_*~ubuntu24.04_amd64.deb     # Ubuntu: your release and architecture
```

Builds of the current `main` are made by GitHub Actions on every push: **Actions** tab → latest
**CI** run → the `geo-photo-library-fc45-rpm` (x86_64), `geo-photo-library-fc45-aarch64-rpm` (ARM),
`geo-photo-library-ubuntu<release>-<arch>-deb` or `geo-photo-library-macos-dmg` artifact (downloading artifacts requires signing in to GitHub).

The macOS disk image is for Apple silicon, macOS 14 or later. The app is not signed with
a developer certificate and not notarized, so after dragging it to Applications remove the quarantine flag,
otherwise macOS will refuse to open it:

```sh
xattr -dr com.apple.quarantine "/Applications/GeoPhotoLibrary.app"
```

## Building (Fedora)

```sh
sudo dnf install gcc gtk4-devel libshumate-devel libheif-devel
cargo run --release
```

Without HEIC: `cargo build --release --no-default-features`.

Scan without the GUI: `geo-photo-library --scan <folder>`.

Packages:

- `./packaging/build-rpm.sh` — RPM in `target/rpmbuild/RPMS/<arch>/`;
- `./packaging/build-deb.sh` — DEB in `target/deb/` (on Ubuntu: needs `libgtk-4-dev libshumate-dev
  libsoup-3.0-dev libheif-dev dpkg-dev desktop-file-utils appstream` and Rust from rustup);
- `./packaging/flatpak/build-flatpak.sh` — Flatpak bundle `target/flatpak/geo-photo-library.flatpak`
  (the SDK goes into the user installation, no root needed).

## Building (macOS)

A native app: the Rust core (scanner, database, thumbnails) is built as a static library, the
interface is SwiftUI with a MapKit map (`macos/`). Requires macOS 14 or later, Xcode with its
license accepted (`sudo xcodebuild -license accept`) and Rust (rustup); GTK and libheif are not
needed — HEIC is decoded by the system.

```sh
./macos/build-app.sh --run   # → target/macos/GeoPhotoLibrary.app
./macos/build-dmg.sh         # → target/macos/GeoPhotoLibrary-<version>-arm64.dmg
```

The features are the same as on Linux, with the differences customary on macOS:

- the map and place search are Apple's (MapKit) instead of OpenStreetMap and Nominatim;
- "My location" uses CoreLocation; the system asks for permission on first use;
- the Edit menu acts on the selected photo: copy (⌘C), delete (⌘⌫), view (⌘O), edit (⌘E),
  show in Finder;
- the Information panel — the "i" button in the title bar or ⌘I; rescan — ⌘R;
- editing — ⌘ + double-click; the photo menu also has "Show in Finder";
- settings (⌘,) apply immediately; the viewer and editor are chosen from the installed apps;
- the interface language and date format come from the system settings;
- photos open on double-click (Enter in the photo grid does not work);
- pane widths are not remembered between launches.

## Data

| What | Where |
| --- | --- |
| Settings (library folder, sorting, thumbnail size, map position, apps) | `~/.config/geo-photo-library/config.toml` |
| Database | `~/.local/share/geo-photo-library/library.db` |
| Thumbnails | `~/.cache/geo-photo-library/thumbs/` |

The Flatpak version keeps the same files separately, in `~/.var/app/io.github.EvilX.GeoPhotoLibrary/`.
On macOS the settings and database live in `~/Library/Application Support/geo-photo-library/`,
thumbnails in `~/Library/Caches/geo-photo-library/thumbs/`.

The network is used only for OpenStreetMap tiles and place search; your photos are never uploaded.
Your location is determined by the system (GeoClue) only when you press the button and is not stored.

## Development

- CI (`.github/workflows/ci.yml`): clippy and tests, then RPM builds for x86_64 and aarch64 — in a `fedora:45` container,
  and DEBs for Ubuntu 24.04 and 26.04 (amd64 and arm64) — in containers of those releases;
  on macOS — core tests and a DMG build. On a `v*` tag the packages and the DMG are attached to the GitHub release.
- `GEO_PHOTO_LIBRARY_TRACE=1` — timing of UI operations.
- `cargo run --release --example bench_thumbs -- <folder>` — thumbnail generation benchmark.
- Instructions for AI agents and debug environment variables (in Russian) — [AGENTS.md](AGENTS.md).

## License

Copyright 2026 Leonid Nasedkin.

Licensed under the [Apache License 2.0](LICENSE).

# Geo photo library

[Русский](README.md) | **English**

A photo catalog for Linux (Rust + GTK4 + SQLite) that places your photos on an OpenStreetMap
map using GPS coordinates from EXIF.

![Map tab: photo previews on a world map and the photos in the visible area](docs/screenshots/map-en.png)

## Features

- **Map** — photo previews on the map, clustered; on the right, the photos in the visible area
  sorted by date (newest or oldest first, the choice is remembered). The map position and zoom
  are remembered between launches.
- **Place search** — a search box above the map: type a city, address or landmark and press
  Enter (Nominatim geocoder, requires internet). Several matches are shown as a list.
- **By date** — a year → month → day tree and a thumbnail grid.
- **Information** — a panel on the right with file and EXIF data: camera, lens, shutter speed,
  aperture, ISO, focal length, coordinates, altitude and all tags. It follows the selected photo.
  Open it with the “i” button in the window header or from the photo menu; close it with the same
  button or the cross.
- **Photo menu** (right click): copy the file, view, edit, information, show on map (jumps to
  where the photo was taken; warns if EXIF has no coordinates), move to trash.
- Double click opens the photo in a viewer, Ctrl + double click in an editor. Both are chosen
  in Settings (by default, the app for the file type and a detected image editor).
- A slider under the photo grids scales thumbnails from 50 to 200%.
- On startup the library is rescanned incrementally (new, changed and deleted files).
- Formats: JPEG, HEIC/HEIF (iPhone, Samsung), PNG, WebP, TIFF, DNG (metadata only).
- Interface languages: English, Russian, Greek — from the system locale (`LANG`, `LC_MESSAGES`,
  `LANGUAGE`), English by default.

## Installation

GitHub Actions builds an RPM for Fedora 45 on every push to `main`: **Actions** tab → latest
**CI** run → `geo-photo-library-fc45-rpm` artifact.

```sh
sudo dnf install ./geo-photo-library-*.fc45.x86_64.rpm
```

## Building (Fedora)

```sh
sudo dnf install gcc gtk4-devel libshumate-devel libheif-devel
cargo run --release
```

Without HEIC: `cargo build --release --no-default-features`.

Scan without the GUI: `geo-photo-library --scan <folder>`.

Packages:

- `./packaging/build-rpm.sh` — RPM in `target/rpmbuild/RPMS/x86_64/`;
- `./packaging/flatpak/build-flatpak.sh` — Flatpak bundle `target/flatpak/geo-photo-library.flatpak`
  (the SDK goes into the user installation, no root needed).

## Data

| What | Where |
| --- | --- |
| Settings (library folder, sorting, thumbnail size, map position, apps) | `~/.config/geo-photo-library/config.toml` |
| Database | `~/.local/share/geo-photo-library/library.db` |
| Thumbnails | `~/.cache/geo-photo-library/thumbs/` |

The Flatpak version keeps the same files separately, in `~/.var/app/io.github.EvilX.GeoPhotoLibrary/`.

The network is used only for OpenStreetMap tiles and place search; your photos are never uploaded.

## Development

- CI (`.github/workflows/ci.yml`): clippy and tests, then an RPM build — in a `fedora:45` container.
- `GEO_PHOTO_LIBRARY_TRACE=1` — timing of UI operations.
- `cargo run --release --example bench_thumbs -- <folder>` — thumbnail generation benchmark.
- Instructions for AI agents and debug environment variables (in Russian) — [AGENTS.md](AGENTS.md).

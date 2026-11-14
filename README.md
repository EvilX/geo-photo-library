# Geo photo library

Каталогизатор фотографий для Linux (Rust + GTK4 + SQLite) с поиском снимков на карте
OpenStreetMap по GPS-координатам из EXIF.

- **По датам** — дерево год → месяц → день и сетка миниатюр.
- **Карта** — превью фотографий на карте с кластеризацией; справа — фото из видимой области, по дате.
- При запуске библиотека инкрементально пересканируется (новые, изменённые, удалённые файлы).
- Форматы: JPEG, HEIC/HEIF (iPhone, Samsung), PNG, WebP, TIFF, DNG (только метаданные).

## Сборка (Fedora)

```sh
sudo dnf install gcc gtk4-devel libshumate-devel libheif-devel
cargo run --release
```

Без HEIC: `cargo build --release --no-default-features`.

Сканирование без GUI: `geo-photo-library --scan <папка>`.

## Данные

| Что | Где |
| --- | --- |
| Настройки | `~/.config/geo-photo-library/config.toml` |
| База | `~/.local/share/geo-photo-library/library.db` |
| Миниатюры | `~/.cache/geo-photo-library/thumbs/` |

## Отладка

- `GEO_PHOTO_LIBRARY_TRACE=1` — замеры времени операций интерфейса.
- `cargo run --release --example bench_thumbs -- <папка>` — бенчмарк генерации миниатюр.

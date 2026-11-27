use anyhow::Result;
use chrono::{Datelike, NaiveDateTime};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::{collections::HashMap, path::Path, time::Duration};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS photos (
    id           INTEGER PRIMARY KEY,
    path         TEXT NOT NULL UNIQUE,
    size         INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    taken_at     TEXT NOT NULL,
    year         INTEGER NOT NULL,
    month        INTEGER NOT NULL,
    day          INTEGER NOT NULL,
    lat          REAL,
    lon          REAL,
    width        INTEGER,
    height       INTEGER,
    orientation  INTEGER NOT NULL DEFAULT 1,
    camera_make  TEXT,
    camera_model TEXT,
    thumb_path   TEXT,
    video        INTEGER NOT NULL DEFAULT 0,
    duration     REAL
);
CREATE INDEX IF NOT EXISTS photos_ymd   ON photos(year, month, day);
CREATE INDEX IF NOT EXISTS photos_taken ON photos(taken_at);
CREATE INDEX IF NOT EXISTS photos_geo   ON photos(lat, lon);
";

/// Версия формата данных (PRAGMA user_version).
const SCHEMA_VERSION: i32 = 4;

const PHOTO_COLS: &str = "id, path, taken_at, lat, lon, thumb_path, video, duration";

/// Фильтр вкладки дат: по координатам в EXIF (только фото) или только видео.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GeoFilter {
    /// Все фото и видео.
    #[default]
    All,
    /// Фото с координатами.
    With,
    /// Фото без координат.
    Without,
    /// Только видео (координат у них нет).
    Video,
}

impl GeoFilter {
    /// Порядок пунктов в выпадающем списке; индекс — он же код в C-интерфейсе.
    pub const ALL: [GeoFilter; 4] = [GeoFilter::All, GeoFilter::With, GeoFilter::Without, GeoFilter::Video];

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|&f| f == self).unwrap_or(0) as u32
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).copied().unwrap_or_default()
    }

    /// Отладка: `GEO_PHOTO_LIBRARY_GEO_FILTER=all|with|without|video` — фильтр при запуске.
    pub fn from_env() -> Option<Self> {
        match std::env::var("GEO_PHOTO_LIBRARY_GEO_FILTER").ok()?.as_str() {
            "all" => Some(GeoFilter::All),
            "with" => Some(GeoFilter::With),
            "without" => Some(GeoFilter::Without),
            "video" => Some(GeoFilter::Video),
            _ => None,
        }
    }

    /// Условие SQL для `WHERE`.
    fn sql(self) -> &'static str {
        match self {
            GeoFilter::All => "1",
            GeoFilter::With => "video = 0 AND lat IS NOT NULL AND lon IS NOT NULL",
            // Видео координат не получают — в «без координат» им не место.
            GeoFilter::Without => "video = 0 AND (lat IS NULL OR lon IS NULL)",
            GeoFilter::Video => "video = 1",
        }
    }
}

/// Фото для отображения в UI.
#[derive(Debug, Clone, PartialEq)]
pub struct Photo {
    pub id: i64,
    pub path: String,
    /// `YYYY-MM-DD HH:MM:SS`
    pub taken_at: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub thumb_path: Option<String>,
    pub video: bool,
    /// Длительность видео, с.
    pub duration: Option<f64>,
}

impl Photo {
    /// День съёмки из `taken_at` — тот же, что в столбцах year/month/day.
    pub fn day(&self) -> Option<Day> {
        let d = chrono::NaiveDate::parse_from_str(self.taken_at.get(..10)?, "%Y-%m-%d").ok()?;
        Some((d.year(), d.month(), d.day()))
    }

    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            path: r.get(1)?,
            taken_at: r.get(2)?,
            lat: r.get(3)?,
            lon: r.get(4)?,
            thumb_path: r.get(5)?,
            video: r.get(6)?,
            duration: r.get(7)?,
        })
    }

    pub fn coords(&self) -> Option<(f64, f64)> {
        Some((self.lat?, self.lon?))
    }

    pub fn file_name(&self) -> &str {
        // На Windows разделитель — и «\», и «/».
        self.path.rsplit(std::path::is_separator).next().unwrap_or(&self.path)
    }

    /// Дата съёмки в формате текущего языка, например `14.07.2023 12:30`.
    pub fn display_date(&self) -> String {
        NaiveDateTime::parse_from_str(&self.taken_at, "%Y-%m-%d %H:%M:%S")
            .map(|d| crate::i18n::datetime(&d))
            .unwrap_or_else(|_| self.taken_at.clone())
    }
}

/// Запись для вставки/обновления сканером.
#[derive(Debug, Clone)]
pub struct NewPhoto {
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    pub taken_at: NaiveDateTime,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: u32,
    pub make: Option<String>,
    pub model: Option<String>,
    pub thumb_path: Option<String>,
    pub video: bool,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateCount {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub count: u32,
}

/// День съёмки: (год, месяц, день).
pub type Day = (i32, u32, u32);

impl DateCount {
    /// Число фото по дням, по возрастанию даты — для фильтра карты по фото в её видимой области.
    pub fn tally(days: impl IntoIterator<Item = Day>) -> Vec<DateCount> {
        let mut counts = std::collections::BTreeMap::<Day, u32>::new();
        for d in days {
            *counts.entry(d).or_default() += 1;
        }
        counts.into_iter().map(|((year, month, day), count)| DateCount { year, month, day, count }).collect()
    }
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        // На новой базе сканер и чтения интерфейса переводят её в WAL одновременно, и
        // проигравшее соединение сразу получает «database is locked» — busy_timeout на этот
        // PRAGMA не действует. Режим WAL хранится в файле: после короткой паузы переход
        // другого соединения закончен, и повтор проходит.
        let mut attempt = 0;
        loop {
            match conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(())) {
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::DatabaseBusy && attempt < 200 =>
                {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                r => break r?,
            }
        }
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        // Сброс mtime заставляет сканер обработать файл заново.
        if version < 2 {
            // v2: миниатюры стали квадратными — пересоздать все.
            conn.execute_batch("UPDATE photos SET mtime = -1")?;
        } else if version < 3 {
            // v3: появилась поддержка DNG — повторить файлы без миниатюр.
            conn.execute_batch("UPDATE photos SET mtime = -1 WHERE thumb_path IS NULL")?;
        }
        // v4: видео — столбцы `video` и `duration` (в новой базе их уже создала схема).
        let has_video: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('photos') WHERE name = 'video'", [], |r| r.get(0))?;
        if !has_video {
            conn.execute_batch("ALTER TABLE photos ADD COLUMN video INTEGER NOT NULL DEFAULT 0; ALTER TABLE photos ADD COLUMN duration REAL;")?;
        }
        if version < SCHEMA_VERSION {
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        Ok(Self { conn })
    }

    /// path → (size, mtime) для инкрементального сканирования.
    pub fn known_files(&self) -> Result<HashMap<String, (i64, i64)>> {
        let mut stmt = self.conn.prepare("SELECT path, size, mtime FROM photos")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Видео без миниатюры: сканер повторяет их, когда кадры можно получить (поставили ffmpeg).
    pub fn videos_without_thumbs(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM photos WHERE video = 1 AND thumb_path IS NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn upsert(&mut self, items: &[NewPhoto]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO photos (path, size, mtime, taken_at, year, month, day, lat, lon,
                                     width, height, orientation, camera_make, camera_model, thumb_path, video, duration)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                 ON CONFLICT(path) DO UPDATE SET
                    size=excluded.size, mtime=excluded.mtime, taken_at=excluded.taken_at,
                    year=excluded.year, month=excluded.month, day=excluded.day,
                    lat=excluded.lat, lon=excluded.lon, width=excluded.width, height=excluded.height,
                    orientation=excluded.orientation, camera_make=excluded.camera_make,
                    camera_model=excluded.camera_model, thumb_path=excluded.thumb_path,
                    video=excluded.video, duration=excluded.duration",
            )?;
            for p in items {
                stmt.execute(params![
                    p.path,
                    p.size,
                    p.mtime,
                    p.taken_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                    p.taken_at.year(),
                    p.taken_at.month(),
                    p.taken_at.day(),
                    p.lat,
                    p.lon,
                    p.width,
                    p.height,
                    p.orientation,
                    p.make,
                    p.model,
                    p.thumb_path,
                    p.video,
                    p.duration,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Удаляет записи и возвращает пути их миниатюр.
    pub fn delete(&mut self, paths: &[String]) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        let mut thumbs = Vec::new();
        {
            let mut sel = tx.prepare("SELECT thumb_path FROM photos WHERE path = ?1")?;
            let mut del = tx.prepare("DELETE FROM photos WHERE path = ?1")?;
            for p in paths {
                if let Ok(Some(t)) = sel.query_row([p], |r| r.get::<_, Option<String>>(0)) {
                    thumbs.push(t);
                }
                del.execute([p])?;
            }
        }
        tx.commit()?;
        Ok(thumbs)
    }

    /// Удаляет одну запись (фото перемещено в корзину из интерфейса).
    /// Возвращает путь миниатюры; `None`, если записи не было или миниатюры нет.
    pub fn remove(&self, id: i64) -> Result<Option<String>> {
        let thumb = self
            .conn
            .query_row("SELECT thumb_path FROM photos WHERE id = ?1", [id], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten();
        self.conn.execute("DELETE FROM photos WHERE id = ?1", [id])?;
        Ok(thumb)
    }

    /// Обновить запись по файлу, заново прочитанному после записи EXIF (`scanner::process_file`):
    /// дата, координаты, ориентация, миниатюра, а также новые размер и время изменения —
    /// чтобы сканер не обрабатывал файл ещё раз. Записи нет — ничего не делает.
    pub fn update(&self, p: &NewPhoto) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET size = ?2, mtime = ?3, taken_at = ?4, year = ?5, month = ?6, day = ?7,
                    lat = ?8, lon = ?9, width = ?10, height = ?11, orientation = ?12,
                    camera_make = ?13, camera_model = ?14, thumb_path = ?15, video = ?16, duration = ?17
             WHERE path = ?1",
            params![
                p.path,
                p.size,
                p.mtime,
                p.taken_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                p.taken_at.year(),
                p.taken_at.month(),
                p.taken_at.day(),
                p.lat,
                p.lon,
                p.width,
                p.height,
                p.orientation,
                p.make,
                p.model,
                p.thumb_path,
                p.video,
                p.duration,
            ],
        )?;
        Ok(())
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))?)
    }

    pub fn date_counts(&self, filter: GeoFilter) -> Result<Vec<DateCount>> {
        let sql = format!(
            "SELECT year, month, day, COUNT(*) FROM photos WHERE {} GROUP BY year, month, day
             ORDER BY year, month, day",
            filter.sql()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            Ok(DateCount { year: r.get(0)?, month: r.get(1)?, day: r.get(2)?, count: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn photos_by_date(&self, year: i32, month: Option<u32>, day: Option<u32>, filter: GeoFilter) -> Result<Vec<Photo>> {
        let sql = format!(
            "SELECT {PHOTO_COLS} FROM photos
             WHERE year = ?1 AND (?2 IS NULL OR month = ?2) AND (?3 IS NULL OR day = ?3) AND {}
             ORDER BY taken_at, path",
            filter.sql()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![year, month, day], Photo::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Все фото с координатами, по дате.
    pub fn geotagged(&self) -> Result<Vec<Photo>> {
        self.geotagged_by_date(None, None, None)
    }

    /// Фото с координатами за год, месяц или день (`year == None` — за всё время), по дате.
    pub fn geotagged_by_date(&self, year: Option<i32>, month: Option<u32>, day: Option<u32>) -> Result<Vec<Photo>> {
        let sql = format!(
            "SELECT {PHOTO_COLS} FROM photos WHERE lat IS NOT NULL AND lon IS NOT NULL
             AND (?1 IS NULL OR year = ?1) AND (?2 IS NULL OR month = ?2) AND (?3 IS NULL OR day = ?3)
             ORDER BY taken_at, path"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![year, month, day], Photo::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn np(path: &str, date: &str, coords: Option<(f64, f64)>) -> NewPhoto {
        NewPhoto {
            path: path.into(),
            size: 1,
            mtime: 1,
            taken_at: NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S").unwrap(),
            lat: coords.map(|c| c.0),
            lon: coords.map(|c| c.1),
            width: None,
            height: None,
            orientation: 1,
            make: None,
            model: None,
            thumb_path: Some(format!("{path}.thumb")),
            video: false,
            duration: None,
        }
    }

    #[test]
    fn crud_and_queries() {
        let mut db = Db::open_in_memory().unwrap();
        db.upsert(&[
            np("/a.jpg", "2023-07-14 12:00:00", Some((55.7, 37.6))),
            np("/b.jpg", "2023-07-14 09:00:00", None),
            np("/c.jpg", "2023-08-01 10:00:00", Some((59.9, 30.3))),
            np("/d.jpg", "2024-01-02 10:00:00", None),
        ])
        .unwrap();
        assert_eq!(db.count().unwrap(), 4);

        let dc = db.date_counts(GeoFilter::All).unwrap();
        assert_eq!(dc[0], DateCount { year: 2023, month: 7, day: 14, count: 2 });
        assert_eq!(dc.len(), 3);

        let july = db.photos_by_date(2023, Some(7), None, GeoFilter::All).unwrap();
        assert_eq!(july.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["/b.jpg", "/a.jpg"]);
        assert_eq!(db.photos_by_date(2023, None, None, GeoFilter::All).unwrap().len(), 3);
        assert_eq!(db.photos_by_date(2023, Some(8), Some(1), GeoFilter::All).unwrap().len(), 1);

        // фильтр вкладки дат по координатам
        let with = db.date_counts(GeoFilter::With).unwrap();
        assert_eq!(with.iter().map(|c| (c.month, c.day, c.count)).collect::<Vec<_>>(), [(7, 14, 1), (8, 1, 1)]);
        let without = db.date_counts(GeoFilter::Without).unwrap();
        assert_eq!(without.iter().map(|c| (c.year, c.count)).collect::<Vec<_>>(), [(2023, 1), (2024, 1)]);
        let paths = |f| db.photos_by_date(2023, Some(7), None, f).unwrap().into_iter().map(|p| p.path).collect::<Vec<_>>();
        assert_eq!(paths(GeoFilter::With), ["/a.jpg"]);
        assert_eq!(paths(GeoFilter::Without), ["/b.jpg"]);
        assert_eq!(GeoFilter::from_index(GeoFilter::Without.index()), GeoFilter::Without);
        assert_eq!(GeoFilter::from_index(3), GeoFilter::Video);
        assert_eq!(GeoFilter::from_index(7), GeoFilter::All);

        let geo = db.geotagged().unwrap();
        assert_eq!(geo.len(), 2);
        assert_eq!(geo[0].coords(), Some((55.7, 37.6)));

        // фильтр карты по датам: фото с координатами за период, дни съёмки и их счётчики
        assert_eq!(geo.iter().map(Photo::day).collect::<Vec<_>>(), [Some((2023, 7, 14)), Some((2023, 8, 1))]);
        assert_eq!(
            DateCount::tally([(2023, 8, 1), (2023, 7, 14), (2023, 8, 1)]),
            [DateCount { year: 2023, month: 7, day: 14, count: 1 }, DateCount { year: 2023, month: 8, day: 1, count: 2 }]
        );
        assert_eq!(db.geotagged_by_date(Some(2023), None, None).unwrap().len(), 2);
        let aug = db.geotagged_by_date(Some(2023), Some(8), None).unwrap();
        assert_eq!(aug.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["/c.jpg"]);
        assert!(db.geotagged_by_date(Some(2024), None, None).unwrap().is_empty());

        // повторная вставка того же пути — обновление
        db.upsert(&[np("/a.jpg", "2020-01-01 00:00:00", None)]).unwrap();
        assert_eq!(db.count().unwrap(), 4);
        assert_eq!(db.geotagged().unwrap().len(), 1);

        let thumbs = db.delete(&["/a.jpg".into(), "/missing.jpg".into()]).unwrap();
        assert_eq!(thumbs, ["/a.jpg.thumb"]);
        assert_eq!(db.known_files().unwrap().len(), 3);

        // обновление после записи EXIF: новые координаты, дата, размер и время изменения
        let mut b = np("/b.jpg", "2023-07-15 10:00:00", Some((40.0, -3.5)));
        (b.size, b.mtime) = (7, 8);
        db.update(&b).unwrap();
        assert_eq!(db.known_files().unwrap()["/b.jpg"], (7, 8));
        assert_eq!(db.photos_by_date(2023, Some(7), Some(14), GeoFilter::All).unwrap().len(), 0);
        assert_eq!(db.photos_by_date(2023, Some(7), Some(15), GeoFilter::With).unwrap().len(), 1);

        let p = db.photos_by_date(2023, Some(8), Some(1), GeoFilter::All).unwrap().remove(0);
        assert_eq!(db.remove(p.id).unwrap(), Some(format!("{}.thumb", p.path)));
        assert_eq!(db.count().unwrap(), 2);
        assert_eq!(db.remove(p.id).unwrap(), None);
    }

    /// Видео — в «Все» и «Видео», но не в фильтрах по координатам.
    #[test]
    fn video_filter() {
        let mut db = Db::open_in_memory().unwrap();
        let mut v = np("/v.mp4", "2023-07-14 10:00:00", None);
        (v.video, v.duration) = (true, Some(12.5));
        db.upsert(&[np("/a.jpg", "2023-07-14 12:00:00", Some((55.7, 37.6))), np("/b.jpg", "2023-07-14 09:00:00", None), v])
            .unwrap();
        let paths = |f| db.photos_by_date(2023, None, None, f).unwrap().into_iter().map(|p| p.path).collect::<Vec<_>>();
        assert_eq!(paths(GeoFilter::All), ["/b.jpg", "/v.mp4", "/a.jpg"]);
        assert_eq!(paths(GeoFilter::With), ["/a.jpg"]);
        assert_eq!(paths(GeoFilter::Without), ["/b.jpg"]);
        assert_eq!(paths(GeoFilter::Video), ["/v.mp4"]);
        assert_eq!(db.date_counts(GeoFilter::Video).unwrap()[0].count, 1);
        let v = db.photos_by_date(2023, None, None, GeoFilter::Video).unwrap().remove(0);
        assert!(v.video);
        assert_eq!(v.duration, Some(12.5));
    }

    /// База v3 без столбцов видео получает их, записи сохраняются.
    #[test]
    fn migration_v3_to_v4_adds_video_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&SCHEMA.replace(",\n    video        INTEGER NOT NULL DEFAULT 0,\n    duration     REAL", "")).unwrap();
        conn.execute_batch("PRAGMA user_version = 3; INSERT INTO photos (path, size, mtime, taken_at, year, month, day) VALUES ('/a.jpg', 1, 1, '2023-01-01 00:00:00', 2023, 1, 1)").unwrap();
        let db = Db::init(conn).unwrap();
        let p = db.photos_by_date(2023, None, None, GeoFilter::All).unwrap();
        assert_eq!(p.len(), 1);
        assert!(!p[0].video);
        assert_eq!(db.known_files().unwrap()["/a.jpg"], (1, 1));
    }

    #[test]
    fn migration_v2_to_v3_retries_missing_thumbs() {
        let mut db = Db::open_in_memory().unwrap();
        let mut no_thumb = np("/raw.dng", "2023-01-01 00:00:00", None);
        no_thumb.thumb_path = None;
        db.upsert(&[np("/ok.jpg", "2023-01-01 00:00:00", None), no_thumb]).unwrap();
        db.conn.execute_batch("PRAGMA user_version = 2").unwrap();
        let db = Db::init(db.conn).unwrap();
        let known = db.known_files().unwrap();
        assert_eq!(known["/ok.jpg"].1, 1);
        assert_eq!(known["/raw.dng"].1, -1);
        let v: i32 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    /// Сканер и чтения интерфейса открывают новую базу одновременно — никто не должен
    /// получить «database is locked».
    #[test]
    fn concurrent_open_of_new_db() {
        for round in 0..20 {
            let dir = std::env::temp_dir().join(format!("gpl-db-open-{}-{round}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let path = dir.join("library.db");
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    let path = path.clone();
                    std::thread::spawn(move || Db::open(&path).and_then(|db| db.count()).map_err(|e| e.to_string()))
                })
                .collect();
            for t in threads {
                assert_eq!(t.join().unwrap(), Ok(0));
            }
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn display() {
        let p = Photo {
            id: 1,
            path: "/x/y/IMG_1.HEIC".into(),
            taken_at: "2023-07-14 12:30:45".into(),
            lat: None,
            lon: None,
            thumb_path: None,
            video: false,
            duration: None,
        };
        assert_eq!(p.file_name(), "IMG_1.HEIC");
        assert!(p.display_date().contains("12:30"));
        #[cfg(windows)]
        assert_eq!(Photo { path: r"C:\Photos\IMG_2.jpg".into(), ..p }.file_name(), "IMG_2.jpg");
    }
}

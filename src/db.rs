use anyhow::Result;
use chrono::{Datelike, NaiveDateTime};
use rusqlite::{params, Connection, Row};
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
    thumb_path   TEXT
);
CREATE INDEX IF NOT EXISTS photos_ymd   ON photos(year, month, day);
CREATE INDEX IF NOT EXISTS photos_taken ON photos(taken_at);
CREATE INDEX IF NOT EXISTS photos_geo   ON photos(lat, lon);
";

const PHOTO_COLS: &str = "id, path, taken_at, lat, lon, thumb_path";

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
}

impl Photo {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            path: r.get(1)?,
            taken_at: r.get(2)?,
            lat: r.get(3)?,
            lon: r.get(4)?,
            thumb_path: r.get(5)?,
        })
    }

    pub fn coords(&self) -> Option<(f64, f64)> {
        Some((self.lat?, self.lon?))
    }

    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// `14.07.2023 12:30`
    pub fn display_date(&self) -> String {
        NaiveDateTime::parse_from_str(&self.taken_at, "%Y-%m-%d %H:%M:%S")
            .map(|d| d.format("%d.%m.%Y %H:%M").to_string())
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateCount {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub count: u32,
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
        conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        conn.busy_timeout(Duration::from_secs(10))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// path → (size, mtime) для инкрементального сканирования.
    pub fn known_files(&self) -> Result<HashMap<String, (i64, i64)>> {
        let mut stmt = self.conn.prepare("SELECT path, size, mtime FROM photos")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn upsert(&mut self, items: &[NewPhoto]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO photos (path, size, mtime, taken_at, year, month, day, lat, lon,
                                     width, height, orientation, camera_make, camera_model, thumb_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(path) DO UPDATE SET
                    size=excluded.size, mtime=excluded.mtime, taken_at=excluded.taken_at,
                    year=excluded.year, month=excluded.month, day=excluded.day,
                    lat=excluded.lat, lon=excluded.lon, width=excluded.width, height=excluded.height,
                    orientation=excluded.orientation, camera_make=excluded.camera_make,
                    camera_model=excluded.camera_model, thumb_path=excluded.thumb_path",
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

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))?)
    }

    pub fn date_counts(&self) -> Result<Vec<DateCount>> {
        let mut stmt = self.conn.prepare(
            "SELECT year, month, day, COUNT(*) FROM photos GROUP BY year, month, day
             ORDER BY year, month, day",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DateCount { year: r.get(0)?, month: r.get(1)?, day: r.get(2)?, count: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn photos_by_date(&self, year: i32, month: Option<u32>, day: Option<u32>) -> Result<Vec<Photo>> {
        let sql = format!(
            "SELECT {PHOTO_COLS} FROM photos
             WHERE year = ?1 AND (?2 IS NULL OR month = ?2) AND (?3 IS NULL OR day = ?3)
             ORDER BY taken_at, path"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![year, month, day], Photo::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Все фото с координатами, по дате.
    pub fn geotagged(&self) -> Result<Vec<Photo>> {
        let sql = format!(
            "SELECT {PHOTO_COLS} FROM photos WHERE lat IS NOT NULL AND lon IS NOT NULL
             ORDER BY taken_at, path"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Photo::from_row)?;
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

        let dc = db.date_counts().unwrap();
        assert_eq!(dc[0], DateCount { year: 2023, month: 7, day: 14, count: 2 });
        assert_eq!(dc.len(), 3);

        let july = db.photos_by_date(2023, Some(7), None).unwrap();
        assert_eq!(july.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["/b.jpg", "/a.jpg"]);
        assert_eq!(db.photos_by_date(2023, None, None).unwrap().len(), 3);
        assert_eq!(db.photos_by_date(2023, Some(8), Some(1)).unwrap().len(), 1);

        let geo = db.geotagged().unwrap();
        assert_eq!(geo.len(), 2);
        assert_eq!(geo[0].coords(), Some((55.7, 37.6)));

        // повторная вставка того же пути — обновление
        db.upsert(&[np("/a.jpg", "2020-01-01 00:00:00", None)]).unwrap();
        assert_eq!(db.count().unwrap(), 4);
        assert_eq!(db.geotagged().unwrap().len(), 1);

        let thumbs = db.delete(&["/a.jpg".into(), "/missing.jpg".into()]).unwrap();
        assert_eq!(thumbs, ["/a.jpg.thumb"]);
        assert_eq!(db.known_files().unwrap().len(), 3);
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
        };
        assert_eq!(p.file_name(), "IMG_1.HEIC");
        assert_eq!(p.display_date(), "14.07.2023 12:30");
    }
}

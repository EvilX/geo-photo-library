//! Поиск встроенных JPEG-превью в DNG (и других RAW на основе TIFF).
//!
//! iPhone ProRAW: IFD0 — JPEG-превью в полном размере (YCbCr), SubIFD0 — RAW (LinearRaw),
//! SubIFD1 — маски/глубина. Читаются только заголовки IFD, без загрузки всего файла.
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

const NEW_SUBFILE_TYPE: u16 = 254;
const WIDTH: u16 = 256;
const HEIGHT: u16 = 257;
const COMPRESSION: u16 = 259;
const PHOTOMETRIC: u16 = 262;
const STRIP_OFFSETS: u16 = 273;
const STRIP_BYTE_COUNTS: u16 = 279;
const SUB_IFDS: u16 = 330;
const JPEG_OFFSET: u16 = 513;
const JPEG_LENGTH: u16 = 514;

/// Не больше стольких IFD — защита от зацикленных/битых файлов.
const MAX_IFDS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preview {
    pub offset: u64,
    pub len: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
struct Ifd {
    subfile: u32,
    width: u32,
    height: u32,
    compression: u32,
    photometric: Option<u32>,
    strip: Option<(u64, u64)>,
    jpeg: (Option<u64>, Option<u64>),
    sub_ifds: Vec<u64>,
    next: u64,
}

struct Tiff<R> {
    r: R,
    le: bool,
}

impl<R: Read + Seek> Tiff<R> {
    fn u16(&mut self) -> io::Result<u16> {
        let mut b = [0; 2];
        self.r.read_exact(&mut b)?;
        Ok(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&mut self) -> io::Result<u32> {
        let mut b = [0; 4];
        self.r.read_exact(&mut b)?;
        Ok(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// Значения SHORT/LONG/IFD-тега (до `limit` штук).
    fn values(&mut self, typ: u16, count: u32, limit: usize) -> io::Result<Vec<u32>> {
        let size = match typ {
            3 => 2,     // SHORT
            4 | 13 => 4, // LONG, IFD
            _ => return Ok(Vec::new()),
        };
        let n = (count as usize).min(limit);
        if size * count as usize > 4 {
            let off = self.u32()?;
            self.r.seek(SeekFrom::Start(off as u64))?;
        }
        (0..n).map(|_| if size == 2 { self.u16().map(u32::from) } else { self.u32() }).collect()
    }

    fn ifd(&mut self, offset: u64) -> io::Result<Ifd> {
        self.r.seek(SeekFrom::Start(offset))?;
        let n = self.u16()?;
        if n > 1000 {
            return Err(io::Error::other("слишком много тегов в IFD"));
        }
        let mut ifd = Ifd::default();
        for i in 0..n as u64 {
            let entry = offset + 2 + i * 12;
            self.r.seek(SeekFrom::Start(entry))?;
            let (tag, typ, count) = (self.u16()?, self.u16()?, self.u32()?);
            let first = |v: Vec<u32>| v.first().copied();
            match tag {
                NEW_SUBFILE_TYPE => ifd.subfile = first(self.values(typ, count, 1)?).unwrap_or(0),
                WIDTH => ifd.width = first(self.values(typ, count, 1)?).unwrap_or(0),
                HEIGHT => ifd.height = first(self.values(typ, count, 1)?).unwrap_or(0),
                COMPRESSION => ifd.compression = first(self.values(typ, count, 1)?).unwrap_or(0),
                PHOTOMETRIC => ifd.photometric = first(self.values(typ, count, 1)?),
                // Превью берём только одним фрагментом.
                STRIP_OFFSETS if count == 1 => {
                    let off = first(self.values(typ, count, 1)?).unwrap_or(0) as u64;
                    ifd.strip = Some((off, ifd.strip.map_or(0, |s| s.1)));
                }
                STRIP_BYTE_COUNTS if count == 1 => {
                    let len = first(self.values(typ, count, 1)?).unwrap_or(0) as u64;
                    ifd.strip = Some((ifd.strip.map_or(0, |s| s.0), len));
                }
                JPEG_OFFSET => ifd.jpeg.0 = first(self.values(typ, count, 1)?).map(u64::from),
                JPEG_LENGTH => ifd.jpeg.1 = first(self.values(typ, count, 1)?).map(u64::from),
                SUB_IFDS => {
                    ifd.sub_ifds = self.values(typ, count, MAX_IFDS)?.into_iter().map(u64::from).collect()
                }
                _ => {}
            }
        }
        self.r.seek(SeekFrom::Start(offset + 2 + n as u64 * 12))?;
        ifd.next = self.u32()? as u64;
        Ok(ifd)
    }
}

impl Ifd {
    /// JPEG-превью (не RAW-данные, не маски/глубина).
    fn preview(&self) -> Option<Preview> {
        // 6 — «старый» JPEG, 7 — JPEG; у RAW тоже бывает 7, отсекаем по фотометрии.
        if !matches!(self.compression, 6 | 7) {
            return None;
        }
        // RGB, YCbCr или не указано (старый JPEG). CFA/LinearRaw/маски — нет.
        if !matches!(self.photometric, None | Some(2) | Some(6)) {
            return None;
        }
        // Бит 0 — уменьшенная копия; бит 2 — маска прозрачности/глубины.
        if self.subfile & 0b100 != 0 {
            return None;
        }
        let (offset, len) = match self.jpeg {
            (Some(o), Some(l)) => (o, l),
            _ => self.strip?,
        };
        (offset > 0 && len > 0).then_some(Preview { offset, len, width: self.width, height: self.height })
    }
}

/// Все встроенные JPEG-превью файла.
pub fn previews(path: &Path) -> io::Result<Vec<Preview>> {
    let mut r = io::BufReader::new(File::open(path)?);
    let mut hdr = [0; 8];
    r.read_exact(&mut hdr)?;
    let le = match &hdr[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(io::Error::other("не TIFF")),
    };
    let mut t = Tiff { r, le };
    let first = if le { u32::from_le_bytes(hdr[4..8].try_into().unwrap()) } else { u32::from_be_bytes(hdr[4..8].try_into().unwrap()) };

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = vec![first as u64];
    while let Some(off) = queue.pop() {
        if off == 0 || !seen.insert(off) || seen.len() > MAX_IFDS {
            continue;
        }
        let ifd = t.ifd(off)?;
        out.extend(ifd.preview());
        queue.push(ifd.next);
        queue.extend(ifd.sub_ifds.iter().copied());
    }
    Ok(out)
}

/// Лучшее превью для миниатюры `target`: самое маленькое, у которого короткая сторона
/// не меньше `target` (меньше декодировать), иначе самое большое.
pub fn best_preview(previews: &[Preview], target: u32) -> Option<Preview> {
    let short = |p: &Preview| p.width.min(p.height);
    previews
        .iter()
        .filter(|p| short(p) >= target)
        .min_by_key(|p| p.width as u64 * p.height as u64)
        .or_else(|| previews.iter().max_by_key(|p| p.width as u64 * p.height as u64))
        .copied()
}

pub fn read_preview(path: &Path, p: &Preview) -> io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(p.offset))?;
    let mut buf = vec![0; p.len as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Минимальный DNG как у iPhone ProRAW: IFD0 = JPEG-превью, SubIFD0 = «RAW»,
    /// SubIFD1 = маска глубины. Little-endian.
    pub fn fake_dng(jpeg: &[u8], width: u32, height: u32) -> Vec<u8> {
        fn entry(out: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: u32) {
            out.extend(tag.to_le_bytes());
            out.extend(typ.to_le_bytes());
            out.extend(count.to_le_bytes());
            if typ == 3 && count == 1 {
                out.extend((value as u16).to_le_bytes());
                out.extend([0, 0]);
            } else {
                out.extend(value.to_le_bytes());
            }
        }
        fn ifd(entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
            let mut out = (entries.len() as u16).to_le_bytes().to_vec();
            for &(t, ty, c, v) in entries {
                entry(&mut out, t, ty, c, v);
            }
            out.extend(0u32.to_le_bytes());
            out
        }
        let ifd_len = |n: usize| 2 + 12 * n + 4;
        let ifd0_off = 8;
        let sub_list_off = ifd0_off + ifd_len(9) as u32;
        let sub0_off = sub_list_off + 8;
        let sub1_off = sub0_off + ifd_len(6) as u32;
        let raw_off = sub1_off + ifd_len(4) as u32;
        let jpeg_off = raw_off + 16;

        let mut out = b"II*\0".to_vec();
        out.extend((ifd0_off).to_le_bytes());
        out.extend(ifd(&[
            (NEW_SUBFILE_TYPE, 4, 1, 1),
            (WIDTH, 4, 1, width),
            (HEIGHT, 4, 1, height),
            (COMPRESSION, 3, 1, 7),
            (PHOTOMETRIC, 3, 1, 6),
            (STRIP_OFFSETS, 4, 1, jpeg_off),
            (274, 3, 1, 1), // Orientation
            (STRIP_BYTE_COUNTS, 4, 1, jpeg.len() as u32),
            (SUB_IFDS, 4, 2, sub_list_off),
        ]));
        out.extend(sub0_off.to_le_bytes());
        out.extend(sub1_off.to_le_bytes());
        out.extend(ifd(&[
            (NEW_SUBFILE_TYPE, 4, 1, 0),
            (WIDTH, 4, 1, width),
            (HEIGHT, 4, 1, height),
            (COMPRESSION, 3, 1, 7),
            (PHOTOMETRIC, 3, 1, 34892),
            (STRIP_OFFSETS, 4, 1, raw_off),
        ]));
        out.extend(ifd(&[
            (NEW_SUBFILE_TYPE, 4, 1, 65540),
            (COMPRESSION, 3, 1, 7),
            (STRIP_OFFSETS, 4, 1, raw_off),
            (STRIP_BYTE_COUNTS, 4, 1, 16),
        ]));
        assert_eq!(out.len() as u32, raw_off);
        out.extend([0u8; 16]);
        out.extend(jpeg);
        out
    }

    #[test]
    fn finds_iphone_style_preview() {
        let dir = std::env::temp_dir().join(format!("geo-photo-library-dng-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jpeg = b"\xff\xd8fake jpeg\xff\xd9".to_vec();
        let path = dir.join("x.dng");
        std::fs::write(&path, fake_dng(&jpeg, 800, 600)).unwrap();

        let p = previews(&path).unwrap();
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!((p[0].width, p[0].height), (800, 600));
        assert_eq!(read_preview(&path, &p[0]).unwrap(), jpeg);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn best_choice() {
        let p = |w, h| Preview { offset: 1, len: 1, width: w, height: h };
        let list = [p(256, 171), p(1024, 768), p(8064, 6048)];
        assert_eq!(best_preview(&list, 320), Some(p(1024, 768)));
        assert_eq!(best_preview(&list[..1], 320), Some(p(256, 171)));
        assert_eq!(best_preview(&[], 320), None);
    }
}

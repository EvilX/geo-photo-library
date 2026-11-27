//! Редактирование EXIF файла: координаты GPS и любые теги основного изображения
//! (читает kamadak-exif, пишет little_exif — kamadak-exif умеет только читать).
//!
//! Пишем только туда, где запись проверена на снимках телефонов и не трогает остальное:
//! JPEG, HEIC/HEIF и WebP. В PNG little_exif кладёт EXIF в текстовый блок, который не
//! читает сканер, а в DNG/TIFF переносит данные внутри файла — оригиналы RAW не трогаем.
use crate::{db::NewPhoto, i18n, metadata, scanner};
use anyhow::{anyhow, bail, Context, Result};
use exif::In;
use little_exif::{
    endian::Endian, exif_tag::ExifTag, exif_tag_format::ExifTagFormat, filetype::FileExtension, ifd::ExifTagGroup,
    metadata::Metadata, rational::uR64,
};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Можно ли записать координаты в этот файл (по расширению).
pub fn writable(path: &Path) -> bool {
    file_type(path).is_some()
}

fn file_type(path: &Path) -> Option<FileExtension> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some(FileExtension::JPEG),
        "heic" | "heif" => Some(FileExtension::HEIF),
        "webp" => Some(FileExtension::WEBP),
        _ => None,
    }
}

/// Размер и время изменения файла — как их хранит сканер.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: i64,
    pub mtime: i64,
}

impl Stamp {
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let md = std::fs::metadata(path)?;
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        Ok(Self { size: md.len() as i64, mtime })
    }
}

/// Что записать в файл: координаты (вкладка «Координаты») и изменённые теги
/// (вкладка «EXIF»: ключ поля [`Field::key`] → новый текст; пустой текст — удалить тег).
/// Остальные теги файла не меняются.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    pub coords: Option<(f64, f64)>,
    pub tags: Vec<(String, String)>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.coords.is_none() && self.tags.is_empty()
    }
}

/// Записать координаты в EXIF файла.
pub fn write_coords(path: &Path, lat: f64, lon: f64) -> Result<Stamp> {
    write(path, &Changes { coords: Some((lat, lon)), tags: Vec::new() })
}

/// Записать изменения и заново прочитать файл, как сканер: запись для `Db::update`.
pub fn apply(path: &Path, changes: &Changes, thumbs_dir: &Path) -> Result<NewPhoto> {
    let st = write(path, changes)?;
    Ok(scanner::process_file(path, st.size, st.mtime, thumbs_dir))
}

/// Записать изменения в EXIF файла. Новый файл собирается в памяти и проверяется
/// (изменённое читается обратно, прежние теги на месте), затем атомарно заменяет старый.
/// Возвращает новые размер и время изменения.
pub fn write(path: &Path, changes: &Changes) -> Result<Stamp> {
    if let Some((lat, lon)) = changes.coords {
        if !metadata::valid_coords(lat, lon) {
            bail!(i18n::invalid_coordinates());
        }
    }
    let Some(kind) = file_type(path) else {
        bail!(i18n::exif_format_unsupported());
    };
    // Сканер ходит по символическим ссылкам: пишем в сам файл, иначе переименование
    // заменило бы ссылку обычным файлом, а оригинал остался бы прежним.
    let target;
    let path = if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        target = std::fs::canonicalize(path)?;
        target.as_path()
    } else {
        path
    };
    let original = std::fs::read(path)?;
    let data = with_changes(&original, kind, changes)?;
    replace_file(path, &data)?;
    Ok(Stamp::of(path)?)
}

/// Содержимое файла с изменениями (проверенное).
fn with_changes(original: &[u8], kind: FileExtension, changes: &Changes) -> Result<Vec<u8>> {
    let before = tags(original);
    // Без EXIF — новый блок. Если EXIF есть, а little_exif его не разобрал, — ещё попытка
    // с исправленными типами тегов; не вышло — файл не трогаем: иначе все прежние теги пропали бы.
    let mut base = original.to_vec();
    let mut md = match decode(&base, kind) {
        Ok(md) => md,
        Err(_) if before.is_none() => Metadata::new(),
        Err(e) => match fix_formats(&base) {
            Some(fixed) => {
                let md = decode(&fixed, kind).context("EXIF")?;
                base = fixed;
                md
            }
            None => return Err(e).context("EXIF"),
        },
    };
    if let Some((lat, lon)) = changes.coords {
        md.set_tag(ExifTag::GPSVersionID(vec![2, 3, 0, 0]));
        md.set_tag(ExifTag::GPSLatitudeRef(if lat < 0.0 { "S" } else { "N" }.into()));
        md.set_tag(ExifTag::GPSLatitude(dms(lat)));
        md.set_tag(ExifTag::GPSLongitudeRef(if lon < 0.0 { "W" } else { "E" }.into()));
        md.set_tag(ExifTag::GPSLongitude(dms(lon)));
    }
    // Ожидаемое после записи: значение тега или `None` — тег удалён.
    let mut expected = Vec::new();
    for (key, text) in &changes.tags {
        let key = Key::parse(key).with_context(|| format!("тег {key}"))?;
        if text.is_empty() {
            md.remove_tag_by_hex_group(key.tag, key.group.little());
            expected.push((key, None));
            continue;
        }
        let (val, raw) = encode(key, text).map_err(|e| anyhow!("{}: {e}", key.name()))?;
        let tag = ExifTag::from_u16_with_data(key.tag, &key.format, &raw, &Endian::Little, &key.group.little())
            .map_err(|e| anyhow!("{}: {e}", key.name()))?;
        md.set_tag(tag);
        expected.push((key, Some(val)));
    }
    let mut data = base;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| md.write_to_vec(&mut data, kind)))
        .map_err(|_| anyhow!("сбой little_exif при записи"))?
        .context("EXIF")?;

    // Проверка: изменённое читается так же, как его прочтёт сканер, и ни один прежний тег
    // (кроме координат и удалённых) не потерян.
    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(&data)).ok();
    if let Some((lat, lon)) = changes.coords {
        let meta = metadata::read_from(&mut Cursor::new(&data));
        let ok = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-6);
        if !ok(meta.lat, lat) || !ok(meta.lon, lon) {
            bail!("координаты не записались");
        }
    }
    for (key, val) in &expected {
        let got = exif.as_ref().and_then(|e| e.get_field(exif::Tag(key.group.context(), key.tag), In::PRIMARY)).map(|f| Val::of(&f.value).0);
        if got.as_ref() != val.as_ref() {
            bail!("тег EXIF {} не записался", key.name());
        }
    }
    if let Some(before) = before {
        let after = tags(&data).unwrap_or_default();
        let removed: HashSet<String> = expected.iter().filter(|e| e.1.is_none()).map(|e| e.0.name()).collect();
        let replaced = |name: &str| (changes.coords.is_some() && name.starts_with("GPS")) || removed.contains(name);
        if let Some(lost) = before.iter().find(|t| !replaced(&t.1) && !after.contains(t)) {
            bail!("после записи пропал тег EXIF {}", lost.1);
        }
    }
    Ok(data)
}

/// Разобрать EXIF файла little_exif. Его утверждения (`assert!`) на странных данных
/// роняли бы поток — паника становится ошибкой.
fn decode(data: &[u8], kind: FileExtension) -> Result<Metadata> {
    std::panic::catch_unwind(|| Metadata::new_from_vec(&data.to_vec(), kind))
        .map_err(|_| anyhow!("сбой little_exif при разборе"))?
        .map_err(Into::into)
}

/// Копия файла, где у известных little_exif тегов исправлен тип, если в файле он другой,
/// но того же размера: знаковое вместо беззнакового и наоборот (камеры пишут, например,
/// ExifImageWidth как SLONG, и little_exif отказывается разбирать весь блок). Значения
/// не меняются. `None` — EXIF не найден или исправлять нечего.
fn fix_formats(original: &[u8]) -> Option<Vec<u8>> {
    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(original)).ok()?;
    let tiff = exif.buf();
    // Блок TIFF лежит в файле как есть — его место ищем по содержимому.
    let head = &tiff[..tiff.len().min(64)];
    let start = original.windows(head.len()).enumerate().find(|(i, w)| *w == head && original[*i..].starts_with(tiff))?.0;
    let mut data = original.to_vec();
    let fixed = fix_tiff(&mut data[start..start + tiff.len()]);
    (fixed > 0).then_some(data)
}

/// Исправить типы тегов в блоке TIFF (см. [`fix_formats`]); возвращает число исправлений.
fn fix_tiff(t: &mut [u8]) -> usize {
    let le = match t.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return 0,
    };
    let u16_at = |t: &[u8], o: usize| t.get(o..o + 2).map(|b| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) });
    let u32_at = |t: &[u8], o: usize| {
        t.get(o..o + 4).map(|b| if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    };
    let Some(first) = u32_at(t, 4) else { return 0 };
    let mut todo = vec![(first as usize, ExifTagGroup::GENERIC)];
    let mut seen = HashSet::new();
    let mut fixed = 0;
    while let Some((ifd, group)) = todo.pop() {
        // Защита от зацикленных ссылок в битых файлах.
        if ifd == 0 || !seen.insert(ifd) {
            continue;
        }
        let Some(n) = u16_at(t, ifd) else { continue };
        for i in 0..n as usize {
            let e = ifd + 2 + i * 12;
            let (Some(tag), Some(format), Some(value)) = (u16_at(t, e), u16_at(t, e + 2), u32_at(t, e + 8)) else { break };
            match (group, tag) {
                (ExifTagGroup::GENERIC, 0x8769) => todo.push((value as usize, ExifTagGroup::EXIF)),
                (ExifTagGroup::GENERIC, 0x8825) => todo.push((value as usize, ExifTagGroup::GPS)),
                (ExifTagGroup::EXIF, 0xa005) => todo.push((value as usize, ExifTagGroup::INTEROP)),
                _ => {}
            }
            let Some(expected) = ExifTag::from_u16(tag, &group).ok().map(|t| t.format().as_u16()) else { continue };
            // Пары одного размера: BYTE/SBYTE, SHORT/SSHORT, LONG/SLONG, RATIONAL/SRATIONAL.
            let twin = |f: u16| match f {
                1 => 6,
                6 => 1,
                3 => 8,
                8 => 3,
                4 => 9,
                9 => 4,
                5 => 10,
                10 => 5,
                _ => 0,
            };
            if format != expected && twin(format) == expected {
                let b = if le { expected.to_le_bytes() } else { expected.to_be_bytes() };
                t[e + 2..e + 4].copy_from_slice(&b);
                fixed += 1;
            }
        }
        // Следующий IFD цепочки (IFD1 — миниатюра) — тоже основной группы.
        if group == ExifTagGroup::GENERIC {
            if let Some(next) = u32_at(t, ifd + 2 + n as usize * 12) {
                todo.push((next as usize, ExifTagGroup::GENERIC));
            }
        }
    }
    fixed
}

/// Теги EXIF файла: (IFD, имя); `None` — EXIF нет.
fn tags(data: &[u8]) -> Option<HashSet<(u16, String)>> {
    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(data)).ok()?;
    Some(exif.fields().map(|f| (f.ifd_num.index(), f.tag.to_string())).collect())
}

/// Десятичные градусы → градусы, минуты, секунды (до десятитысячной секунды, ~3 мм).
fn dms(v: f64) -> Vec<uR64> {
    let v = v.abs();
    let deg = v.trunc();
    let min_f = (v - deg) * 60.0;
    let min = min_f.trunc();
    let sec = ((min_f - min) * 60.0 * 10_000.0).round();
    // Округление секунд до 60 не переносим в минуты — значение остаётся верным.
    vec![
        uR64 { nominator: deg as u32, denominator: 1 },
        uR64 { nominator: min as u32, denominator: 1 },
        uR64 { nominator: sec as u32, denominator: 10_000 },
    ]
}

// ---------- Теги EXIF для вкладки «EXIF» ----------

/// Больше значений в теге — поле только для чтения (таблицы, массивы производителя).
const MAX_VALUES: usize = 64;

/// Теги, которые показываются, даже если их нет в файле: их можно добавить.
const COMMON: &[(Group, u16)] = &[
    (Group::Tiff, 0x010e), // ImageDescription
    (Group::Tiff, 0x010f), // Make
    (Group::Tiff, 0x0110), // Model
    (Group::Tiff, 0x0132), // DateTime
    (Group::Tiff, 0x013b), // Artist
    (Group::Tiff, 0x8298), // Copyright
    (Group::Exif, 0x9003), // DateTimeOriginal
    (Group::Exif, 0x9004), // DateTimeDigitized
];

/// Служебные теги: смещения других блоков, данные миниатюры, MakerNote — не редактируются.
const SERVICE: &[(Group, u16)] = &[
    (Group::Tiff, 0x0111),
    (Group::Tiff, 0x0117),
    (Group::Tiff, 0x014a),
    (Group::Tiff, 0x0201),
    (Group::Tiff, 0x0202),
    (Group::Tiff, 0x8769),
    (Group::Tiff, 0x8825),
    (Group::Exif, 0xa005),
    (Group::Exif, 0x927c),
];

/// Теги даты и времени: строго `ГГГГ:ММ:ДД ЧЧ:ММ:СС`.
const DATETIME: &[(Group, u16)] = &[(Group::Tiff, 0x0132), (Group::Exif, 0x9003), (Group::Exif, 0x9004)];

/// Блок (IFD) тега; порядок — порядок в списке.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Group {
    Tiff,
    Exif,
    Gps,
    Interop,
}

impl Group {
    const ALL: [Group; 4] = [Group::Tiff, Group::Exif, Group::Gps, Group::Interop];

    fn code(self) -> &'static str {
        match self {
            Group::Tiff => "tiff",
            Group::Exif => "exif",
            Group::Gps => "gps",
            Group::Interop => "interop",
        }
    }

    fn context(self) -> exif::Context {
        match self {
            Group::Tiff => exif::Context::Tiff,
            Group::Exif => exif::Context::Exif,
            Group::Gps => exif::Context::Gps,
            Group::Interop => exif::Context::Interop,
        }
    }

    fn of(c: exif::Context) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.context() == c)
    }

    fn little(self) -> ExifTagGroup {
        match self {
            Group::Tiff => ExifTagGroup::GENERIC,
            Group::Exif => ExifTagGroup::EXIF,
            Group::Gps => ExifTagGroup::GPS,
            Group::Interop => ExifTagGroup::INTEROP,
        }
    }
}

/// Тег и формат, в котором он пишется. Для известных little_exif тегов — его формат
/// (значение он всё равно приводит к нему), для прочих — формат из файла.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    group: Group,
    tag: u16,
    format: ExifTagFormat,
}

impl Key {
    fn new(group: Group, tag: u16, file_format: Option<ExifTagFormat>) -> Option<Self> {
        let known = ExifTag::from_u16(tag, &group.little()).ok().map(|t| t.format());
        Some(Self { group, tag, format: known.or(file_format)? })
    }

    /// `exif:9003:2` — блок, номер тега, код формата.
    fn code(self) -> String {
        format!("{}:{:04x}:{}", self.group.code(), self.tag, self.format.as_u16())
    }

    fn parse(s: &str) -> Option<Self> {
        let mut it = s.split(':');
        let group = it.next()?;
        let group = Group::ALL.into_iter().find(|g| g.code() == group)?;
        let tag = u16::from_str_radix(it.next()?, 16).ok()?;
        let format = ExifTagFormat::from_u16(it.next()?.parse().ok()?)?;
        it.next().is_none().then_some(Self { group, tag, format })
    }

    fn name(self) -> String {
        exif::Tag(self.group.context(), self.tag).to_string()
    }

    /// Можно ли править: не служебный тег, формат — не «undefined» (байты).
    fn editable(self) -> bool {
        !SERVICE.contains(&(self.group, self.tag)) && self.format != ExifTagFormat::UNDEF
    }

    /// Сколько значений требует тег (если число фиксировано).
    fn count(self) -> Option<usize> {
        let t = ExifTag::from_u16(self.tag, &self.group.little()).ok()?;
        (!t.is_string()).then(|| t.number_of_components() as usize).filter(|&n| n > 0)
    }
}

/// Значение тега — как его прочитал kamadak-exif или как оно разобрано из поля ввода.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Text(String),
    Uint(Vec<u64>),
    Int(Vec<i64>),
    Ratio(Vec<(u64, u64)>),
    SRatio(Vec<(i64, i64)>),
    Float(Vec<f64>),
    Bytes(Vec<u8>),
}

impl Val {
    /// Значение и формат, в котором оно лежит в файле.
    fn of(v: &exif::Value) -> (Self, Option<ExifTagFormat>) {
        use exif::Value as V;
        use ExifTagFormat as F;
        let uint = |v: Vec<u64>, f| (Val::Uint(v), Some(f));
        let int = |v: Vec<i64>, f| (Val::Int(v), Some(f));
        match v {
            V::Byte(v) => uint(v.iter().map(|&x| x.into()).collect(), F::INT8U),
            V::Short(v) => uint(v.iter().map(|&x| x.into()).collect(), F::INT16U),
            V::Long(v) => uint(v.iter().map(|&x| x.into()).collect(), F::INT32U),
            V::SByte(v) => int(v.iter().map(|&x| x.into()).collect(), F::INT8S),
            V::SShort(v) => int(v.iter().map(|&x| x.into()).collect(), F::INT16S),
            V::SLong(v) => int(v.iter().map(|&x| x.into()).collect(), F::INT32S),
            V::Ascii(v) => {
                let parts: Vec<String> = v.iter().map(|s| String::from_utf8_lossy(s).trim_end_matches('\0').to_string()).collect();
                (Val::Text(parts.join(" ")), Some(F::STRING))
            }
            V::Rational(v) => (Val::Ratio(v.iter().map(|r| (r.num.into(), r.denom.into())).collect()), Some(F::RATIONAL64U)),
            V::SRational(v) => (Val::SRatio(v.iter().map(|r| (r.num.into(), r.denom.into())).collect()), Some(F::RATIONAL64S)),
            V::Float(v) => (Val::Float(v.iter().map(|&x| x.into()).collect()), Some(F::FLOAT)),
            V::Double(v) => (Val::Float(v.clone()), Some(F::DOUBLE)),
            V::Undefined(v, _) => (Val::Bytes(v.clone()), Some(F::UNDEF)),
            V::Unknown(..) => (Val::Bytes(Vec::new()), None),
        }
    }

    fn len(&self) -> usize {
        match self {
            Val::Text(_) => 1,
            Val::Uint(v) => v.len(),
            Val::Int(v) => v.len(),
            Val::Ratio(v) => v.len(),
            Val::SRatio(v) => v.len(),
            Val::Float(v) => v.len(),
            Val::Bytes(v) => v.len(),
        }
    }

    /// Текст для поля; длинные массивы обрезаются (такие поля только для чтения).
    fn text(&self) -> String {
        fn join<T>(v: &[T], f: impl Fn(&T) -> String) -> String {
            let mut s: Vec<String> = v.iter().take(MAX_VALUES).map(f).collect();
            if v.len() > MAX_VALUES {
                s.push(format!("… ({})", v.len()));
            }
            s.join(" ")
        }
        fn ratio<T: std::fmt::Display + PartialEq + From<u8>>(n: &T, d: &T) -> String {
            if *d == T::from(1) {
                n.to_string()
            } else {
                format!("{n}/{d}")
            }
        }
        match self {
            Val::Text(s) => s.clone(),
            Val::Uint(v) => join(v, u64::to_string),
            Val::Int(v) => join(v, i64::to_string),
            Val::Ratio(v) => join(v, |(n, d)| ratio(n, d)),
            Val::SRatio(v) => join(v, |(n, d)| ratio(n, d)),
            Val::Float(v) => join(v, f64::to_string),
            Val::Bytes(v) => {
                let text = v.strip_suffix(&[0]).unwrap_or(v);
                if !text.is_empty() && text.iter().all(|b| (0x20..0x7f).contains(b)) {
                    String::from_utf8_lossy(text).into_owned()
                } else {
                    i18n::exif_bytes(v.len())
                }
            }
        }
    }
}

/// Поле вкладки «EXIF».
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Ключ для [`Changes::tags`] и [`check`].
    pub key: String,
    /// Имя тега (как у exiftool: `DateTimeOriginal`).
    pub name: String,
    /// Текст значения; `None` — у выбранных фото значения разные.
    pub value: Option<String>,
    pub editable: bool,
}

/// Теги основного изображения в файле (без координат — они на своей вкладке).
fn file_tags(path: &Path) -> Vec<(Group, u16, Val, Option<ExifTagFormat>)> {
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) else {
        return Vec::new();
    };
    exif.fields()
        .filter(|f| f.ifd_num == In::PRIMARY)
        .filter_map(|f| {
            let group = Group::of(f.tag.context())?;
            let tag = f.tag.number();
            if group == Group::Gps && (1..=4).contains(&tag) {
                return None;
            }
            let (val, format) = Val::of(&f.value);
            Some((group, tag, val, format))
        })
        .collect()
}

/// Теги одного файла: (блок, номер) → значение и формат.
type FileTags = HashMap<(Group, u16), (Val, Option<ExifTagFormat>)>;

/// Поля вкладки «EXIF» для фото `paths`: все теги их файлов и частые теги ([`COMMON`]).
/// Тег, которого у части фото нет, — «разные значения». Читает файлы — не из главного потока.
pub fn fields(paths: &[PathBuf]) -> Vec<Field> {
    use rayon::prelude::*;
    let files: Vec<FileTags> =
        paths.par_iter().map(|p| file_tags(p).into_iter().map(|(g, t, v, f)| ((g, t), (v, f))).collect()).collect();
    let all_writable = paths.iter().all(|p| writable(p));
    let mut ids: Vec<(Group, u16)> = files.iter().flat_map(|f| f.keys().copied()).chain(COMMON.iter().copied()).collect();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .filter_map(|(group, tag)| {
            let present: Vec<&(Val, Option<ExifTagFormat>)> = files.iter().filter_map(|f| f.get(&(group, tag))).collect();
            let key = Key::new(group, tag, present.iter().find_map(|v| v.1))?;
            let texts: Vec<String> = files.iter().map(|f| f.get(&(group, tag)).map(|v| v.0.text()).unwrap_or_default()).collect();
            let value = texts.iter().all(|t| *t == texts[0]).then(|| texts.first().cloned().unwrap_or_default());
            let editable = all_writable
                && key.editable()
                && present.iter().all(|(v, _)| !matches!(v, Val::Bytes(_)) && v.len() <= MAX_VALUES);
            Some(Field { key: key.code(), name: key.name(), value, editable })
        })
        .collect()
}

/// Проверить текст поля: `Err` — пояснение, что не так.
pub fn check(key: &str, text: &str) -> std::result::Result<(), String> {
    let key = Key::parse(key).ok_or_else(|| key.to_string())?;
    if text.is_empty() {
        return Ok(());
    }
    encode(key, text).map(|_| ())
}

/// Текст поля → значение (для проверки после записи) и сырые байты для little_exif.
fn encode(key: Key, text: &str) -> std::result::Result<(Val, Vec<u8>), String> {
    use ExifTagFormat as F;
    if key.format == F::STRING {
        if text.contains('\0') {
            return Err(i18n::exif_expect_text().into());
        }
        if DATETIME.contains(&(key.group, key.tag)) && chrono::NaiveDateTime::parse_from_str(text, "%Y:%m:%d %H:%M:%S").is_err() {
            return Err(i18n::exif_expect_datetime().into());
        }
        return Ok((Val::Text(text.to_string()), text.as_bytes().to_vec()));
    }
    let items: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|s| !s.is_empty()).collect();
    if let Some(n) = key.count().filter(|&n| n != items.len()) {
        return Err(i18n::exif_expect_count(n));
    }
    if items.is_empty() {
        return Err(i18n::exif_expect_numbers().into());
    }
    let bad = || i18n::exif_expect_numbers().to_string();
    let mut raw = Vec::new();
    let val = match key.format {
        F::INT8U | F::INT16U | F::INT32U => {
            let max = match key.format {
                F::INT8U => u8::MAX.into(),
                F::INT16U => u16::MAX.into(),
                _ => u64::from(u32::MAX),
            };
            let v = items.iter().map(|s| s.parse::<u64>().ok().filter(|&x| x <= max)).collect::<Option<Vec<_>>>().ok_or_else(bad)?;
            for &x in &v {
                match key.format {
                    F::INT8U => raw.push(x as u8),
                    F::INT16U => raw.extend((x as u16).to_le_bytes()),
                    _ => raw.extend((x as u32).to_le_bytes()),
                }
            }
            Val::Uint(v)
        }
        F::INT8S | F::INT16S | F::INT32S => {
            let (min, max): (i64, i64) = match key.format {
                F::INT8S => (i8::MIN.into(), i8::MAX.into()),
                F::INT16S => (i16::MIN.into(), i16::MAX.into()),
                _ => (i32::MIN.into(), i32::MAX.into()),
            };
            let v = items.iter().map(|s| s.parse::<i64>().ok().filter(|x| (min..=max).contains(x))).collect::<Option<Vec<_>>>().ok_or_else(bad)?;
            for &x in &v {
                match key.format {
                    F::INT8S => raw.push(x as i8 as u8),
                    F::INT16S => raw.extend((x as i16).to_le_bytes()),
                    _ => raw.extend((x as i32).to_le_bytes()),
                }
            }
            Val::Int(v)
        }
        F::RATIONAL64U => {
            let v = items
                .iter()
                .map(|s| parse_ratio(s).filter(|&(n, d)| n >= 0 && n <= i64::from(u32::MAX) && d <= i64::from(u32::MAX)).map(|(n, d)| (n as u64, d as u64)))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(bad)?;
            for &(n, d) in &v {
                raw.extend((n as u32).to_le_bytes());
                raw.extend((d as u32).to_le_bytes());
            }
            Val::Ratio(v)
        }
        F::RATIONAL64S => {
            let v = items
                .iter()
                .map(|s| parse_ratio(s).filter(|&(n, d)| (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&n) && d <= i64::from(i32::MAX)))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(bad)?;
            for &(n, d) in &v {
                raw.extend((n as i32).to_le_bytes());
                raw.extend((d as i32).to_le_bytes());
            }
            Val::SRatio(v)
        }
        F::FLOAT => {
            let v = items.iter().map(|s| s.parse::<f32>().ok().filter(|x| x.is_finite())).collect::<Option<Vec<_>>>().ok_or_else(bad)?;
            v.iter().for_each(|x| raw.extend(x.to_le_bytes()));
            Val::Float(v.iter().map(|&x| x.into()).collect())
        }
        F::DOUBLE => {
            let v = items.iter().map(|s| s.parse::<f64>().ok().filter(|x| x.is_finite())).collect::<Option<Vec<_>>>().ok_or_else(bad)?;
            v.iter().for_each(|x| raw.extend(x.to_le_bytes()));
            Val::Float(v)
        }
        F::STRING | F::UNDEF => return Err(bad()),
    };
    Ok((val, raw))
}

/// `n/d`, целое или десятичная дробь (`2.8` → `14/5`); знаменатель положительный.
fn parse_ratio(s: &str) -> Option<(i64, i64)> {
    if let Some((n, d)) = s.split_once('/') {
        let (n, d) = (n.parse().ok()?, d.parse().ok()?);
        return (d > 0).then_some((n, d));
    }
    let s = s.replace(',', ".");
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    if frac.len() > 6 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let d = 10i64.pow(frac.len() as u32);
    let neg = int.starts_with('-');
    let whole: i64 = if int.is_empty() || int == "-" { 0 } else { int.parse().ok()? };
    let part: i64 = if frac.is_empty() { 0 } else { frac.parse().ok()? };
    let n = whole.checked_mul(d)?.checked_add(if neg { -part } else { part })?;
    let g = gcd(n.unsigned_abs(), d as u64).max(1) as i64;
    Some((n / g, d / g))
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Записать во временный файл рядом и переименовать: при сбое оригинал остаётся целым.
fn replace_file(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = temp_path(path);
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        let md = std::fs::metadata(path)?;
        keep_created(&f, &md);
        drop(f);
        std::fs::set_permissions(&tmp, md.permissions())?;
        copy_attributes(path, &tmp);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

/// Дата создания файла: новый файл заменяет старый, а «создан» в Finder и Проводнике
/// должно остаться прежним. В Linux её задать нельзя.
#[cfg(any(target_os = "macos", windows))]
fn keep_created(f: &std::fs::File, original: &std::fs::Metadata) {
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::FileTimesExt;
    #[cfg(windows)]
    use std::os::windows::fs::FileTimesExt;
    if let Ok(created) = original.created() {
        let _ = f.set_times(std::fs::FileTimes::new().set_created(created));
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn keep_created(_f: &std::fs::File, _original: &std::fs::Metadata) {}

/// Расширенные атрибуты и ACL прежнего файла (метки Finder, «откуда загружен») — на новый.
/// Не получилось — не ошибка: сами данные записаны.
#[cfg(target_os = "macos")]
fn copy_attributes(from: &Path, to: &Path) {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let (Ok(from), Ok(to)) = (CString::new(from.as_os_str().as_bytes()), CString::new(to.as_os_str().as_bytes())) else { return };
    // SAFETY: обе строки — завершённые нулём пути, состояние копирования не используется.
    unsafe {
        libc::copyfile(from.as_ptr(), to.as_ptr(), std::ptr::null_mut(), libc::COPYFILE_XATTR | libc::COPYFILE_ACL);
    }
}

#[cfg(not(target_os = "macos"))]
fn copy_attributes(_from: &Path, _to: &Path) {}

/// Временный файл рядом с оригиналом; имя своё у каждой записи — две записи одного файла
/// (или две программы) не пишут в один и тот же.
fn temp_path(path: &Path) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Скрытый файл (с точкой) сканер пропускает.
    path.with_file_name(format!(".{name}.{}-{n}.gpl-tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::jpeg::JpegEncoder, RgbImage};

    fn jpeg() -> Vec<u8> {
        let mut out = Vec::new();
        JpegEncoder::new(&mut out).encode_image(&RgbImage::new(16, 16)).unwrap();
        out
    }

    #[test]
    fn dms_roundtrip() {
        for v in [55.751244, -33.8568, 151.2153, -0.000123, 179.999999] {
            let d: Vec<f64> = dms(v).iter().map(|r| r.nominator as f64 / r.denominator as f64).collect();
            let back = metadata::dms_to_decimal(&d, v < 0.0).unwrap();
            assert!((back - v).abs() < 1e-6, "{v} → {back}");
        }
    }

    /// Запись через символическую ссылку меняет сам файл, ссылка остаётся ссылкой.
    #[cfg(unix)]
    #[test]
    fn write_through_symlink() {
        let dir = std::env::temp_dir().join(format!("gpl-geotag-link-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("real")).unwrap();
        let (real, link) = (dir.join("real/a.jpg"), dir.join("link.jpg"));
        std::fs::write(&real, jpeg()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_coords(&link, 10.5, 20.5).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert!((metadata::read(&real).lat.unwrap() - 10.5).abs() < 1e-6);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Замена файла сохраняет расширенные атрибуты и дату создания.
    #[cfg(target_os = "macos")]
    #[test]
    fn attributes_survive() {
        let dir = std::env::temp_dir().join(format!("gpl-geotag-attr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jpg");
        std::fs::write(&path, jpeg()).unwrap();
        let xattr = |args: &[&str]| std::process::Command::new("xattr").args(args).arg(&path).output().unwrap();
        assert!(xattr(&["-w", "user.gpl-test", "метка"]).status.success());
        let created = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        {
            use std::os::macos::fs::FileTimesExt;
            let f = std::fs::File::options().write(true).open(&path).unwrap();
            f.set_times(std::fs::FileTimes::new().set_created(created)).unwrap();
        }
        write_coords(&path, 10.5, 20.5).unwrap();
        assert_eq!(String::from_utf8_lossy(&xattr(&["-p", "user.gpl-test"]).stdout).trim(), "метка");
        assert_eq!(std::fs::metadata(&path).unwrap().created().unwrap(), created);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_jpeg_without_and_with_exif() {
        let dir = std::env::temp_dir().join(format!("gpl-geotag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.JPG");
        std::fs::write(&path, jpeg()).unwrap();
        assert_eq!(metadata::read(&path).lat, None);

        let stamp = write_coords(&path, -33.8568, 151.2153).unwrap();
        assert_eq!(stamp, Stamp::of(&path).unwrap());
        let m = metadata::read(&path);
        assert!((m.lat.unwrap() + 33.8568).abs() < 1e-6 && (m.lon.unwrap() - 151.2153).abs() < 1e-6);

        // Повторная запись меняет координаты, другие теги остаются.
        write_coords(&path, 55.75, -37.62).unwrap();
        let m = metadata::read(&path);
        assert!((m.lat.unwrap() - 55.75).abs() < 1e-6 && (m.lon.unwrap() + 37.62).abs() < 1e-6);
        assert!(!std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".gpl-tmp")));
        // Картинка по-прежнему читается.
        image::open(&path).unwrap();

        assert!(write_coords(&path, 91.0, 0.0).is_err());
        let png = dir.join("b.png");
        RgbImage::new(4, 4).save(&png).unwrap();
        assert!(!writable(&png));
        assert!(write_coords(&png, 10.0, 10.0).is_err());
        assert!(writable(Path::new("/x/IMG_1.HEIC")));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn field<'a>(fields: &'a [Field], name: &str) -> &'a Field {
        fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("нет поля {name}"))
    }

    #[test]
    fn edit_tags() {
        let dir = std::env::temp_dir().join(format!("gpl-exif-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.jpg"), dir.join("b.jpg"));
        std::fs::write(&a, jpeg()).unwrap();
        std::fs::write(&b, jpeg()).unwrap();

        // Без EXIF — только частые теги, пустые и доступные для правки.
        let empty = fields(std::slice::from_ref(&a));
        assert!(empty.iter().all(|f| f.value.as_deref() == Some("") && f.editable));
        let artist = field(&empty, "Artist").key.clone();
        let date = field(&empty, "DateTimeOriginal").key.clone();
        assert_eq!(check(&date, "2024:01:02"), Err(i18n::exif_expect_datetime().into()));

        // У a — автор, дата и ориентация (число), у b — только автор и координаты.
        let orientation = Key::new(Group::Tiff, 0x0112, None).unwrap().code();
        assert_eq!(check(&orientation, "1 2"), Err(i18n::exif_expect_count(1)));
        assert_eq!(check(&orientation, "x"), Err(i18n::exif_expect_numbers().into()));
        let tags = |v: &[(&str, &str)]| v.iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
        write(&a, &Changes { coords: None, tags: tags(&[(&artist, "Я"), (&date, "2024:01:02 03:04:05"), (&orientation, "6")]) }).unwrap();
        write(&b, &Changes { coords: Some((10.0, 20.0)), tags: tags(&[(&artist, "Я")]) }).unwrap();
        let m = metadata::read(&a);
        assert_eq!(m.orientation, 6);
        assert_eq!(m.taken_at.unwrap().to_string(), "2024-01-02 03:04:05");

        let both = fields(&[a.clone(), b.clone()]);
        assert_eq!(field(&both, "Artist").value.as_deref(), Some("Я"));
        assert_eq!(field(&both, "DateTimeOriginal").value, None);
        assert_eq!(field(&both, "Orientation").value, None);
        // Координаты — на своей вкладке.
        assert!(both.iter().all(|f| f.name != "GPSLatitude"));

        // Изменение одного тега у пачки: остальное у каждого файла — своё.
        let copyright = field(&both, "Copyright").key.clone();
        for p in [&a, &b] {
            write(p, &Changes { coords: None, tags: tags(&[(&copyright, "CC BY")]) }).unwrap();
        }
        let both = fields(&[a.clone(), b.clone()]);
        assert_eq!(field(&both, "Copyright").value.as_deref(), Some("CC BY"));
        assert_eq!(field(&both, "DateTimeOriginal").value, None);
        assert_eq!(metadata::read(&b).lat, Some(10.0));
        assert_eq!(metadata::read(&a).orientation, 6);

        // Пустой текст удаляет тег; дробь пишется как есть, десятичная — дробью.
        let exposure = Key::new(Group::Exif, 0x829a, None).unwrap().code();
        let fnumber = Key::new(Group::Exif, 0x829d, None).unwrap().code();
        write(&a, &Changes { coords: None, tags: tags(&[(&artist, ""), (&exposure, "1/250"), (&fnumber, "2.8")]) }).unwrap();
        let one = fields(std::slice::from_ref(&a));
        assert_eq!(field(&one, "Artist").value.as_deref(), Some(""));
        assert_eq!(field(&one, "ExposureTime").value.as_deref(), Some("1/250"));
        assert_eq!(field(&one, "FNumber").value.as_deref(), Some("14/5"));
        // Байтовые (undefined) и служебные теги не правятся.
        assert!(!Key::new(Group::Exif, 0x9000, None).unwrap().editable());
        assert!(!Key::new(Group::Tiff, 0x8769, None).unwrap().editable());
        image::open(&a).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrong_signedness_is_fixed() {
        let dir = std::env::temp_dir().join(format!("gpl-exif-sign-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jpg");
        std::fs::write(&path, jpeg()).unwrap();
        let width = Key::new(Group::Exif, 0xa002, None).unwrap().code();
        write(&path, &Changes { coords: None, tags: vec![(width, "16".into())] }).unwrap();
        // Как у некоторых камер: ExifImageWidth записан как SLONG (9), а не LONG (4).
        let mut data = std::fs::read(&path).unwrap();
        let at = data.windows(4).position(|w| w == [0x02, 0xa0, 0x04, 0x00]).unwrap();
        data[at + 2] = 9;
        std::fs::write(&path, &data).unwrap();
        assert!(decode(&data, FileExtension::JPEG).is_err());

        write_coords(&path, 55.04, 82.92).unwrap();
        let m = metadata::read(&path);
        assert!((m.lat.unwrap() - 55.04).abs() < 1e-6);
        let one = fields(std::slice::from_ref(&path));
        assert_eq!(field(&one, "PixelXDimension").value.as_deref(), Some("16"));
        image::open(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ratios_and_keys() {
        assert_eq!(parse_ratio("2.8"), Some((14, 5)));
        assert_eq!(parse_ratio("-0,5"), Some((-1, 2)));
        assert_eq!(parse_ratio("1/0"), None);
        assert_eq!(parse_ratio("10"), Some((10, 1)));
        let k = Key::new(Group::Exif, 0x9003, None).unwrap();
        assert_eq!(k.code(), "exif:9003:2");
        assert_eq!(Key::parse(&k.code()), Some(k));
        assert_eq!(Key::parse("exif:9003"), None);
        assert_eq!(k.name(), "DateTimeOriginal");
    }
}

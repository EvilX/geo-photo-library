//! C-интерфейс ядра для нативного приложения macOS (Swift, см. `macos/`).
//! Заголовок с теми же объявлениями — `macos/Sources/CGeoPhotoCore/include/gpl.h`:
//! при изменении функций и структур править оба файла.
//!
//! Правила для вызывающей стороны:
//! - строки на входе — UTF-8 с завершающим нулём; `NULL` допустим там, где это указано;
//! - строку, которую вернула библиотека (`*mut c_char`), освобождать [`gpl_string_free`];
//! - списки и снимки карты освобождать своей функцией `*_free`; указатели внутрь них
//!   действительны до этого момента;
//! - функции не держат общего состояния (каждый запрос сам открывает БД), поэтому их
//!   можно вызывать из любого потока; тяжёлые (`gpl_scan`, запросы к БД) — не из главного.
#![allow(clippy::missing_safety_doc)]

use crate::{
    config::{snap_thumb_scale, Config, MapView, SortOrder},
    db::{DateCount, Day, Db, Photo},
    geo, i18n, metadata, paths, scanner, thumbs, trace,
};
use image::RgbImage;
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::{Path, PathBuf},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

unsafe fn str_arg<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn c_string(s: impl AsRef<str>) -> CString {
    // Нулевой байт внутри строки (в пути его быть не может) — обрезаем по нему.
    let bytes = s.as_ref().as_bytes();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    CString::new(&bytes[..end]).unwrap_or_default()
}

fn out_string(s: impl AsRef<str>) -> *mut c_char {
    c_string(s).into_raw()
}

unsafe fn set_err(err: *mut *mut c_char, e: &anyhow::Error) {
    if !err.is_null() {
        *err = out_string(format!("{e:#}"));
    }
}

#[no_mangle]
pub unsafe extern "C" fn gpl_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Язык интерфейса (`ru`, `el-GR`…; неизвестный — английский). Вызвать один раз
/// при запуске, до остальных функций.
#[no_mangle]
pub unsafe extern "C" fn gpl_init(lang: *const c_char) {
    i18n::set_lang(str_arg(lang).unwrap_or("en"));
}

// ---------- Строки интерфейса ----------

/// Строка интерфейса по ключу — имени функции в `i18n.rs`; `NULL` для неизвестного ключа.
#[no_mangle]
pub unsafe extern "C" fn gpl_text(key: *const c_char) -> *mut c_char {
    let text = match str_arg(key).unwrap_or("") {
        "tab_dates" => i18n::tab_dates(),
        "tab_map" => i18n::tab_map(),
        "settings" => i18n::settings(),
        "app_summary" => i18n::app_summary(),
        "rescan" => i18n::rescan(),
        "searching" => i18n::searching(),
        "stopping_scan" => i18n::stopping_scan(),
        "view" => i18n::view(),
        "browse" => i18n::browse(),
        "photo_folder" => i18n::photo_folder(),
        "folder_hint" => i18n::folder_hint(),
        "folder_missing" => i18n::folder_missing(),
        "no_photos" => i18n::no_photos(),
        "no_geotagged" => i18n::no_geotagged(),
        "show_in_finder" => i18n::show_in_finder(),
        "about" => i18n::about(),
        "copy" => i18n::copy(),
        "edit" => i18n::edit(),
        "delete" => i18n::delete(),
        "info" => i18n::info(),
        "show_on_map" => i18n::show_on_map(),
        "no_coordinates" => i18n::no_coordinates(),
        "ok" => i18n::ok(),
        "close" => i18n::close(),
        "select_photo" => i18n::select_photo(),
        "sort_newest" => i18n::sort_label(SortOrder::Newest),
        "sort_oldest" => i18n::sort_label(SortOrder::Oldest),
        "sort_tooltip" => i18n::sort_tooltip(),
        "thumb_size_tooltip" => i18n::thumb_size_tooltip(),
        "thumbs_smaller" => i18n::thumbs_smaller(),
        "thumbs_larger" => i18n::thumbs_larger(),
        "photo_viewer" => i18n::photo_viewer(),
        "photo_editor" => i18n::photo_editor(),
        "automatic" => i18n::automatic(),
        "apps_hint" => i18n::apps_hint_mac(),
        "search_place" => i18n::search_place(),
        "place_not_found" => i18n::place_not_found(),
        "place_search_failed" => i18n::place_search_failed(),
        "my_location" => i18n::my_location(),
        "all_time" => i18n::all_time(),
        "select_all" => i18n::select_all(),
        "cancel" => i18n::cancel(),
        "move_to_trash" => i18n::move_to_trash(),
        "confirm_trash_detail" => i18n::confirm_trash_detail(),
        "filter_by_date" => i18n::filter_by_date(),
        "location_failed" => i18n::location_failed(),
        "location_unavailable" => i18n::location_unavailable(),
        "location_denied" => i18n::location_denied(),
        "location_purpose" => i18n::location_purpose(),
        _ => return ptr::null_mut(),
    };
    out_string(text)
}

#[no_mangle]
pub extern "C" fn gpl_text_photos(n: usize) -> *mut c_char {
    out_string(i18n::photos(n))
}

#[no_mangle]
pub extern "C" fn gpl_text_processing(done: usize, total: usize) -> *mut c_char {
    out_string(i18n::processing(done, total))
}

#[no_mangle]
pub extern "C" fn gpl_text_scan_summary(stats: GplScanStats) -> *mut c_char {
    out_string(i18n::scan_summary(stats.found, stats.added, stats.updated, stats.removed))
}

#[no_mangle]
pub unsafe extern "C" fn gpl_text_scan_error(e: *const c_char) -> *mut c_char {
    out_string(i18n::scan_error(str_arg(e).unwrap_or("")))
}

#[no_mangle]
pub extern "C" fn gpl_text_in_map_area(visible: usize, total: usize) -> *mut c_char {
    out_string(i18n::in_map_area(visible, total))
}

#[no_mangle]
pub unsafe extern "C" fn gpl_text_no_coordinates_detail(name: *const c_char) -> *mut c_char {
    out_string(i18n::no_coordinates_detail(str_arg(name).unwrap_or("")))
}

#[no_mangle]
pub unsafe extern "C" fn gpl_text_trashed(name: *const c_char) -> *mut c_char {
    out_string(i18n::trashed(str_arg(name).unwrap_or("")))
}

#[no_mangle]
pub unsafe extern "C" fn gpl_text_confirm_trash(subject: *const c_char) -> *mut c_char {
    out_string(i18n::confirm_trash(str_arg(subject).unwrap_or("")))
}

#[no_mangle]
pub unsafe extern "C" fn gpl_text_trash_failed(name: *const c_char, e: *const c_char) -> *mut c_char {
    out_string(i18n::trash_failed(str_arg(name).unwrap_or(""), str_arg(e).unwrap_or("")))
}

/// Подпись узла дерева дат: год; месяц (`day == 0`); день.
#[no_mangle]
pub extern "C" fn gpl_text_date_label(year: i32, month: u32, day: u32) -> *mut c_char {
    out_string(match (valid_month(month), day) {
        (Some(m), 0) => i18n::month(m).to_string(),
        (Some(m), d) => i18n::day(m, d),
        _ => year.to_string(),
    })
}

/// Заголовок выбранного периода: «2023 год», «Март 2023», «8 марта 2023».
#[no_mangle]
pub extern "C" fn gpl_text_date_title(year: i32, month: u32, day: u32) -> *mut c_char {
    out_string(match (valid_month(month), day) {
        (Some(m), 0) => format!("{} {year}", i18n::month(m)),
        (Some(m), d) => i18n::full_day(year, m, d),
        _ => i18n::year(year),
    })
}

/// Названия месяцев берутся из таблицы по индексу — чужой номер не должен ронять приложение.
fn valid_month(m: u32) -> Option<u32> {
    (1..=12).contains(&m).then_some(m)
}

// ---------- Настройки ----------

/// Папка библиотеки из настроек; `NULL`, если не задана.
#[no_mangle]
pub extern "C" fn gpl_config_library_path() -> *mut c_char {
    match Config::load().library_path.as_deref().and_then(Path::to_str) {
        Some(p) => out_string(p),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn gpl_config_set_library_path(path: *const c_char, err: *mut *mut c_char) -> bool {
    let mut config = Config::load();
    config.library_path = str_arg(path).filter(|p| !p.is_empty()).map(PathBuf::from);
    save(&config, err)
}

/// Положение карты, сохранённое при выходе; `false` — его нет (показать все фото).
#[no_mangle]
pub unsafe extern "C" fn gpl_config_map_view(lat: *mut f64, lon: *mut f64, zoom: *mut f64) -> bool {
    let Some(view) = Config::load().map_view.and_then(MapView::valid) else { return false };
    *lat = view.lat;
    *lon = view.lon;
    *zoom = view.zoom;
    true
}

#[no_mangle]
pub unsafe extern "C" fn gpl_config_set_map_view(lat: f64, lon: f64, zoom: f64, err: *mut *mut c_char) -> bool {
    let Some(view) = (MapView { lat, lon, zoom }).valid() else { return false };
    let mut config = Config::load();
    config.map_view = Some(view);
    save(&config, err)
}

/// Сортировка фото в сетках: `true` — новые сверху.
#[no_mangle]
pub extern "C" fn gpl_config_newest_first() -> bool {
    Config::load().sort == SortOrder::Newest
}

#[no_mangle]
pub unsafe extern "C" fn gpl_config_set_newest_first(newest: bool, err: *mut *mut c_char) -> bool {
    let mut config = Config::load();
    config.sort = if newest { SortOrder::Newest } else { SortOrder::Oldest };
    save(&config, err)
}

/// Масштаб миниатюр в процентах, приведённый к допустимому диапазону и шагу.
#[no_mangle]
pub extern "C" fn gpl_config_thumb_scale() -> u32 {
    snap_thumb_scale(Config::load().thumb_scale as f64)
}

#[no_mangle]
pub unsafe extern "C" fn gpl_config_set_thumb_scale(percent: u32, err: *mut *mut c_char) -> bool {
    let mut config = Config::load();
    config.thumb_scale = snap_thumb_scale(percent as f64);
    save(&config, err)
}

/// Программа из настроек: `editor` — редактор, иначе просмотрщик. На macOS это
/// идентификатор пакета приложения; `NULL` — выбирать автоматически.
#[no_mangle]
pub extern "C" fn gpl_config_app(editor: bool) -> *mut c_char {
    let config = Config::load();
    match if editor { config.editor } else { config.viewer } {
        Some(id) => out_string(id),
        None => ptr::null_mut(),
    }
}

/// `id == NULL` или пустой — выбирать автоматически.
#[no_mangle]
pub unsafe extern "C" fn gpl_config_set_app(editor: bool, id: *const c_char, err: *mut *mut c_char) -> bool {
    let mut config = Config::load();
    let id = str_arg(id).filter(|s| !s.is_empty()).map(str::to_owned);
    if editor {
        config.editor = id;
    } else {
        config.viewer = id;
    }
    save(&config, err)
}

unsafe fn save(config: &Config, err: *mut *mut c_char) -> bool {
    match config.save() {
        Ok(()) => true,
        Err(e) => {
            set_err(err, &e);
            false
        }
    }
}

// ---------- Сканирование ----------

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GplScanStats {
    pub found: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// Флаг отмены сканирования: выставляется из другого потока.
pub struct GplCancel(AtomicBool);

#[no_mangle]
pub extern "C" fn gpl_cancel_new() -> *mut GplCancel {
    Box::into_raw(Box::new(GplCancel(AtomicBool::new(false))))
}

#[no_mangle]
pub unsafe extern "C" fn gpl_cancel_set(cancel: *const GplCancel) {
    if let Some(c) = cancel.as_ref() {
        c.0.store(true, Ordering::Relaxed);
    }
}

/// Освобождать только после возврата из `gpl_scan`, которому передан флаг.
#[no_mangle]
pub unsafe extern "C" fn gpl_cancel_free(cancel: *mut GplCancel) {
    if !cancel.is_null() {
        drop(Box::from_raw(cancel));
    }
}

pub type GplProgress = unsafe extern "C" fn(ctx: *mut c_void, done: usize, total: usize);

pub const GPL_SCAN_OK: c_int = 0;
pub const GPL_SCAN_CANCELLED: c_int = 1;
pub const GPL_SCAN_ERROR: c_int = 2;

/// Указатель контекста обратного вызова: вызывающая сторона отвечает за то, что
/// `progress` можно вызывать с ним из потока сканирования.
struct Ctx(*mut c_void);
unsafe impl Sync for Ctx {}

/// Сканирует папку `root` в БД приложения; блокирует поток до конца. `progress`
/// вызывается из этого же потока. Возвращает `GPL_SCAN_*`; при ошибке её текст — в `err`.
#[no_mangle]
pub unsafe extern "C" fn gpl_scan(
    root: *const c_char,
    progress: Option<GplProgress>,
    ctx: *mut c_void,
    cancel: *const GplCancel,
    stats: *mut GplScanStats,
    err: *mut *mut c_char,
) -> c_int {
    let Some(root) = str_arg(root) else { return GPL_SCAN_ERROR };
    let ctx = Ctx(ctx);
    let report = |done: usize, total: usize| {
        let ctx = &ctx;
        if let Some(f) = progress {
            unsafe { f(ctx.0, done, total) }
        }
    };
    let never = AtomicBool::new(false);
    let cancel = cancel.as_ref().map_or(&never, |c| &c.0);
    match scanner::scan(&paths::db_path(), Path::new(root), &paths::thumbs_dir(), &report, cancel) {
        Ok(s) => {
            if !stats.is_null() {
                *stats = GplScanStats { found: s.found, added: s.added, updated: s.updated, removed: s.removed };
            }
            GPL_SCAN_OK
        }
        Err(e) if e.is::<scanner::Cancelled>() => GPL_SCAN_CANCELLED,
        Err(e) => {
            set_err(err, &e);
            GPL_SCAN_ERROR
        }
    }
}

// ---------- Запросы к БД ----------

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GplDateCount {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub count: u32,
}

#[repr(C)]
pub struct GplPhoto {
    pub id: i64,
    pub path: *const c_char,
    /// `YYYY-MM-DD HH:MM:SS`
    pub taken_at: *const c_char,
    /// `NULL`, если миниатюру создать не удалось.
    pub thumb_path: *const c_char,
    pub lat: f64,
    pub lon: f64,
    pub has_coords: bool,
}

/// Список фото: строки, на которые указывают элементы, живут вместе со списком.
pub struct GplPhotos {
    rows: Vec<GplPhoto>,
    _strings: Vec<CString>,
    /// Проекция на зум 0 — считается один раз, чтобы обновление карты шло без тригонометрии.
    points: Vec<(f64, f64)>,
    /// День съёмки каждого фото — для дерева фильтра карты по датам.
    days: Vec<Option<Day>>,
}

impl GplPhotos {
    fn new(photos: Vec<Photo>) -> Self {
        let mut strings = Vec::with_capacity(photos.len() * 3);
        let mut keep = |s: &str| {
            let c = c_string(s);
            let p = c.as_ptr();
            strings.push(c);
            p
        };
        let rows: Vec<GplPhoto> = photos
            .iter()
            .map(|p| {
                let (lat, lon) = p.coords().unwrap_or((0.0, 0.0));
                GplPhoto {
                    id: p.id,
                    path: keep(&p.path),
                    taken_at: keep(&p.taken_at),
                    thumb_path: p.thumb_path.as_deref().map_or(ptr::null(), &mut keep),
                    lat,
                    lon,
                    has_coords: p.coords().is_some(),
                }
            })
            .collect();
        let points = rows.iter().map(|r| geo::project(r.lat, r.lon, 0.0)).collect();
        let days = photos.iter().map(Photo::day).collect();
        Self { rows, _strings: strings, points, days }
    }
}

fn open_db() -> anyhow::Result<Db> {
    Db::open(&paths::db_path())
}

/// Ошибки запросов (БД ещё нет, занята) — в stderr; интерфейсу достаточно пустого результата.
fn query<T: Default>(what: &str, f: impl FnOnce(&Db) -> anyhow::Result<T>) -> T {
    open_db().and_then(|db| f(&db)).unwrap_or_else(|e| {
        eprintln!("БД, {what}: {e:#}");
        T::default()
    })
}

#[no_mangle]
pub extern "C" fn gpl_db_count() -> i64 {
    query("число фото", Db::count)
}

/// Число фото по дням, по возрастанию даты. Освобождать `gpl_date_counts_free`.
#[no_mangle]
pub unsafe extern "C" fn gpl_db_date_counts(len: *mut usize) -> *mut GplDateCount {
    date_counts_out(query("даты", Db::date_counts), len)
}

unsafe fn date_counts_out(counts: Vec<DateCount>, len: *mut usize) -> *mut GplDateCount {
    let counts: Box<[GplDateCount]> = counts
        .into_iter()
        .map(|c| GplDateCount { year: c.year, month: c.month, day: c.day, count: c.count })
        .collect();
    *len = counts.len();
    Box::into_raw(counts).cast()
}

#[no_mangle]
pub unsafe extern "C" fn gpl_date_counts_free(counts: *mut GplDateCount, len: usize) {
    if !counts.is_null() {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(counts, len)));
    }
}

/// Фото за год, месяц (`day == 0`) или день (`month == 0` — весь год), по дате съёмки.
#[no_mangle]
pub extern "C" fn gpl_db_photos_by_date(year: i32, month: u32, day: u32) -> *mut GplPhotos {
    let some = |v: u32| (v != 0).then_some(v);
    let photos = query("фото по дате", |db| db.photos_by_date(year, some(month), some(day)));
    Box::into_raw(Box::new(GplPhotos::new(photos)))
}

/// Фото с координатами за год, месяц (`day == 0`) или день (`month == 0` — весь год),
/// `year == 0` — за всё время; по дате съёмки.
#[no_mangle]
pub extern "C" fn gpl_db_geotagged(year: i32, month: u32, day: u32) -> *mut GplPhotos {
    let some = |v: u32| (v != 0).then_some(v);
    let year = (year != 0).then_some(year);
    let photos = query("фото с координатами", |db| db.geotagged_by_date(year, some(month), some(day)));
    Box::into_raw(Box::new(GplPhotos::new(photos)))
}

/// Убрать из БД фото, перемещённое в корзину, вместе с его миниатюрой.
#[no_mangle]
pub extern "C" fn gpl_db_remove(id: i64) {
    if let Some(thumb) = query("удаление фото", |db| db.remove(id)) {
        let _ = std::fs::remove_file(thumb);
    }
}

/// Данные для панели «Информация» — JSON: `sections` (разделы `title` + `rows`),
/// `note` (пояснение, если EXIF нет) и `all` (`title` + `rows` всех тегов EXIF).
/// Строка — `key`, `value` и, для даты съёмки, `date` (`YYYY-MM-DD HH:MM:SS`): её
/// интерфейс форматирует по системной локали, а `value` (часовой пояс) дописывает в скобках.
/// Читает файл — вызывать не из главного потока.
#[no_mangle]
pub unsafe extern "C" fn gpl_photo_info(path: *const c_char, has_coords: bool, lat: f64, lon: f64) -> *mut c_char {
    let path = Path::new(str_arg(path).unwrap_or(""));
    out_string(photo_info(path, has_coords.then_some((lat, lon))).to_string())
}

fn photo_info(path: &Path, coords: Option<(f64, f64)>) -> serde_json::Value {
    use i18n::Info;
    use serde_json::json;

    let row = |key: Info, value: Option<String>| Some(json!({ "key": i18n::info_label(key), "value": value? }));
    // Пустые поля пропускаются, пустой раздел не показывается.
    let section = |title: Info, rows: Vec<Option<serde_json::Value>>| {
        let rows: Vec<_> = rows.into_iter().flatten().collect();
        (!rows.is_empty()).then(|| json!({ "title": i18n::info_label(title), "rows": rows }))
    };
    let exif = metadata::read_details(path);
    let e = exif.as_ref();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let folder = path.parent().map(|p| p.display().to_string());
    let size = std::fs::metadata(path).ok().map(|m| i18n::file_size(m.len()));
    let coords = coords.map(|(lat, lon)| format!("{lat:.6}, {lon:.6}"));

    let mut sections = vec![section(
        Info::File,
        vec![
            row(Info::Name, name),
            row(Info::Folder, folder),
            row(Info::Size, size),
            row(Info::Dimensions, e.and_then(|e| Some(i18n::dimensions(e.width?, e.height?)))),
        ],
    )];
    let Some(e) = e else {
        sections.push(section(Info::Location, vec![row(Info::Coordinates, coords)]));
        let sections: Vec<_> = sections.into_iter().flatten().collect();
        return json!({ "sections": sections, "note": i18n::no_exif() });
    };
    let taken = e.taken_at.map(|d| {
        json!({
            "key": i18n::info_label(Info::Taken),
            "value": e.offset.clone().unwrap_or_default(),
            "date": d.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    });
    sections.push(section(
        Info::Shooting,
        vec![
            taken,
            row(Info::Camera, e.camera.clone()),
            row(Info::Lens, e.lens.clone()),
            row(Info::Exposure, e.exposure_time.map(|t| i18n::exposure(&metadata::format_exposure(t)))),
            row(Info::Aperture, e.f_number.map(i18n::aperture)),
            row(Info::Iso, e.iso.map(|i| i.to_string())),
            row(Info::FocalLength, e.focal_length.map(|f| i18n::focal_length(f, e.focal_35mm))),
            row(Info::ExposureBias, e.exposure_bias.map(i18n::exposure_bias)),
            row(Info::Flash, e.flash_fired.map(|f| i18n::flash(f).to_string())),
            row(Info::Software, e.software.clone()),
        ],
    ));
    sections.push(section(
        Info::Location,
        vec![row(Info::Coordinates, coords), row(Info::Altitude, e.altitude.map(i18n::altitude))],
    ));
    let sections: Vec<_> = sections.into_iter().flatten().collect();
    let all: Vec<_> = e.all.iter().map(|(tag, value)| json!({ "key": tag, "value": value })).collect();
    json!({ "sections": sections, "all": { "title": i18n::info_label(Info::AllTags), "rows": all } })
}

#[no_mangle]
pub unsafe extern "C" fn gpl_photos_len(photos: *const GplPhotos) -> usize {
    photos.as_ref().map_or(0, |p| p.rows.len())
}

/// Массив из `gpl_photos_len` элементов.
#[no_mangle]
pub unsafe extern "C" fn gpl_photos_data(photos: *const GplPhotos) -> *const GplPhoto {
    photos.as_ref().map_or(ptr::null(), |p| p.rows.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn gpl_photos_free(photos: *mut GplPhotos) {
    if !photos.is_null() {
        drop(Box::from_raw(photos));
    }
}

// ---------- Карта ----------

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GplCluster {
    pub lat: f64,
    pub lon: f64,
    /// Число фото в кластере.
    pub count: u32,
    /// Индекс первого фото кластера в списке — его миниатюра на маркере.
    pub cover: u32,
    /// Сумма id фото: вместе с `cover` и `count` — ключ кластера, по которому
    /// интерфейс понимает, что маркер не изменился.
    pub id_sum: i64,
}

/// Что видно на карте: кластеры для маркеров и индексы фото в видимой области.
pub struct GplMapView {
    clusters: Vec<GplCluster>,
    visible: Vec<u32>,
}

/// Снимок карты для списка `photos` (из `gpl_db_geotagged`): центр, зум Web Mercator
/// (тайл 256 пикселей), размер области в пикселях и размер ячейки кластеризации.
/// Маркеры считаются с запасом в одну ячейку за краем — не появляются рывком.
#[no_mangle]
pub unsafe extern "C" fn gpl_map_view(
    photos: *const GplPhotos,
    lat: f64,
    lon: f64,
    zoom: f64,
    width: f64,
    height: f64,
    cell: f64,
) -> *mut GplMapView {
    let mut out = GplMapView { clusters: Vec::new(), visible: Vec::new() };
    if let Some(photos) = photos.as_ref() {
        let _t = trace::span(format!("ffi.map_view (zoom {zoom:.1})"));
        let view = geo::View::new(lat, lon, zoom, width, height);
        let mut near = Vec::new();
        for (i, &(x, y)) in photos.points.iter().enumerate() {
            if view.contains_world(x, y, cell) {
                near.push(i);
                if view.contains_world(x, y, 0.0) {
                    out.visible.push(i as u32);
                }
            }
        }
        let pts: Vec<(f64, f64)> = near.iter().map(|&i| photos.points[i]).collect();
        out.clusters = geo::cluster_world(&pts, zoom, cell)
            .iter()
            .map(|c| GplCluster {
                lat: c.lat,
                lon: c.lon,
                count: c.members.len() as u32,
                cover: near[c.members[0]] as u32,
                id_sum: c.members.iter().map(|&m| photos.rows[near[m]].id).fold(0, i64::wrapping_add),
            })
            .collect();
    }
    Box::into_raw(Box::new(out))
}

/// Число фото списка в видимой области карты по дням, по возрастанию даты — дерево
/// фильтра карты по датам. Освобождать `gpl_date_counts_free`.
#[no_mangle]
pub unsafe extern "C" fn gpl_visible_date_counts(
    photos: *const GplPhotos,
    lat: f64,
    lon: f64,
    zoom: f64,
    width: f64,
    height: f64,
    len: *mut usize,
) -> *mut GplDateCount {
    let Some(photos) = photos.as_ref() else {
        *len = 0;
        return ptr::null_mut();
    };
    let view = geo::View::new(lat, lon, zoom, width, height);
    let days = photos.points.iter().zip(&photos.days).filter_map(|(&(x, y), &d)| d.filter(|_| view.contains_world(x, y, 0.0)));
    date_counts_out(DateCount::tally(days), len)
}

#[no_mangle]
pub unsafe extern "C" fn gpl_map_view_clusters(view: *const GplMapView, len: *mut usize) -> *const GplCluster {
    let Some(v) = view.as_ref() else {
        *len = 0;
        return ptr::null();
    };
    *len = v.clusters.len();
    v.clusters.as_ptr()
}

/// Индексы фото в видимой области, по возрастанию (то есть по дате съёмки).
#[no_mangle]
pub unsafe extern "C" fn gpl_map_view_visible(view: *const GplMapView, len: *mut usize) -> *const u32 {
    let Some(v) = view.as_ref() else {
        *len = 0;
        return ptr::null();
    };
    *len = v.visible.len();
    v.visible.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn gpl_map_view_free(view: *mut GplMapView) {
    if !view.is_null() {
        drop(Box::from_raw(view));
    }
}

/// Центр и целый зум, при которых все фото списка помещаются в область `width`×`height`
/// с отступом `pad`, но не крупнее `max_zoom`. `false` — список пуст.
#[no_mangle]
pub unsafe extern "C" fn gpl_photos_fit(
    photos: *const GplPhotos,
    width: f64,
    height: f64,
    pad: f64,
    max_zoom: f64,
    lat: *mut f64,
    lon: *mut f64,
    zoom: *mut f64,
) -> bool {
    let Some(photos) = photos.as_ref() else { return false };
    let pts: Vec<(f64, f64)> = photos.rows.iter().map(|r| (r.lat, r.lon)).collect();
    let Some((clat, clon)) = geo::center(&pts) else { return false };
    *lat = clat;
    *lon = clon;
    // Одна точка даёт бесконечный зум — ограничиваем сверху.
    *zoom = geo::fit_zoom(&pts, width, height, pad).min(max_zoom).max(1.0).floor();
    true
}

// ---------- HEIC ----------

/// Изображение от декодера: `rgba` — `width * height * 4` байт, выделенных `malloc`
/// (библиотека освободит их через `free`); `orig_*` — размеры оригинала после поворота.
#[repr(C)]
pub struct GplImage {
    pub rgba: *mut u8,
    pub width: u32,
    pub height: u32,
    pub orig_width: u32,
    pub orig_height: u32,
}

/// Декодер HEIC: уже повёрнутое изображение с короткой стороной не меньше `min_side`
/// (если оригинал не меньше). Вызывается из потоков сканера, одновременно для разных файлов.
pub type GplHeifDecoder = unsafe extern "C" fn(path: *const c_char, min_side: u32, out: *mut GplImage) -> bool;

static HEIF_DECODER: OnceLock<GplHeifDecoder> = OnceLock::new();

/// Подключить декодер HEIC (на macOS — ImageIO). Один раз при запуске, до сканирования.
#[no_mangle]
pub extern "C" fn gpl_set_heif_decoder(decoder: GplHeifDecoder) {
    if HEIF_DECODER.set(decoder).is_ok() {
        thumbs::set_heif_decoder(decode_heif);
    }
}

fn decode_heif(path: &Path, min_side: u32) -> anyhow::Result<(RgbImage, (u32, u32))> {
    use anyhow::Context;
    let decoder = HEIF_DECODER.get().context("декодер HEIC не подключён")?;
    let c_path = CString::new(path.as_os_str().as_encoded_bytes()).context("нулевой байт в пути")?;
    let mut out = GplImage { rgba: ptr::null_mut(), width: 0, height: 0, orig_width: 0, orig_height: 0 };
    // SAFETY: декодер получает действительные указатели; буфер по договорённости
    // выделен malloc, содержит width*height*4 байт и освобождается здесь.
    let rgb = unsafe {
        let ok = decoder(c_path.as_ptr(), min_side, &mut out);
        if out.rgba.is_null() {
            anyhow::bail!("системный декодер не прочитал файл");
        }
        let len = out.width as usize * out.height as usize * 4;
        let rgba = std::slice::from_raw_parts(out.rgba, len);
        let rgb: Vec<u8> = (0..len / 4).flat_map(|i| [rgba[4 * i], rgba[4 * i + 1], rgba[4 * i + 2]]).collect();
        libc::free(out.rgba.cast());
        if !ok {
            anyhow::bail!("системный декодер не прочитал файл");
        }
        rgb
    };
    let img = RgbImage::from_raw(out.width, out.height, rgb).context("неверный буфер изображения")?;
    Ok((img, (out.orig_width, out.orig_height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn take(s: *mut c_char) -> String {
        let text = CStr::from_ptr(s).to_str().unwrap().to_owned();
        gpl_string_free(s);
        text
    }

    #[test]
    fn strings() {
        unsafe {
            assert!(!take(gpl_text(c"tab_map".as_ptr())).is_empty());
            assert!(gpl_text(c"нет такого".as_ptr()).is_null());
            assert!(gpl_text(ptr::null()).is_null());
            assert_eq!(take(gpl_text_date_label(2023, 0, 0)), "2023");
            assert_eq!(take(gpl_text_date_label(2023, 3, 0)), i18n::month(3));
            assert_eq!(take(gpl_text_date_label(2023, 3, 8)), i18n::day(3, 8));
            assert_eq!(take(gpl_text_date_title(2023, 3, 8)), i18n::full_day(2023, 3, 8));
            // Месяц вне 1…12 не роняет: подпись — год.
            assert_eq!(take(gpl_text_date_label(2023, 13, 1)), "2023");
        }
        assert_eq!(c_string("a\0b").as_bytes(), b"a");
    }

    #[test]
    fn info_without_exif() {
        // Файла нет: имя и папка из пути, координаты из БД, пояснение вместо EXIF.
        let info = photo_info(Path::new("/lib/2023/a.jpg"), Some((34.68, 32.96)));
        let sections = info["sections"].as_array().unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0]["rows"][0]["value"], "a.jpg");
        assert_eq!(sections[0]["rows"][1]["value"], "/lib/2023");
        assert_eq!(sections[1]["rows"][0]["value"], "34.680000, 32.960000");
        assert_eq!(info["note"], i18n::no_exif());
        // Без координат раздел «Место» не показывается.
        assert_eq!(photo_info(Path::new("/lib/a.jpg"), None)["sections"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn photos_and_map_view() {
        let photo = |id: i64, lat: f64, lon: f64| Photo {
            id,
            path: format!("/lib/{id}.jpg"),
            taken_at: "2023-07-14 12:30:45".into(),
            lat: Some(lat),
            lon: Some(lon),
            thumb_path: (id != 3).then(|| format!("/thumbs/{id}.jpg")),
        };
        // Два фото рядом в Лимасоле, одно в Афинах.
        let list = GplPhotos::new(vec![photo(1, 34.68, 32.96), photo(2, 34.6801, 32.9601), photo(3, 37.98, 23.73)]);
        let photos: *const GplPhotos = &list;
        unsafe {
            assert_eq!(gpl_photos_len(photos), 3);
            let rows = std::slice::from_raw_parts(gpl_photos_data(photos), 3);
            assert_eq!(CStr::from_ptr(rows[1].path).to_str().unwrap(), "/lib/2.jpg");
            assert!(rows[2].thumb_path.is_null() && rows[2].has_coords);

            // Вблизи Лимасола: один кластер из двух фото, Афины за пределами области.
            let view = gpl_map_view(photos, 34.68, 32.96, 10.0, 800.0, 600.0, 72.0);
            let mut n = 0;
            let clusters = std::slice::from_raw_parts(gpl_map_view_clusters(view, &mut n), n);
            assert_eq!((n, clusters[0].count, clusters[0].cover, clusters[0].id_sum), (1, 2, 0, 3));
            let visible = std::slice::from_raw_parts(gpl_map_view_visible(view, &mut n), n);
            assert_eq!(visible, [0, 1]);
            gpl_map_view_free(view);

            let (mut lat, mut lon, mut zoom) = (0.0, 0.0, 0.0);
            assert!(gpl_photos_fit(photos, 800.0, 600.0, 40.0, 17.0, &mut lat, &mut lon, &mut zoom));
            assert!((34.0..38.0).contains(&lat) && (23.0..33.0).contains(&lon));
            assert!((1.0..10.0).contains(&zoom) && zoom.fract() == 0.0);
            let empty = GplPhotos::new(Vec::new());
            assert!(!gpl_photos_fit(&empty, 800.0, 600.0, 40.0, 17.0, &mut lat, &mut lon, &mut zoom));
            assert_eq!(gpl_photos_len(ptr::null()), 0);
        }
    }
}

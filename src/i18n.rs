//! Локализация интерфейса: английский (по умолчанию), русский, греческий.
//!
//! Язык выбирается один раз при запуске по переменным окружения в порядке glibc:
//! `LANGUAGE` (список через `:`), затем `LC_ALL`, `LC_MESSAGES`, `LANG`.
//! Все строки, которые видит пользователь, собраны здесь; диагностика в stderr
//! (`eprintln!`, трассировка) не переводится.
use chrono::NaiveDateTime;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
    El,
}

static LANG: OnceLock<Lang> = OnceLock::new();

/// Текущий язык интерфейса.
pub fn lang() -> Lang {
    *LANG.get_or_init(|| detect(|k| std::env::var(k).ok()))
}

/// Язык по кодам локали `ru_RU.UTF-8`, `el_GR`, `en`…; `None` — не поддерживается.
fn parse(locale: &str) -> Option<Lang> {
    let code = locale.split(['_', '.', '@', '-']).next()?.to_ascii_lowercase();
    match code.as_str() {
        "en" => Some(Lang::En),
        "ru" => Some(Lang::Ru),
        "el" => Some(Lang::El),
        _ => None,
    }
}

fn detect(var: impl Fn(&str) -> Option<String>) -> Lang {
    let nonempty = |k: &str| var(k).filter(|v| !v.is_empty());
    let locale = nonempty("LC_ALL")
        .or_else(|| nonempty("LC_MESSAGES"))
        .or_else(|| nonempty("LANG"))
        .unwrap_or_default();
    // Как в glibc: при локали C/POSIX переменная LANGUAGE игнорируется.
    if locale.is_empty() || locale == "C" || locale.starts_with("C.") || locale == "POSIX" {
        return Lang::En;
    }
    nonempty("LANGUAGE")
        .and_then(|list| list.split(':').find_map(parse))
        .or_else(|| parse(&locale))
        .unwrap_or(Lang::En)
}

/// Выбор строки для текущего языка.
fn tr(en: &'static str, ru: &'static str, el: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Ru => ru,
        Lang::El => el,
    }
}

// ---------- Главное окно ----------

pub fn tab_dates() -> &'static str {
    tr("By date", "По датам", "Ανά ημερομηνία")
}

pub fn tab_map() -> &'static str {
    tr("Map", "Карта", "Χάρτης")
}

pub fn settings() -> &'static str {
    tr("Settings", "Настройки", "Ρυθμίσεις")
}

pub fn main_menu() -> &'static str {
    tr("Main menu", "Главное меню", "Κύριο μενού")
}

pub fn about() -> &'static str {
    tr("About", "О программе", "Σχετικά")
}

/// Краткое описание программы (окно «О программе»).
pub fn app_summary() -> &'static str {
    tr(
        "Browse photos by date and on a world map",
        "Просмотр фотографий по датам и на карте мира",
        "Περιήγηση φωτογραφιών ανά ημερομηνία και στον παγκόσμιο χάρτη",
    )
}

pub fn rescan() -> &'static str {
    tr("Rescan library", "Пересканировать библиотеку", "Επανασάρωση βιβλιοθήκης")
}

/// «5 photos», «5 фото», «5 φωτογραφίες».
pub fn photos(n: usize) -> String {
    photos_in(lang(), n)
}

fn photos_in(lang: Lang, n: usize) -> String {
    match lang {
        Lang::En if n == 1 => "1 photo".into(),
        Lang::En => format!("{n} photos"),
        Lang::Ru => format!("{n} фото"),
        Lang::El if n == 1 => "1 φωτογραφία".into(),
        Lang::El => format!("{n} φωτογραφίες"),
    }
}

pub fn stopping_scan() -> &'static str {
    tr("Stopping current scan…", "Остановка текущего сканирования…", "Διακοπή τρέχουσας σάρωσης…")
}

pub fn searching() -> &'static str {
    tr("Searching for photos…", "Поиск фотографий…", "Αναζήτηση φωτογραφιών…")
}

pub fn processing(done: usize, total: usize) -> String {
    let word = tr("Processing", "Обработка", "Επεξεργασία");
    format!("{word} {done} / {total}")
}

pub fn scan_summary(found: usize, added: usize, updated: usize, removed: usize) -> String {
    let photos = photos(found);
    match lang() {
        Lang::En => format!("{photos} ({added} new, {updated} changed, {removed} removed)"),
        Lang::Ru => format!("{photos} (новых {added}, изменено {updated}, удалено {removed})"),
        Lang::El => format!("{photos} (νέες {added}, αλλαγμένες {updated}, διαγραμμένες {removed})"),
    }
}

pub fn scan_error(e: &str) -> String {
    format!("{}: {e}", tr("Scan error", "Ошибка сканирования", "Σφάλμα σάρωσης"))
}

pub fn sort_label(order: crate::config::SortOrder) -> &'static str {
    use crate::config::SortOrder;
    match order {
        SortOrder::Newest => tr("Newest first", "Сначала новые", "Πρώτα οι νεότερες"),
        SortOrder::Oldest => tr("Oldest first", "Сначала старые", "Πρώτα οι παλαιότερες"),
    }
}

pub fn thumb_size_tooltip() -> &'static str {
    tr("Thumbnail size", "Размер миниатюр", "Μέγεθος μικρογραφιών")
}

pub fn thumbs_smaller() -> &'static str {
    tr("Smaller thumbnails", "Уменьшить миниатюры", "Μικρότερες μικρογραφίες")
}

pub fn thumbs_larger() -> &'static str {
    tr("Larger thumbnails", "Увеличить миниатюры", "Μεγαλύτερες μικρογραφίες")
}

pub fn sort_tooltip() -> &'static str {
    tr("Sort order", "Сортировка", "Ταξινόμηση")
}

// ---------- Контекстное меню фото ----------

pub fn copy() -> &'static str {
    tr("Copy", "Копировать", "Αντιγραφή")
}

pub fn view() -> &'static str {
    tr("View", "Просмотр", "Προβολή")
}

pub fn edit() -> &'static str {
    tr("Edit", "Редактирование", "Επεξεργασία")
}

pub fn delete() -> &'static str {
    tr("Delete", "Удалить", "Διαγραφή")
}

pub fn info() -> &'static str {
    tr("Information", "Информация", "Πληροφορίες")
}

pub fn show_on_map() -> &'static str {
    tr("Show on map", "Найти на карте", "Εύρεση στον χάρτη")
}

pub fn no_coordinates() -> &'static str {
    tr("No coordinates", "Координаты отсутствуют", "Δεν υπάρχουν συντεταγμένες")
}

pub fn no_coordinates_detail(name: &str) -> String {
    match lang() {
        Lang::En => format!("The EXIF data of {name} has no GPS coordinates."),
        Lang::Ru => format!("В EXIF фотографии {name} нет GPS-координат."),
        Lang::El => format!("Τα δεδομένα EXIF της φωτογραφίας {name} δεν περιέχουν συντεταγμένες GPS."),
    }
}

pub fn ok() -> &'static str {
    tr("OK", "OK", "OK")
}

pub fn trashed(name: &str) -> String {
    format!("{}: {name}", tr("Moved to trash", "Перемещено в корзину", "Μετακινήθηκε στα απορρίμματα"))
}

pub fn trash_failed(name: &str, e: &str) -> String {
    match lang() {
        Lang::En => format!("Could not move {name} to trash: {e}"),
        Lang::Ru => format!("Не удалось переместить {name} в корзину: {e}"),
        Lang::El => format!("Αδυναμία μετακίνησης του {name} στα απορρίμματα: {e}"),
    }
}

// ---------- Сканер и CLI ----------

pub fn scan_cancelled() -> &'static str {
    tr("scan cancelled", "сканирование отменено", "η σάρωση ακυρώθηκε")
}

pub fn folder_unavailable(path: &std::path::Path) -> String {
    let p = path.display();
    match lang() {
        Lang::En => format!("folder {p} is not accessible"),
        Lang::Ru => format!("папка {p} недоступна"),
        Lang::El => format!("ο φάκελος {p} δεν είναι προσβάσιμος"),
    }
}

pub fn cli_usage() -> &'static str {
    tr(
        "Specify a folder: geo-photo-library --scan <folder>",
        "Укажите папку: geo-photo-library --scan <папка>",
        "Καθορίστε φάκελο: geo-photo-library --scan <φάκελος>",
    )
}

pub fn cli_summary(found: usize, added: usize, updated: usize, removed: usize) -> String {
    match lang() {
        Lang::En => format!("Found: {found}, added: {added}, updated: {updated}, removed: {removed}"),
        Lang::Ru => format!("Найдено: {found}, добавлено: {added}, обновлено: {updated}, удалено: {removed}"),
        Lang::El => format!(
            "Βρέθηκαν: {found}, προστέθηκαν: {added}, ενημερώθηκαν: {updated}, διαγράφηκαν: {removed}"
        ),
    }
}

pub fn error() -> &'static str {
    tr("Error", "Ошибка", "Σφάλμα")
}

// ---------- Настройки ----------

pub fn browse() -> &'static str {
    tr("Browse…", "Выбрать…", "Επιλογή…")
}

pub fn cancel() -> &'static str {
    tr("Cancel", "Отмена", "Άκυρο")
}

pub fn save() -> &'static str {
    tr("Save", "Сохранить", "Αποθήκευση")
}

pub fn photo_folder() -> &'static str {
    tr("Photo folder", "Папка с фотографиями", "Φάκελος φωτογραφιών")
}

pub fn folder_hint() -> &'static str {
    tr(
        "The folder is scanned recursively on every launch: JPEG, HEIC, PNG, WebP, TIFF, DNG.",
        "Папка сканируется рекурсивно при каждом запуске: JPEG, HEIC, PNG, WebP, TIFF, DNG.",
        "Ο φάκελος σαρώνεται αναδρομικά σε κάθε εκκίνηση: JPEG, HEIC, PNG, WebP, TIFF, DNG.",
    )
}

pub fn photo_viewer() -> &'static str {
    tr("Photo viewer", "Просмотрщик фотографий", "Πρόγραμμα προβολής φωτογραφιών")
}

pub fn photo_editor() -> &'static str {
    tr("Photo editor", "Редактор фотографий", "Επεξεργαστής φωτογραφιών")
}

pub fn automatic() -> &'static str {
    tr("Automatic", "Автоматически", "Αυτόματα")
}

pub fn apps_hint() -> &'static str {
    tr(
        "Used by double-click (Ctrl + double-click to edit) and the photo context menu.",
        "Используются при двойном щелчке (Ctrl + двойной щелчок — редактирование) и в контекстном меню фото.",
        "Χρησιμοποιούνται με διπλό κλικ (Ctrl + διπλό κλικ για επεξεργασία) και στο μενού της φωτογραφίας.",
    )
}

pub fn folder_missing() -> &'static str {
    tr("Folder does not exist", "Папка не существует", "Ο φάκελος δεν υπάρχει")
}

// ---------- Вкладки ----------

pub fn no_photos() -> &'static str {
    tr("No photos", "Нет фотографий", "Δεν υπάρχουν φωτογραφίες")
}

pub fn no_geotagged() -> &'static str {
    tr("No photos with coordinates", "Нет фото с координатами", "Δεν υπάρχουν φωτογραφίες με συντεταγμένες")
}

pub fn in_map_area(visible: usize, total: usize) -> String {
    match lang() {
        Lang::En => format!("On map: {visible} of {total}"),
        Lang::Ru => format!("На карте: {visible} из {total}"),
        Lang::El => format!("Στον χάρτη: {visible} από {total}"),
    }
}

/// Строка поиска местности на карте.
pub fn search_place() -> &'static str {
    tr("Search for a place…", "Поиск местности…", "Αναζήτηση τοποθεσίας…")
}

pub fn place_not_found() -> &'static str {
    tr("Nothing found", "Ничего не найдено", "Δεν βρέθηκε τίποτα")
}

pub fn place_search_failed() -> &'static str {
    tr(
        "Search failed. Check your internet connection.",
        "Не удалось выполнить поиск. Проверьте подключение к интернету.",
        "Η αναζήτηση απέτυχε. Ελέγξτε τη σύνδεση στο διαδίκτυο.",
    )
}

pub fn my_location() -> &'static str {
    tr("My location", "Моё местоположение", "Η τοποθεσία μου")
}

pub fn location_failed() -> &'static str {
    tr("Location unavailable", "Местоположение не определено", "Η τοποθεσία δεν είναι διαθέσιμη")
}

pub fn location_unavailable() -> &'static str {
    tr(
        "Check that location services are turned on in the system settings and try again.",
        "Проверьте, что службы геолокации включены в настройках системы, и повторите попытку.",
        "Ελέγξτε ότι οι υπηρεσίες τοποθεσίας είναι ενεργές στις ρυθμίσεις συστήματος και δοκιμάστε ξανά.",
    )
}

pub fn location_denied() -> &'static str {
    tr(
        "Access to your location was denied. You can allow it in the system privacy settings.",
        "Доступ к местоположению запрещён. Разрешить его можно в настройках конфиденциальности системы.",
        "Η πρόσβαση στην τοποθεσία σας απορρίφθηκε. Μπορείτε να την επιτρέψετε στις ρυθμίσεις απορρήτου του συστήματος.",
    )
}

/// Код языка для внешних сервисов (названия мест в ответе геокодера).
pub fn lang_code() -> &'static str {
    match lang() {
        Lang::En => "en",
        Lang::Ru => "ru",
        Lang::El => "el",
    }
}

// ---------- Панель «Информация» ----------

pub fn close() -> &'static str {
    tr("Close", "Закрыть", "Κλείσιμο")
}

pub fn select_photo() -> &'static str {
    tr("Select a photo", "Выберите фотографию", "Επιλέξτε μια φωτογραφία")
}

pub fn no_exif() -> &'static str {
    tr("No EXIF data", "Нет данных EXIF", "Δεν υπάρχουν δεδομένα EXIF")
}

/// Заголовки разделов и подписи полей панели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Info {
    File,
    Shooting,
    Location,
    AllTags,
    Name,
    Folder,
    Size,
    Dimensions,
    Taken,
    Camera,
    Lens,
    Exposure,
    Aperture,
    Iso,
    FocalLength,
    ExposureBias,
    Flash,
    Software,
    Coordinates,
    Altitude,
}

pub fn info_label(key: Info) -> &'static str {
    match key {
        Info::File => tr("File", "Файл", "Αρχείο"),
        Info::Shooting => tr("Shooting", "Съёмка", "Λήψη"),
        Info::Location => tr("Location", "Место", "Τοποθεσία"),
        Info::AllTags => tr("All EXIF tags", "Все теги EXIF", "Όλες οι ετικέτες EXIF"),
        Info::Name => tr("Name", "Имя", "Όνομα"),
        Info::Folder => tr("Folder", "Папка", "Φάκελος"),
        Info::Size => tr("Size", "Размер", "Μέγεθος"),
        Info::Dimensions => tr("Dimensions", "Разрешение", "Διαστάσεις"),
        Info::Taken => tr("Date taken", "Дата съёмки", "Ημερομηνία λήψης"),
        Info::Camera => tr("Camera", "Камера", "Κάμερα"),
        Info::Lens => tr("Lens", "Объектив", "Φακός"),
        Info::Exposure => tr("Shutter speed", "Выдержка", "Ταχύτητα κλείστρου"),
        Info::Aperture => tr("Aperture", "Диафрагма", "Διάφραγμα"),
        Info::Iso => "ISO",
        Info::FocalLength => tr("Focal length", "Фокусное расстояние", "Εστιακή απόσταση"),
        Info::ExposureBias => tr("Exposure compensation", "Экспокоррекция", "Αντιστάθμιση έκθεσης"),
        Info::Flash => tr("Flash", "Вспышка", "Φλας"),
        Info::Software => tr("Software", "Программа", "Λογισμικό"),
        Info::Coordinates => tr("Coordinates", "Координаты", "Συντεταγμένες"),
        Info::Altitude => tr("Altitude", "Высота", "Υψόμετρο"),
    }
}

pub fn flash(fired: bool) -> &'static str {
    if fired {
        tr("Fired", "Сработала", "Ενεργοποιήθηκε")
    } else {
        tr("Did not fire", "Не сработала", "Δεν ενεργοποιήθηκε")
    }
}

/// Число с одним знаком после запятой без лишнего нуля; в ru/el — десятичная запятая.
fn decimal(lang: Lang, v: f64) -> String {
    let s = format!("{v:.1}");
    let s = s.strip_suffix(".0").unwrap_or(&s).to_string();
    match lang {
        Lang::En => s,
        Lang::Ru | Lang::El => s.replace('.', ","),
    }
}

/// Выдержка: «1/250 s», «1/250 с», «1/250 δ».
pub fn exposure(t: &str) -> String {
    format!("{t} {}", tr("s", "с", "δ"))
}

/// Фокусное расстояние и эквивалент для 35-мм кадра.
pub fn focal_length(mm: f64, equiv: Option<u32>) -> String {
    focal_length_in(lang(), mm, equiv)
}

fn focal_length_in(lang: Lang, mm: f64, equiv: Option<u32>) -> String {
    let (unit, eq) = match lang {
        Lang::En => ("mm", "35 mm equivalent"),
        Lang::Ru => ("мм", "в пересчёте на 35 мм"),
        Lang::El => ("mm", "ισοδύναμο 35 mm"),
    };
    let f = decimal(lang, mm);
    match equiv {
        Some(e) => format!("{f} {unit} ({eq}: {e} {unit})"),
        None => format!("{f} {unit}"),
    }
}

pub fn aperture(f: f64) -> String {
    format!("f/{}", decimal(lang(), f))
}

/// Экспокоррекция со знаком: «+0,7 EV», «0 EV».
pub fn exposure_bias(ev: f64) -> String {
    let v = decimal(lang(), ev);
    if ev > 0.05 {
        format!("+{v} EV")
    } else {
        format!("{v} EV")
    }
}

/// Высота над уровнем моря в метрах.
pub fn altitude(m: f64) -> String {
    format!("{} {}", m.round() as i64, tr("m", "м", "μ"))
}

/// Размер файла: «850 KB», «3.2 MB»; в ru — «3,2 МБ».
pub fn file_size(bytes: u64) -> String {
    file_size_in(lang(), bytes)
}

fn file_size_in(lang: Lang, bytes: u64) -> String {
    let (kb, mb) = match lang {
        Lang::En | Lang::El => ("KB", "MB"),
        Lang::Ru => ("КБ", "МБ"),
    };
    let b = bytes as f64;
    if b < 1024.0 * 1024.0 {
        format!("{} {kb}", (b / 1024.0).ceil() as u64)
    } else {
        format!("{} {mb}", decimal(lang, b / 1024.0 / 1024.0))
    }
}

/// Разрешение снимка: «4032 × 3024 (12.2 MP)».
pub fn dimensions(w: u32, h: u32) -> String {
    let mp = decimal(lang(), w as f64 * h as f64 / 1e6);
    format!("{w} × {h} ({mp} {})", tr("MP", "Мп", "MP"))
}

// ---------- Даты ----------

const MONTHS_EN: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];
const MONTHS_RU: [&str; 12] = [
    "Январь", "Февраль", "Март", "Апрель", "Май", "Июнь",
    "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];
const MONTHS_RU_GEN: [&str; 12] = [
    "января", "февраля", "марта", "апреля", "мая", "июня",
    "июля", "августа", "сентября", "октября", "ноября", "декабря",
];
const MONTHS_EL: [&str; 12] = [
    "Ιανουάριος", "Φεβρουάριος", "Μάρτιος", "Απρίλιος", "Μάιος", "Ιούνιος",
    "Ιούλιος", "Αύγουστος", "Σεπτέμβριος", "Οκτώβριος", "Νοέμβριος", "Δεκέμβριος",
];
const MONTHS_EL_GEN: [&str; 12] = [
    "Ιανουαρίου", "Φεβρουαρίου", "Μαρτίου", "Απριλίου", "Μαΐου", "Ιουνίου",
    "Ιουλίου", "Αυγούστου", "Σεπτεμβρίου", "Οκτωβρίου", "Νοεμβρίου", "Δεκεμβρίου",
];

/// Название месяца (1…12) в именительном падеже: «March», «Март», «Μάρτιος».
pub fn month(m: u32) -> &'static str {
    month_in(lang(), m)
}

fn month_in(lang: Lang, m: u32) -> &'static str {
    let i = m as usize - 1;
    match lang {
        Lang::En => MONTHS_EN[i],
        Lang::Ru => MONTHS_RU[i],
        Lang::El => MONTHS_EL[i],
    }
}

/// День месяца без года: «March 8», «8 марта», «8 Μαρτίου».
pub fn day(m: u32, d: u32) -> String {
    day_in(lang(), m, d)
}

fn day_in(lang: Lang, m: u32, d: u32) -> String {
    let i = m as usize - 1;
    match lang {
        Lang::En => format!("{} {d}", MONTHS_EN[i]),
        Lang::Ru => format!("{d} {}", MONTHS_RU_GEN[i]),
        Lang::El => format!("{d} {}", MONTHS_EL_GEN[i]),
    }
}

/// Полная дата: «March 8, 2023», «8 марта 2023», «8 Μαρτίου 2023».
pub fn full_day(y: i32, m: u32, d: u32) -> String {
    full_day_in(lang(), y, m, d)
}

fn full_day_in(lang: Lang, y: i32, m: u32, d: u32) -> String {
    match lang {
        Lang::En => format!("{}, {y}", day_in(lang, m, d)),
        Lang::Ru | Lang::El => format!("{} {y}", day_in(lang, m, d)),
    }
}

/// Год как заголовок: «2023», «2023 год», «2023».
pub fn year(y: i32) -> String {
    match lang() {
        Lang::Ru => format!("{y} год"),
        Lang::En | Lang::El => y.to_string(),
    }
}

/// Дата и время снимка в подписи: «2023-07-14 12:30», «14.07.2023 12:30», «14/07/2023 12:30».
pub fn datetime(dt: &NaiveDateTime) -> String {
    datetime_in(lang(), dt)
}

fn datetime_in(lang: Lang, dt: &NaiveDateTime) -> String {
    let fmt = match lang {
        Lang::En => "%Y-%m-%d %H:%M",
        Lang::Ru => "%d.%m.%Y %H:%M",
        Lang::El => "%d/%m/%Y %H:%M",
    };
    dt.format(fmt).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn info_units() {
        assert_eq!(decimal(Lang::En, 2.0), "2");
        assert_eq!(decimal(Lang::Ru, 1.78), "1,8");
        assert_eq!(file_size_in(Lang::En, 3_355_443), "3.2 MB");
        assert_eq!(file_size_in(Lang::Ru, 3_355_443), "3,2 МБ");
        assert_eq!(file_size_in(Lang::En, 1000), "1 KB");
        assert_eq!(focal_length_in(Lang::Ru, 6.86, Some(26)), "6,9 мм (в пересчёте на 35 мм: 26 мм)");
        assert_eq!(focal_length_in(Lang::En, 50.0, None), "50 mm");
    }

    #[test]
    fn detection() {
        assert_eq!(detect(env(&[])), Lang::En);
        assert_eq!(detect(env(&[("LANG", "ru_RU.UTF-8")])), Lang::Ru);
        assert_eq!(detect(env(&[("LANG", "el_GR.UTF-8")])), Lang::El);
        assert_eq!(detect(env(&[("LANG", "de_DE.UTF-8")])), Lang::En);
        assert_eq!(detect(env(&[("LANG", "C.UTF-8")])), Lang::En);
        // LC_ALL важнее LC_MESSAGES, та — важнее LANG.
        assert_eq!(detect(env(&[("LANG", "ru_RU"), ("LC_MESSAGES", "el_GR")])), Lang::El);
        assert_eq!(detect(env(&[("LC_MESSAGES", "el_GR"), ("LC_ALL", "en_US")])), Lang::En);
        assert_eq!(detect(env(&[("LANG", "ru_RU"), ("LC_ALL", "")])), Lang::Ru);
        // LANGUAGE: первый поддерживаемый из списка, но не при локали C.
        assert_eq!(detect(env(&[("LANG", "en_US"), ("LANGUAGE", "de:el:ru")])), Lang::El);
        assert_eq!(detect(env(&[("LANG", "ru_RU"), ("LANGUAGE", "de")])), Lang::Ru);
        assert_eq!(detect(env(&[("LANG", "C"), ("LANGUAGE", "ru")])), Lang::En);
        assert_eq!(detect(env(&[("LANG", "sr_RS@latin")])), Lang::En);
    }

    #[test]
    fn plurals() {
        assert_eq!(photos_in(Lang::En, 1), "1 photo");
        assert_eq!(photos_in(Lang::En, 5), "5 photos");
        assert_eq!(photos_in(Lang::Ru, 1), "1 фото");
        assert_eq!(photos_in(Lang::El, 1), "1 φωτογραφία");
        assert_eq!(photos_in(Lang::El, 0), "0 φωτογραφίες");
    }

    #[test]
    fn dates() {
        assert_eq!(month_in(Lang::El, 5), "Μάιος");
        assert_eq!(day_in(Lang::En, 3, 8), "March 8");
        assert_eq!(day_in(Lang::Ru, 3, 8), "8 марта");
        assert_eq!(day_in(Lang::El, 5, 8), "8 Μαΐου");
        assert_eq!(full_day_in(Lang::En, 2023, 12, 31), "December 31, 2023");
        assert_eq!(full_day_in(Lang::Ru, 2023, 12, 31), "31 декабря 2023");
        let dt = NaiveDateTime::parse_from_str("2023-07-14 12:30:45", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(datetime_in(Lang::En, &dt), "2023-07-14 12:30");
        assert_eq!(datetime_in(Lang::Ru, &dt), "14.07.2023 12:30");
        assert_eq!(datetime_in(Lang::El, &dt), "14/07/2023 12:30");
    }
}

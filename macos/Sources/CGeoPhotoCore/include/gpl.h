// C-интерфейс ядра GeoPhotoLibrary (Rust, src/ffi.rs).
// Объявления должны совпадать с src/ffi.rs — при изменении править оба файла.
//
// Строки на входе — UTF-8 с завершающим нулём. Строки, которые вернула библиотека
// (char *), освобождать gpl_string_free; списки и снимки карты — своей *_free.
// Общего состояния нет: функции можно вызывать из любого потока.
#ifndef GPL_H
#define GPL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

void gpl_string_free(char *s);

// Язык интерфейса ("ru", "el-GR"…). Один раз при запуске, до остальных функций.
void gpl_init(const char *lang);

// ---------- Строки интерфейса ----------

// Строка по ключу — имени функции в i18n.rs; NULL для неизвестного ключа.
char *gpl_text(const char *key);
char *gpl_text_photos(size_t n);
char *gpl_text_processing(size_t done, size_t total);
char *gpl_text_scan_error(const char *e);
char *gpl_text_in_map_area(size_t visible, size_t total);
char *gpl_text_no_coordinates_detail(const char *name);
char *gpl_text_trashed(const char *name);
char *gpl_text_trash_failed(const char *name, const char *e);
// Вопрос перед удалением: subject — имя файла или «5 фото».
char *gpl_text_confirm_trash(const char *subject);
char *gpl_text_save_count(size_t n);
char *gpl_text_confirm_update_coordinates_detail(const char *name);
char *gpl_text_confirm_save_changes(size_t n);
// tags — имена изменённых тегов через перевод строки.
char *gpl_text_confirm_save_changes_detail(bool coords, const char *tags);
char *gpl_text_changes_saved(const char *subject);
char *gpl_text_changes_failed(const char *name, const char *e);
// Подпись узла дерева дат и заголовок периода: month == 0 — год, day == 0 — месяц.
char *gpl_text_date_label(int32_t year, uint32_t month, uint32_t day);
char *gpl_text_date_title(int32_t year, uint32_t month, uint32_t day);

// ---------- Настройки ----------

// Папка библиотеки; NULL, если не задана.
char *gpl_config_library_path(void);
bool gpl_config_set_library_path(const char *path, char **err);
// Положение карты, сохранённое при выходе; false — его нет.
bool gpl_config_map_view(double *lat, double *lon, double *zoom);
bool gpl_config_set_map_view(double lat, double lon, double zoom, char **err);
// Сортировка фото в сетках: true — новые сверху.
bool gpl_config_newest_first(void);
bool gpl_config_set_newest_first(bool newest, char **err);
// Масштаб миниатюр в процентах (приводится к диапазону и шагу).
uint32_t gpl_config_thumb_scale(void);
bool gpl_config_set_thumb_scale(uint32_t percent, char **err);
// Программа из настроек (editor — редактор, иначе просмотрщик): идентификатор пакета
// приложения; NULL или пустая строка — выбирать автоматически.
char *gpl_config_app(bool editor);
bool gpl_config_set_app(bool editor, const char *id, char **err);
// Проверять обновления при запуске.
bool gpl_config_check_updates(void);
bool gpl_config_set_check_updates(bool check, char **err);

// ---------- Обновления ----------

// Адрес API последнего выпуска на GitHub (site == false) или сайта программы.
char *gpl_update_url(bool site);
// Версия из ответа GitHub (json, len байт), если она новее текущей; иначе NULL.
char *gpl_update_newer(const uint8_t *json, size_t len);
// Отладка (GEO_PHOTO_LIBRARY_UPDATE): true — версия подменена и запрос не нужен,
// в *version — она, если новее текущей (иначе NULL).
bool gpl_update_debug(char **version);
char *gpl_text_update_available(const char *version);

// ---------- Сканирование ----------

typedef struct {
    size_t found;
    size_t added;
    size_t updated;
    size_t removed;
} GplScanStats;

char *gpl_text_scan_summary(GplScanStats stats);

// Флаг отмены: gpl_cancel_set можно вызвать из другого потока.
// Освобождать только после возврата из gpl_scan.
typedef struct GplCancel GplCancel;
GplCancel *gpl_cancel_new(void);
void gpl_cancel_set(const GplCancel *cancel);
void gpl_cancel_free(GplCancel *cancel);

typedef void (*GplProgress)(void *ctx, size_t done, size_t total);

enum { GPL_SCAN_OK = 0, GPL_SCAN_CANCELLED = 1, GPL_SCAN_ERROR = 2 };

// Сканирует папку в БД приложения, блокируя поток; progress вызывается из него же.
// Возвращает GPL_SCAN_*; при ошибке её текст — в err.
int gpl_scan(const char *root, GplProgress progress, void *ctx, const GplCancel *cancel,
             GplScanStats *stats, char **err);

// ---------- Запросы к БД ----------

int64_t gpl_db_count(void);

typedef struct {
    int32_t year;
    uint32_t month;
    uint32_t day;
    uint32_t count;
} GplDateCount;

// Фильтр вкладки дат: 0 — все фото и видео, 1 — фото с координатами, 2 — фото без координат, 3 — видео.
// Число фото по дням, по возрастанию даты.
GplDateCount *gpl_db_date_counts(uint32_t filter, size_t *len);
void gpl_date_counts_free(GplDateCount *counts, size_t len);

typedef struct {
    int64_t id;
    const char *path;
    const char *taken_at;   // YYYY-MM-DD HH:MM:SS
    const char *thumb_path; // NULL, если миниатюры нет
    double lat;
    double lon;
    bool has_coords;
    bool is_video;
    double duration;        // длительность видео, с; < 0 — неизвестна (и у фото)
} GplPhoto;

typedef struct GplPhotos GplPhotos;

// Фильтр вкладки дат при запуске из GEO_PHOTO_LIBRARY_GEO_FILTER (отладка); 0 — все фото.
uint32_t gpl_debug_geo_filter(void);
// Фото за год, месяц (day == 0) или день; month == 0 — весь год. По дате съёмки;
// filter — как в gpl_db_date_counts.
GplPhotos *gpl_db_photos_by_date(int32_t year, uint32_t month, uint32_t day, uint32_t filter);
// Фото с координатами за год, месяц (day == 0) или день (month == 0 — весь год);
// year == 0 — за всё время. По дате съёмки.
GplPhotos *gpl_db_geotagged(int32_t year, uint32_t month, uint32_t day);
// Координата из поля ввода (точка или запятая); false — не число.
bool gpl_parse_coord(const char *text, double *out);
// Пара «широта, долгота» одной строкой (вставка из карт); false — не пара координат.
bool gpl_parse_coord_pair(const char *text, double *lat, double *lon);
// Координата для поля ввода (6 знаков после точки).
char *gpl_format_coord(double v);
// Допустимы ли координаты для записи в фото.
bool gpl_valid_coords(double lat, double lon);
// Поля вкладки «EXIF» для файлов paths (JSON-массив путей): JSON-массив
// {key, name, value, editable}, value — null, если у фото значения разные. Читает файлы.
char *gpl_exif_fields(const char *paths);
// Проверить текст поля тега: NULL — подходит, иначе пояснение (gpl_string_free).
char *gpl_exif_check(const char *key, const char *text);
// Записать в EXIF файла координаты (если has_coords) и теги tags (JSON-массив пар
// [ключ, текст], пустой текст — удалить тег), затем обновить запись в БД. NULL — успех,
// иначе текст ошибки (gpl_string_free). Пишет файл — не из главного потока.
char *gpl_save_edit(const char *path, bool has_coords, double lat, double lon, const char *tags);
// Убрать из БД фото, перемещённое в корзину, вместе с миниатюрой.
void gpl_db_remove(int64_t id);
// Данные панели «Информация» в JSON (формат — в src/ffi.rs). Читает файл.
char *gpl_photo_info(const char *path, bool has_coords, double lat, double lon);
size_t gpl_photos_len(const GplPhotos *photos);
const GplPhoto *gpl_photos_data(const GplPhotos *photos);
void gpl_photos_free(GplPhotos *photos);

// ---------- Карта ----------

typedef struct {
    double lat;
    double lon;
    uint32_t count; // число фото в кластере
    uint32_t cover; // индекс фото в списке — миниатюра маркера
    int64_t id_sum; // вместе с cover и count — ключ кластера
} GplCluster;

typedef struct GplMapView GplMapView;

// Снимок карты для списка из gpl_db_geotagged: центр, зум Web Mercator (тайл 256),
// размер области в пикселях, размер ячейки кластеризации.
GplMapView *gpl_map_view(const GplPhotos *photos, double lat, double lon, double zoom,
                         double width, double height, double cell);
// Число фото списка в видимой области карты по дням, по возрастанию даты (фильтр карты
// по датам). Освобождать gpl_date_counts_free.
GplDateCount *gpl_visible_date_counts(const GplPhotos *photos, double lat, double lon, double zoom,
                                      double width, double height, size_t *len);
const GplCluster *gpl_map_view_clusters(const GplMapView *view, size_t *len);
// Индексы фото в видимой области, по возрастанию.
const uint32_t *gpl_map_view_visible(const GplMapView *view, size_t *len);
void gpl_map_view_free(GplMapView *view);

// Центр и целый зум (не больше max_zoom), при которых все фото помещаются в область.
bool gpl_photos_fit(const GplPhotos *photos, double width, double height, double pad,
                    double max_zoom, double *lat, double *lon, double *zoom);

// ---------- HEIC ----------

// rgba — width * height * 4 байт, выделенных malloc (библиотека освободит их free);
// orig_* — размеры оригинала после поворота.
typedef struct {
    uint8_t *rgba;
    uint32_t width;
    uint32_t height;
    uint32_t orig_width;
    uint32_t orig_height;
} GplImage;

// Декодер HEIC: повёрнутое изображение с короткой стороной не меньше min_side.
// Вызывается из потоков сканера, одновременно для разных файлов.
typedef bool (*GplHeifDecoder)(const char *path, uint32_t min_side, GplImage *out);
void gpl_set_heif_decoder(GplHeifDecoder decoder);
// Декодер кадра видео (тот же договор: кадр в начале видео, уже повёрнутый; orig_* не нужны).
void gpl_set_video_decoder(GplHeifDecoder decoder);

#endif

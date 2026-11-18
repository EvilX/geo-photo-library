// Обёртка над C-интерфейсом ядра (gpl.h): строки, настройки, запросы к БД, сканирование.
import CGeoPhotoCore
import Foundation

struct Photo: Identifiable, Hashable {
    let id: Int64
    let path: String
    /// `YYYY-MM-DD HH:MM:SS`
    let takenAt: String
    let thumbPath: String?
    let lat: Double
    let lon: Double
    let hasCoords: Bool

    var fileName: String { (path as NSString).lastPathComponent }
    var url: URL { URL(fileURLWithPath: path) }
}

struct DateCount: Hashable {
    let year: Int32
    let month: UInt32
    let day: UInt32
    let count: UInt32
}

struct Cluster {
    /// Ключ кластера: пока он тот же, маркер на карте не пересоздаётся.
    struct Key: Hashable {
        let cover: Int64
        let count: UInt32
        let idSum: Int64
    }

    let lat: Double
    let lon: Double
    let count: Int
    let cover: Photo
    let key: Key
}

struct MapSnapshot {
    var clusters: [Cluster] = []
    /// Индексы фото списка в видимой области, по дате съёмки.
    var visible: [Int] = []
}

struct ScanStats {
    let raw: GplScanStats
}

/// Данные панели «Информация» (JSON от ядра, формат — в `src/ffi.rs`).
struct PhotoInfo: Decodable {
    struct Row: Decodable, Hashable {
        let key: String
        let value: String
        /// Дата съёмки `YYYY-MM-DD HH:MM:SS`; `value` тогда — часовой пояс или пусто.
        let date: String?
    }

    struct Section: Decodable, Hashable {
        let title: String
        let rows: [Row]
    }

    let sections: [Section]
    let note: String?
    let all: Section?
}

enum ScanResult {
    case finished(ScanStats)
    case cancelled
    case failed(String)
}

/// Список фото из БД. Указатель ядра нужен карте: в нём лежат готовые проекции точек.
final class PhotoList: @unchecked Sendable {
    static let empty = PhotoList(nil)

    let photos: [Photo]
    private let raw: OpaquePointer?

    fileprivate init(_ raw: OpaquePointer?) {
        self.raw = raw
        let rows = UnsafeBufferPointer(start: gpl_photos_data(raw), count: gpl_photos_len(raw))
        photos = rows.map { r in
            Photo(
                id: r.id,
                path: String(cString: r.path),
                takenAt: String(cString: r.taken_at),
                thumbPath: r.thumb_path.map { String(cString: $0) },
                lat: r.lat,
                lon: r.lon,
                hasCoords: r.has_coords
            )
        }
    }

    deinit {
        gpl_photos_free(raw)
    }

    /// Что видно на карте: центр, зум Web Mercator, размер области и ячейки в пикселях.
    func snapshot(lat: Double, lon: Double, zoom: Double, size: CGSize, cell: Double) -> MapSnapshot {
        let view = gpl_map_view(raw, lat, lon, zoom, size.width, size.height, cell)
        defer { gpl_map_view_free(view) }
        var n = 0
        let clusters = UnsafeBufferPointer(start: gpl_map_view_clusters(view, &n), count: n).map { c in
            let cover = photos[Int(c.cover)]
            return Cluster(
                lat: c.lat,
                lon: c.lon,
                count: Int(c.count),
                cover: cover,
                key: .init(cover: cover.id, count: c.count, idSum: c.id_sum)
            )
        }
        let visible = UnsafeBufferPointer(start: gpl_map_view_visible(view, &n), count: n).map { Int($0) }
        return MapSnapshot(clusters: clusters, visible: visible)
    }

    /// Центр и зум, при которых все фото помещаются в область; `nil` — список пуст.
    /// Число фото списка в видимой области карты по дням — дерево фильтра карты по датам.
    func visibleDateCounts(lat: Double, lon: Double, zoom: Double, size: CGSize) -> [DateCount] {
        var n = 0
        return Core.dateCounts(gpl_visible_date_counts(raw, lat, lon, zoom, size.width, size.height, &n), n)
    }

    func fit(size: CGSize, pad: Double, maxZoom: Double) -> (lat: Double, lon: Double, zoom: Double)? {
        var (lat, lon, zoom) = (0.0, 0.0, 0.0)
        guard gpl_photos_fit(raw, size.width, size.height, pad, maxZoom, &lat, &lon, &zoom) else { return nil }
        return (lat, lon, zoom)
    }
}

/// Флаг отмены сканирования.
final class CancelToken: @unchecked Sendable {
    fileprivate let raw = gpl_cancel_new()

    func cancel() {
        gpl_cancel_set(raw)
    }

    deinit {
        gpl_cancel_free(raw)
    }
}

private final class ProgressBox {
    let report: (Int, Int) -> Void

    init(_ report: @escaping (Int, Int) -> Void) {
        self.report = report
    }
}

enum Core {
    /// Забрать строку у ядра и освободить её.
    private static func take(_ p: UnsafeMutablePointer<CChar>?) -> String? {
        guard let p else { return nil }
        defer { gpl_string_free(p) }
        return String(cString: p)
    }

    /// Язык интерфейса — первый поддерживаемый из системного списка языков.
    static func start() {
        let supported = ["en", "ru", "el"]
        let lang = Locale.preferredLanguages
            .compactMap { Locale(identifier: $0).language.languageCode?.identifier }
            .first(where: supported.contains) ?? "en"
        gpl_init(lang)
        gpl_set_heif_decoder(decodeHeif)
    }

    // MARK: Строки

    /// Строка интерфейса по имени функции в `i18n.rs`.
    static func text(_ key: String) -> String {
        take(gpl_text(key)) ?? key
    }

    static func photos(_ n: Int) -> String {
        take(gpl_text_photos(n)) ?? ""
    }

    static func processing(_ done: Int, _ total: Int) -> String {
        take(gpl_text_processing(done, total)) ?? ""
    }

    static func scanSummary(_ stats: ScanStats) -> String {
        take(gpl_text_scan_summary(stats.raw)) ?? ""
    }

    static func scanError(_ e: String) -> String {
        take(gpl_text_scan_error(e)) ?? e
    }

    static func inMapArea(_ visible: Int, _ total: Int) -> String {
        take(gpl_text_in_map_area(visible, total)) ?? ""
    }

    static func noCoordinatesDetail(_ name: String) -> String {
        take(gpl_text_no_coordinates_detail(name)) ?? ""
    }

    static func trashed(_ name: String) -> String {
        take(gpl_text_trashed(name)) ?? ""
    }

    static func trashFailed(_ name: String, _ e: String) -> String {
        take(gpl_text_trash_failed(name, e)) ?? e
    }

    static func dateLabel(year: Int32, month: UInt32 = 0, day: UInt32 = 0) -> String {
        take(gpl_text_date_label(year, month, day)) ?? ""
    }

    static func dateTitle(year: Int32, month: UInt32 = 0, day: UInt32 = 0) -> String {
        take(gpl_text_date_title(year, month, day)) ?? ""
    }

    // MARK: Настройки

    static var libraryPath: String? {
        take(gpl_config_library_path())
    }

    /// Возвращает текст ошибки, если настройки не сохранились.
    static func setLibraryPath(_ path: String) -> String? {
        var err: UnsafeMutablePointer<CChar>?
        return gpl_config_set_library_path(path, &err) ? nil : take(err) ?? ""
    }

    static var mapView: (lat: Double, lon: Double, zoom: Double)? {
        var (lat, lon, zoom) = (0.0, 0.0, 0.0)
        return gpl_config_map_view(&lat, &lon, &zoom) ? (lat, lon, zoom) : nil
    }

    static func setMapView(lat: Double, lon: Double, zoom: Double) {
        report { gpl_config_set_map_view(lat, lon, zoom, $0) }
    }

    static var newestFirst: Bool {
        gpl_config_newest_first()
    }

    static func setNewestFirst(_ newest: Bool) {
        report { gpl_config_set_newest_first(newest, $0) }
    }

    static var thumbScale: Int {
        Int(gpl_config_thumb_scale())
    }

    static func setThumbScale(_ percent: Int) {
        report { gpl_config_set_thumb_scale(UInt32(max(percent, 0)), $0) }
    }

    /// Идентификатор пакета просмотрщика или редактора; `nil` — выбирать автоматически.
    static func app(editor: Bool) -> String? {
        take(gpl_config_app(editor))
    }

    static func setApp(editor: Bool, id: String?) {
        report { gpl_config_set_app(editor, id ?? "", $0) }
    }

    /// Выполнить сохранение настроек; ошибку — в stderr.
    private static func report(_ save: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Bool) {
        var err: UnsafeMutablePointer<CChar>?
        if !save(&err), let e = take(err) {
            FileHandle.standardError.write(Data("Не удалось сохранить настройки: \(e)\n".utf8))
        }
    }

    // MARK: БД (вызывать не из главного потока)

    /// Убрать из БД фото, перемещённое в корзину, вместе с миниатюрой.
    static func remove(id: Int64) {
        gpl_db_remove(id)
    }

    /// Данные панели «Информация»: читает файл и EXIF.
    static func info(_ photo: Photo) -> PhotoInfo? {
        guard let json = take(gpl_photo_info(photo.path, photo.hasCoords, photo.lat, photo.lon)) else { return nil }
        return try? JSONDecoder().decode(PhotoInfo.self, from: Data(json.utf8))
    }

    static func count() -> Int {
        Int(gpl_db_count())
    }

    static func dateCounts() -> [DateCount] {
        var n = 0
        return dateCounts(gpl_db_date_counts(&n), n)
    }

    /// Счётчики из ядра; память освобождается здесь.
    static func dateCounts(_ raw: UnsafeMutablePointer<GplDateCount>?, _ n: Int) -> [DateCount] {
        defer { gpl_date_counts_free(raw, n) }
        return UnsafeBufferPointer(start: raw, count: n).map {
            DateCount(year: $0.year, month: $0.month, day: $0.day, count: $0.count)
        }
    }

    static func photos(year: Int32, month: UInt32, day: UInt32) -> [Photo] {
        PhotoList(gpl_db_photos_by_date(year, month, day)).photos
    }

    /// Фото с координатами за период узла дерева дат; `nil` — за всё время.
    static func geotagged(_ period: DatePeriod?) -> PhotoList {
        PhotoList(gpl_db_geotagged(period?.year ?? 0, period?.month ?? 0, period?.day ?? 0))
    }

    // MARK: Сканирование

    /// Блокирует поток до конца сканирования; `progress` вызывается из него же.
    static func scan(root: String, cancel: CancelToken, progress: @escaping (Int, Int) -> Void) -> ScanResult {
        let box = Unmanaged.passRetained(ProgressBox(progress))
        defer { box.release() }
        var stats = GplScanStats()
        var err: UnsafeMutablePointer<CChar>?
        let code = gpl_scan(
            root,
            { ctx, done, total in
                guard let ctx else { return }
                Unmanaged<ProgressBox>.fromOpaque(ctx).takeUnretainedValue().report(done, total)
            },
            box.toOpaque(),
            cancel.raw,
            &stats,
            &err
        )
        switch Int(code) {
        case GPL_SCAN_OK: return .finished(ScanStats(raw: stats))
        case GPL_SCAN_CANCELLED: return .cancelled
        default: return .failed(take(err) ?? "")
        }
    }
}

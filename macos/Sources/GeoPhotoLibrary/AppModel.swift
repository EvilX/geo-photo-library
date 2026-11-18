// Состояние приложения: папка библиотеки, фоновое сканирование, данные вкладок, действия с фото.
import AppKit
import Foundation
import Observation
import UniformTypeIdentifiers

enum AppTab: String, CaseIterable {
    case map, dates

    var title: String {
        Core.text(self == .map ? "tab_map" : "tab_dates")
    }
}

/// Узел дерева дат: год → месяц → день.
struct DateNode: Identifiable, Hashable {
    let year: Int32
    var month: UInt32 = 0
    var day: UInt32 = 0
    var count: UInt32
    var children: [DateNode]?

    var id: String { "\(year)-\(month)-\(day)" }
    var label: String { Core.dateLabel(year: year, month: month, day: day) }
    var title: String { Core.dateTitle(year: year, month: month, day: day) }

    static func tree(_ counts: [DateCount]) -> [DateNode] {
        var years: [DateNode] = []
        for c in counts {
            if years.last?.year != c.year {
                years.append(DateNode(year: c.year, count: 0, children: []))
            }
            let y = years.count - 1
            if years[y].children?.last?.month != c.month {
                years[y].children?.append(DateNode(year: c.year, month: c.month, count: 0, children: []))
            }
            let m = years[y].children!.count - 1
            years[y].children![m].children?.append(DateNode(year: c.year, month: c.month, day: c.day, count: c.count))
            years[y].children![m].count += c.count
            years[y].count += c.count
        }
        // Годы — от новых к старым, как в версии GTK; месяцы и дни — по порядку.
        return years.reversed()
    }
}

/// Сообщение в окне: заголовок и пояснение.
struct AlertMessage: Identifiable {
    let id = UUID()
    let title: String
    let detail: String
}

/// Карта, которой модель отдаёт команды (её координатор регистрируется сам).
@MainActor
protocol MapController: AnyObject {
    /// Сдвинуть карту к фото, не меняя зум.
    func center(on photo: Photo)
    /// Перейти к месту съёмки крупным планом и выделить фото в списке, когда оно появится.
    func show(_ photo: Photo)
    /// Показать точку с окрестностями радиуса `radius` метров; `nil` — крупным планом.
    func go(lat: Double, lon: Double, radius: Double?, maxZoom: Double)
    /// Отметить текущее местоположение точкой.
    func markLocation(lat: Double, lon: Double)
}

@MainActor
@Observable
final class AppModel {
    /// Во время сканирования интерфейс перечитывает БД с этим интервалом.
    private static let reloadInterval: TimeInterval = 3
    static let thumbScaleRange = 50.0...200.0
    /// Шаг слайдера масштаба и кнопок «−»/«+».
    static let thumbScaleStep = 5.0
    static let thumbScaleButtonStep = 10.0

    var tab: AppTab = ProcessInfo.processInfo.environment["GEO_PHOTO_LIBRARY_TAB"] == "dates" ? .dates : .map
    private(set) var libraryPath: String?
    var status = ""
    /// Доля обработанных файлов; `nil` — сканирование не идёт или ещё ищет файлы.
    private(set) var progress: Double?
    private(set) var scanning = false

    private(set) var dates: [DateNode] = []
    private(set) var geotagged = PhotoList.empty
    /// Растёт при каждом изменении данных в БД — вкладки перечитывают по нему свои списки.
    private(set) var revision = 0

    /// Сортировка сеток: одна на обе вкладки, сохраняется в настройках.
    var newestFirst: Bool {
        didSet {
            if newestFirst != oldValue {
                Core.setNewestFirst(newestFirst)
            }
        }
    }

    /// Масштаб миниатюр в процентах; сохраняется по окончании движения слайдера (`saveThumbScale`).
    var thumbScale: Double
    /// Просмотрщик и редактор из настроек (идентификаторы пакетов); `nil` — автоматически.
    var viewerID: String? {
        didSet { Core.setApp(editor: false, id: viewerID) }
    }

    var editorID: String? {
        didSet { Core.setApp(editor: true, id: editorID) }
    }

    /// Фото в видимой области карты по дате съёмки и подпись над списком.
    var mapVisible: [Photo] = []
    var mapInfo = ""
    var mapSelection: Photo?
    var dateSelection: Photo?
    var infoOpen = false
    var alert: AlertMessage?
    weak var map: MapController?

    private var cancel: CancelToken?
    /// Папку сменили во время сканирования: текущее отменено, после него начнётся новое.
    private var rescanPending = false
    private var reloadTimer: Timer?
    private var dateCounts: [DateCount] = []
    /// Номер последнего запрошенного чтения БД.
    private var reloadGeneration = 0

    init() {
        libraryPath = Core.libraryPath
        newestFirst = Core.newestFirst
        thumbScale = Double(Core.thumbScale)
        viewerID = Core.app(editor: false)
        editorID = Core.app(editor: true)
    }

    /// Фото, выбранное на открытой вкладке, — его показывает панель «Информация».
    var selectedPhoto: Photo? {
        tab == .map ? mapSelection : dateSelection
    }

    /// Список в порядке выбранной сортировки (на входе — по дате съёмки).
    func ordered(_ photos: [Photo]) -> [Photo] {
        newestFirst ? photos.reversed() : photos
    }

    func stepThumbScale(_ step: Double) {
        let snapped = ((thumbScale + step) / Self.thumbScaleStep).rounded() * Self.thumbScaleStep
        thumbScale = min(max(snapped, Self.thumbScaleRange.lowerBound), Self.thumbScaleRange.upperBound)
        saveThumbScale()
    }

    func saveThumbScale() {
        Core.setThumbScale(Int(thumbScale))
    }

    // MARK: Сканирование

    func start() {
        reload()
        rescan()
    }

    func setLibrary(_ path: String) {
        if let e = Core.setLibraryPath(path) {
            status = Core.scanError(e)
            return
        }
        libraryPath = path
        rescan()
    }

    func rescan() {
        guard let root = libraryPath else { return }
        if scanning {
            rescanPending = true
            cancel?.cancel()
            status = Core.text("stopping_scan")
            return
        }
        scanning = true
        progress = nil
        status = Core.text("searching")
        let token = CancelToken()
        cancel = token
        reloadTimer = Timer.scheduledTimer(withTimeInterval: Self.reloadInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.reload() }
        }
        Thread.detachNewThread { [weak self] in
            let result = Core.scan(root: root, cancel: token) { done, total in
                Task { @MainActor in self?.report(done, total, token) }
            }
            Task { @MainActor in self?.finished(result) }
        }
    }

    private func report(_ done: Int, _ total: Int, _ token: CancelToken) {
        // Сообщения отменённого сканирования не затирают «Останавливаю…».
        guard scanning, cancel === token, !rescanPending, total > 0 else { return }
        progress = Double(done) / Double(total)
        status = Core.processing(done, total)
    }

    private func finished(_ result: ScanResult) {
        scanning = false
        progress = nil
        cancel = nil
        reloadTimer?.invalidate()
        reloadTimer = nil
        if rescanPending {
            rescanPending = false
            reload()
            rescan()
            return
        }
        switch result {
        case .finished(let stats): status = Core.scanSummary(stats)
        case .cancelled: status = ""
        case .failed(let e): status = Core.scanError(e)
        }
        reload(keepStatus: true)
    }

    /// Перечитать БД в фоне; данные заменяются, только если изменились.
    func reload(keepStatus: Bool = false) {
        reloadGeneration += 1
        let generation = reloadGeneration
        Task.detached(priority: .userInitiated) {
            let counts = Core.dateCounts()
            let geotagged = Core.geotagged()
            let total = Core.count()
            await MainActor.run { self.apply(generation, counts, geotagged, total, keepStatus) }
        }
    }

    private func apply(_ generation: Int, _ counts: [DateCount], _ geo: PhotoList, _ total: Int, _ keepStatus: Bool) {
        // Чтения идут параллельно и могут ждать сканер: результат более раннего, пришедший
        // после более позднего, затёр бы свежие данные (например, пустой БД до первого скана).
        guard generation == reloadGeneration else { return }
        var changed = false
        if counts != dateCounts {
            dateCounts = counts
            dates = DateNode.tree(counts)
            changed = true
        }
        if geo.photos != geotagged.photos {
            geotagged = geo
            changed = true
        }
        if changed {
            revision += 1
        }
        if !scanning, !keepStatus {
            status = Core.photos(total)
        }
    }

    // MARK: Действия с фото

    /// Просмотр: программа из настроек, иначе — программа по умолчанию для типа файла.
    func open(_ photo: Photo) {
        if !Apps.open(photo.url, with: viewerID) {
            NSWorkspace.shared.open(photo.url)
        }
    }

    /// Редактирование: редактор из настроек, иначе найденный графический редактор,
    /// иначе — программа по умолчанию.
    func edit(_ photo: Photo) {
        if !Apps.open(photo.url, with: editorID), !Apps.open(photo.url, with: Apps.findEditor(for: photo.url)) {
            NSWorkspace.shared.open(photo.url)
        }
    }

    func showInFinder(_ photo: Photo) {
        NSWorkspace.shared.activateFileViewerSelecting([photo.url])
    }

    /// Положить файл в буфер обмена: как файл (вставка в Finder) и как путь.
    func copy(_ photo: Photo) {
        let board = NSPasteboard.general
        board.clearContents()
        board.writeObjects([photo.url as NSURL])
        board.addTypes([.string], owner: nil)
        board.setString(photo.path, forType: .string)
    }

    /// Открыть панель «Информация» для фото.
    func showInfo(_ photo: Photo) {
        select(photo)
        infoOpen = true
    }

    /// Перейти к месту съёмки на карте; без координат — предупреждение.
    func showOnMap(_ photo: Photo) {
        guard photo.hasCoords else {
            alert = AlertMessage(
                title: Core.text("no_coordinates"), detail: Core.noCoordinatesDetail(photo.fileName)
            )
            return
        }
        tab = .map
        map?.show(photo)
    }

    /// Переместить файл в корзину и убрать фото из БД и интерфейса.
    func trash(_ photo: Photo) {
        do {
            try FileManager.default.trashItem(at: photo.url, resultingItemURL: nil)
        } catch {
            status = Core.trashFailed(photo.fileName, error.localizedDescription)
            return
        }
        if mapSelection?.id == photo.id { mapSelection = nil }
        if dateSelection?.id == photo.id { dateSelection = nil }
        status = Core.trashed(photo.fileName)
        Task.detached(priority: .userInitiated) {
            Core.remove(id: photo.id)
            await MainActor.run { self.reload(keepStatus: true) }
        }
    }

    /// Выделить фото на открытой вкладке (правый щелчок, меню).
    func select(_ photo: Photo) {
        if tab == .map {
            mapSelection = photo
        } else {
            dateSelection = photo
        }
    }
}

/// Программы для просмотра и редактирования фото.
@MainActor
enum Apps {
    struct Choice: Identifiable, Hashable {
        /// Идентификатор пакета.
        let id: String
        let name: String
        let url: URL
    }

    /// Программы, которые открывают фото (JPEG, HEIC, PNG), по имени; без этой программы.
    static func imageApps() -> [Choice] {
        var seen: Set<String> = [Bundle.main.bundleIdentifier ?? ""]
        var apps: [Choice] = []
        for type in [UTType.jpeg, .heic, .png] {
            for url in NSWorkspace.shared.urlsForApplications(toOpen: type) {
                guard let id = Bundle(url: url)?.bundleIdentifier, seen.insert(id).inserted else { continue }
                apps.append(Choice(id: id, name: FileManager.default.displayName(atPath: url.path), url: url))
            }
        }
        return apps.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    /// Открыть файл в программе с данным идентификатором; `false` — её нет.
    static func open(_ file: URL, with bundleID: String?) -> Bool {
        guard let bundleID, let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else {
            return false
        }
        NSWorkspace.shared.open([file], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration()) { _, error in
            if let error {
                FileHandle.standardError.write(Data("не удалось запустить \(bundleID): \(error)\n".utf8))
            }
        }
        return true
    }

    /// Графический редактор для файла: программа, открывающая его тип, из категорий
    /// «графика и дизайн» или «фотография» (как RasterGraphics/Photography на Linux).
    static func findEditor(for file: URL) -> String? {
        let categories = ["public.app-category.graphics-design", "public.app-category.photography"]
        let own = Bundle.main.bundleIdentifier
        return NSWorkspace.shared.urlsForApplications(toOpen: file).lazy.compactMap { url -> String? in
            guard let bundle = Bundle(url: url), let id = bundle.bundleIdentifier, id != own,
                  let category = bundle.object(forInfoDictionaryKey: "LSApplicationCategoryType") as? String,
                  categories.contains(category)
            else { return nil }
            return id
        }.first
    }
}

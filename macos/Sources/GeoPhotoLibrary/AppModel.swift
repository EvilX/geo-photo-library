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

/// Период: год, месяц (`day == 0`) или день; `month == 0` — весь год.
struct DatePeriod: Hashable {
    let year: Int32
    var month: UInt32 = 0
    var day: UInt32 = 0

    var id: String { "\(year)-\(month)-\(day)" }
}

/// Узел дерева дат: год → месяц → день.
struct DateNode: Identifiable, Hashable {
    let year: Int32
    var month: UInt32 = 0
    var day: UInt32 = 0
    var count: UInt32
    var children: [DateNode]?

    var period: DatePeriod { DatePeriod(year: year, month: month, day: day) }
    var id: String { period.id }
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
        // Годы и месяцы — от новых к старым, как в версии GTK; дни — по порядку.
        for y in years.indices {
            years[y].children?.reverse()
        }
        return years.reversed()
    }

    /// Узел дерева `nodes` с идентификатором `id`.
    static func find(_ id: ID?, in nodes: [DateNode]) -> DateNode? {
        guard let id else { return nil }
        for year in nodes {
            if year.id == id { return year }
            for month in year.children ?? [] {
                if month.id == id { return month }
                if let day = month.children?.first(where: { $0.id == id }) { return day }
            }
        }
        return nil
    }
}

/// Выделенные фото сетки. `anchor` — опорное фото: от него Shift-щелчок выделяет
/// диапазон, к нему прокручивается сетка.
struct PhotoSelection: Equatable {
    private(set) var photos: [Photo.ID: Photo] = [:]
    private(set) var anchor: Photo.ID?

    init(_ photo: Photo? = nil) {
        if let photo {
            photos[photo.id] = photo
            anchor = photo.id
        }
    }

    /// Несколько фото; опорное — `anchor` (или первое).
    init(_ list: some Collection<Photo>, anchor: Photo.ID? = nil) {
        for photo in list {
            photos[photo.id] = photo
        }
        self.anchor = anchor ?? list.first?.id
    }

    var isEmpty: Bool { photos.isEmpty }
    var count: Int { photos.count }
    /// Выделенное фото, если оно одно.
    var single: Photo? { photos.count == 1 ? photos.first?.value : nil }
    /// Все выделенные — по пути к файлу, чтобы порядок не зависел от словаря.
    var all: [Photo] { photos.values.sorted { $0.path < $1.path } }

    func contains(_ photo: Photo) -> Bool { photos[photo.id] != nil }

    /// Добавить фото или снять с него выделение; оно становится опорным.
    mutating func toggle(_ photo: Photo) {
        photos[photo.id] = photos[photo.id] == nil ? photo : nil
        anchor = photo.id
    }

    mutating func formUnion(_ other: PhotoSelection) {
        photos.merge(other.photos) { a, _ in a }
        anchor = other.anchor ?? anchor
    }

    /// Оставить только фото из `list` (остальные пропали из сетки).
    mutating func keep(only list: [Photo]) {
        guard !photos.isEmpty else { return }
        let ids = Set(list.map(\.id))
        photos = photos.filter { ids.contains($0.key) }
    }

    mutating func remove(_ ids: Set<Photo.ID>) {
        photos = photos.filter { !ids.contains($0.key) }
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
    /// Фото с координатами на карте — за период фильтра `mapPeriod`.
    private(set) var geotagged = PhotoList.empty
    /// Все фото с координатами, без фильтра: по ним считается дерево фильтра.
    private(set) var allGeotagged = PhotoList.empty
    /// Дерево фильтра карты: даты фото с координатами в видимой области карты
    /// (без учёта фильтра — иначе в нём остался бы один выбранный период).
    private(set) var mapDates: [DateNode] = []
    /// Фильтр карты по датам; `nil` — всё время. Смена перечитывает фото карты,
    /// не трогая её положение и масштаб.
    var mapPeriod: DatePeriod? {
        didSet {
            if mapPeriod != oldValue {
                reload(keepStatus: true)
            }
        }
    }
    /// Панель фильтра по датам слева от карты открыта.
    var mapFilterOpen = false
    /// Раскрытые узлы дерева фильтра (`DateNode.id`): панель при закрытии убирается
    /// целиком, и без этого при новом открытии дерево было бы свёрнуто.
    var mapExpanded: Set<String> = []
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
    var mapSelection = PhotoSelection()
    var dateSelection = PhotoSelection()
    /// Фото вкладки дат за выбранный период (по дате съёмки) — для «Выделить все».
    var datePhotos: [Photo] = []
    var infoOpen = false
    var alert: AlertMessage?
    /// Фото, удаление которых ждёт подтверждения в окне.
    var pendingTrash: [Photo]?
    weak var map: MapController?

    private var cancel: CancelToken?
    /// Папку сменили во время сканирования: текущее отменено, после него начнётся новое.
    private var rescanPending = false
    private var reloadTimer: Timer?
    private var dateCounts: [DateCount] = []
    private var mapDateCounts: [DateCount] = []
    /// Номер последнего запрошенного чтения БД.
    private var reloadGeneration = 0

    init() {
        libraryPath = Core.libraryPath
        newestFirst = Core.newestFirst
        thumbScale = Double(Core.thumbScale)
        viewerID = Core.app(editor: false)
        editorID = Core.app(editor: true)
    }

    /// Выделение на открытой вкладке.
    var selection: PhotoSelection {
        get { tab == .map ? mapSelection : dateSelection }
        set {
            if tab == .map {
                mapSelection = newValue
            } else {
                dateSelection = newValue
            }
        }
    }

    /// Фото, выбранное на открытой вкладке, если оно одно, — его показывает панель «Информация».
    var selectedPhoto: Photo? { selection.single }

    /// Выделить все фото сетки открытой вкладки.
    func selectAll() {
        selection = PhotoSelection(tab == .map ? mapVisible : datePhotos, anchor: selection.anchor)
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
        let period = mapPeriod
        Task.detached(priority: .userInitiated) {
            let counts = Core.dateCounts()
            let all = Core.geotagged(nil)
            let geotagged = period == nil ? all : Core.geotagged(period)
            let total = Core.count()
            await MainActor.run {
                self.apply(generation, Loaded(counts: counts, all: all, geo: geotagged, total: total), keepStatus)
            }
        }
    }

    private struct Loaded: Sendable {
        let counts: [DateCount]
        let all: PhotoList
        let geo: PhotoList
        let total: Int
    }

    /// Даты фото в видимой области карты — пересчитываются при каждом её обновлении.
    /// Выбранный период остаётся, даже если его фото ушли из видимой области.
    func setMapDateCounts(_ counts: [DateCount]) {
        guard counts != mapDateCounts else { return }
        mapDateCounts = counts
        mapDates = DateNode.tree(counts)
    }

    private func apply(_ generation: Int, _ loaded: Loaded, _ keepStatus: Bool) {
        // Чтения идут параллельно и могут ждать сканер: результат более раннего, пришедший
        // после более позднего, затёр бы свежие данные (например, пустой БД до первого скана).
        guard generation == reloadGeneration else { return }
        var changed = false
        if loaded.counts != dateCounts {
            dateCounts = loaded.counts
            dates = DateNode.tree(loaded.counts)
            changed = true
        }
        if loaded.all.photos != allGeotagged.photos {
            allGeotagged = loaded.all
        }
        if loaded.geo.photos != geotagged.photos {
            geotagged = loaded.geo
            changed = true
        }
        if changed {
            revision += 1
        }
        let total = loaded.total
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

    /// Положить файлы в буфер обмена: как файлы (вставка в Finder) и как пути по строке.
    func copy(_ photos: [Photo]) {
        guard !photos.isEmpty else { return }
        let board = NSPasteboard.general
        board.clearContents()
        board.writeObjects(photos.map { $0.url as NSURL })
        board.addTypes([.string], owner: nil)
        board.setString(photos.map(\.path).joined(separator: "\n"), forType: .string)
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

    /// Удалить фото — после подтверждения в окне (`pendingTrash`).
    func trash(_ photos: [Photo]) {
        if !photos.isEmpty {
            pendingTrash = photos
        }
    }

    /// Вопрос перед удалением `photos`.
    static func trashQuestion(_ photos: [Photo]) -> String {
        Core.confirmTrash(photos.count == 1 ? photos[0].fileName : Core.photos(photos.count))
    }

    /// Переместить файлы в корзину и убрать фото из БД и интерфейса. В строке состояния —
    /// имя фото (или их число), при ошибке — первая из них.
    func confirmTrash(_ photos: [Photo]) {
        pendingTrash = nil
        var moved: [Photo] = []
        var failure: String?
        for photo in photos {
            do {
                try FileManager.default.trashItem(at: photo.url, resultingItemURL: nil)
                moved.append(photo)
            } catch {
                failure = failure ?? Core.trashFailed(photo.fileName, error.localizedDescription)
            }
        }
        let ids = Set(moved.map(\.id))
        mapSelection.remove(ids)
        dateSelection.remove(ids)
        status = failure ?? (moved.count == 1 ? Core.trashed(moved[0].fileName) : Core.trashed(Core.photos(moved.count)))
        guard !moved.isEmpty else { return }
        Task.detached(priority: .userInitiated) {
            for id in ids {
                Core.remove(id: id)
            }
            await MainActor.run { self.reload(keepStatus: true) }
        }
    }

    /// Выделить фото на открытой вкладке (правый щелчок, меню).
    func select(_ photo: Photo) {
        selection = PhotoSelection(photo)
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

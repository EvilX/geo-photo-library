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
/// Фильтр вкладки дат по координатам (у фото) или только видео; значение — код фильтра в ядре (`GeoFilter::index`).
enum GeoFilter: UInt32, CaseIterable, Sendable {
    case all, with, without, video

    var title: String {
        switch self {
        case .all: Core.text("geo_all")
        case .with: Core.text("geo_with")
        case .without: Core.text("geo_without")
        case .video: Core.text("geo_video")
        }
    }
}

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
    /// Среди выделенных нет фото — только видео (или пусто). Без сортировки: спрашивает меню
    /// при каждой смене выделения.
    var allVideo: Bool { photos.values.allSatisfy(\.isVideo) }

    func contains(_ photo: Photo) -> Bool { photos[photo.id] != nil }
    func contains(id: Photo.ID) -> Bool { photos[id] != nil }

    /// Добавить фото или снять с него выделение; оно становится опорным.
    mutating func toggle(_ photo: Photo) {
        photos[photo.id] = photos[photo.id] == nil ? photo : nil
        anchor = photo.id
    }

    mutating func formUnion(_ other: PhotoSelection) {
        photos.merge(other.photos) { a, _ in a }
        anchor = other.anchor ?? anchor
    }

    /// Привести к списку сетки `list`: фото, которых в нём нет, из выделения выпадают, а у
    /// оставшихся значения берутся из списка — после правки EXIF или пересканирования в
    /// выделении иначе остались бы прежние координаты и дата.
    mutating func sync(with list: [Photo]) {
        guard !photos.isEmpty else { return }
        var fresh: [Photo.ID: Photo] = [:]
        for photo in list where photos[photo.id] != nil {
            fresh[photo.id] = photo
        }
        if fresh != photos {
            photos = fresh
        }
    }

    mutating func remove(_ ids: Set<Photo.ID>) {
        photos = photos.filter { !ids.contains($0.key) }
    }
}

/// Редактор EXIF для фото (лист над окном).
struct ExifEdit: Identifiable {
    let id = UUID()
    let photos: [Photo]
    /// Положение главной карты — с него открывается карта редактора, если у фото нет координат.
    let view: (lat: Double, lon: Double, zoom: Double)?
}

/// Сообщение в окне: заголовок и пояснение.
struct AlertMessage: Identifiable {
    let id = UUID()
    let title: String
    let detail: String
}

/// Карта, которой модель отдаёт команды (её координатор регистрируется сам).
@MainActor
protocol MapController: PlaceTarget {
    /// Сдвинуть карту к фото, не меняя зум.
    func center(on photo: Photo)
    /// Перейти к месту съёмки крупным планом и выделить фото в списке, когда оно появится.
    func show(_ photo: Photo)
    /// Текущие центр и зум — с них открывается редактор координат фото без координат.
    var currentView: (lat: Double, lon: Double, zoom: Double)? { get }
    /// Файлы миниатюр перезаписаны — маркеры перечитывают свои картинки.
    func thumbnailsChanged()
    /// Сменили папку библиотеки: прежнее положение карты не нужно, подогнать под новые фото.
    func libraryChanged()
}

/// Карта, куда поиск местности и «Моё местоположение» переводят взгляд.
@MainActor
protocol PlaceTarget: AnyObject {
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
    /// Ход записи изменений или перемещения в корзину («… 3 / 10»): пока он есть, строка
    /// состояния показывает его, а не `status` (тот во время сканирования меняется постоянно).
    private(set) var fileStatus: String?
    /// Доля обработанных файлов; `nil` — сканирование не идёт или ещё ищет файлы.
    private(set) var progress: Double?
    private(set) var scanning = false

    private(set) var dates: [DateNode] = []
    /// Фильтр вкладки дат по координатам: смена перечитывает дерево дат.
    var dateFilter = Core.debugGeoFilter {
        didSet {
            // Фото карты от этого фильтра не зависят — перечитываются только даты.
            if dateFilter != oldValue {
                reload(.dates, keepStatus: true)
            }
        }
    }
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
            guard mapPeriod != oldValue else { return }
            if mapPeriod == nil {
                // «Всё время» — список уже есть; чтение прежнего периода, если оно ещё идёт, не нужно.
                generations[ReloadParts.period.rawValue, default: 0] += 1
                if geotagged !== allGeotagged {
                    geotagged = allGeotagged
                }
            } else {
                reload(.period, keepStatus: true)
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
    /// Растёт, когда изменены сами файлы (правка EXIF, пересканирование): миниатюры на диске
    /// перезаписаны под прежними именами, данные панели «Информация» устарели.
    private(set) var filesRevision = 0
    /// Перечитать список вкладки дат при следующем применении `reload` (фото изменены).
    @ObservationIgnored private var forceDateReload = false

    /// Сортировка сеток: одна на обе вкладки, сохраняется в настройках.
    var newestFirst: Bool {
        didSet {
            if newestFirst != oldValue {
                Core.setNewestFirst(newestFirst)
                mapList = ordered(mapVisible)
                dateList = ordered(datePhotos)
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

    /// Проверять обновления при запуске; новая версия, если она есть (уведомление над вкладками).
    var checkUpdates: Bool {
        didSet { Core.setCheckUpdates(checkUpdates) }
    }
    var newVersion: String?

    /// Фото в видимой области карты по дате съёмки и подпись над списком.
    var mapVisible: [Photo] = [] {
        didSet { mapList = ordered(mapVisible) }
    }
    var mapInfo = ""
    var mapSelection = PhotoSelection()
    var dateSelection = PhotoSelection()
    /// Фото вкладки дат за выбранный период (по дате съёмки) — для «Выделить все».
    var datePhotos: [Photo] = [] {
        didSet { dateList = ordered(datePhotos) }
    }
    /// Те же списки в порядке выбранной сортировки — их показывают сетки. Хранятся, а не
    /// считаются при каждой отрисовке: разворот списка — копия всех фото.
    private(set) var mapList: [Photo] = []
    private(set) var dateList: [Photo] = []
    var infoOpen = false
    var alert: AlertMessage?
    /// Фото, удаление которых ждёт подтверждения в окне.
    var pendingTrash: [Photo]?
    /// Открытый редактор координат.
    var exifEdit: ExifEdit?
    weak var map: MapController?

    private var cancel: CancelToken?
    /// Папку сменили во время сканирования: текущее отменено, после него начнётся новое.
    private var rescanPending = false
    private var reloadTimer: Timer?
    private var dateCounts: [DateCount] = []
    private var mapDateCounts: [DateCount] = []
    /// Номер последнего запрошенного чтения БД — свой у каждой части данных (`ReloadParts`):
    /// чтение одних только дат не отменяет идущее чтение фото карты.
    @ObservationIgnored private var generations: [Int: Int] = [:]
    /// Сколько раз подряд БД не прочиталась (занята) — после нескольких попыток повтор прекращается.
    @ObservationIgnored private var reloadFailures = 0

    /// Запись в файлы и перемещение в корзину — по очереди, вне главного потока.
    @ObservationIgnored private let fileQueue = DispatchQueue(label: "geo-photo-library.files", qos: .userInitiated)
    /// Фото, в файлы которых сейчас пишутся изменения; фото, которые перемещаются в корзину;
    /// фото, записанные, но ещё не перечитанные из БД (в списках — прежние координаты).
    /// Над ними нельзя начинать новую правку или удаление.
    @ObservationIgnored private var writing: Set<Photo.ID> = []
    @ObservationIgnored private var trashing: Set<Photo.ID> = []
    @ObservationIgnored private var awaitingRefresh: Set<Photo.ID> = []

    init() {
        libraryPath = Core.libraryPath
        newestFirst = Core.newestFirst
        thumbScale = Double(Core.thumbScale)
        viewerID = Core.app(editor: false)
        editorID = Core.app(editor: true)
        checkUpdates = Core.checkUpdates
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
    private func ordered(_ photos: [Photo]) -> [Photo] {
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
        if checkUpdates {
            checkForUpdates()
        }
    }

    /// Спросить GitHub о последнем выпуске. Ошибки — только в stderr: уведомление
    /// появляется, лишь если найдена версия новее текущей.
    private func checkForUpdates() {
        if let debug = Core.debugUpdate {
            newVersion = debug
            return
        }
        guard let url = Core.releasesURL else { return }
        var request = URLRequest(url: url, timeoutInterval: 15)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        Task {
            do {
                let (data, response) = try await URLSession.shared.data(for: request)
                if let http = response as? HTTPURLResponse, http.statusCode != 200 {
                    throw URLError(.badServerResponse)
                }
                let version = Core.newerVersion(data)
                newVersion = version
            } catch {
                FileHandle.standardError.write(Data("Проверка обновлений: \(error)\n".utf8))
            }
        }
    }

    func setLibrary(_ path: String) {
        if let e = Core.setLibraryPath(path) {
            status = Core.scanError(e)
            return
        }
        libraryPath = path
        // Фильтр по датам и положение карты относились к прежней библиотеке.
        mapPeriod = nil
        map?.libraryChanged()
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
            // Прерванное сканирование могло успеть обновить часть файлов.
            filesChanged(nil)
            reload(force: true)
            rescan()
            return
        }
        // Сканер перезаписал миниатюры изменённых файлов под прежними именами. Сколько их
        // обновило прерванное сканирование, неизвестно — считаем, что обновило.
        var updated = true
        switch result {
        case .finished(let stats):
            status = Core.scanSummary(stats)
            updated = stats.raw.updated > 0
        case .cancelled: status = ""
        case .failed(let e): status = Core.scanError(e)
        }
        if updated {
            filesChanged(nil)
        }
        reload(keepStatus: true, force: updated)
    }

    /// Файлы фото изменены: забыть миниатюры (`nil` — все) и сообщить тем, кто их показывает.
    private func filesChanged(_ thumbs: [String]?) {
        ThumbnailCache.shared.forget(thumbs)
        filesRevision += 1
        map?.thumbnailsChanged()
    }

    /// Части данных, которые читает `reload`.
    struct ReloadParts: OptionSet, Sendable {
        let rawValue: Int
        /// Дерево вкладки дат и общее число фото.
        static let dates = ReloadParts(rawValue: 1)
        /// Все фото с координатами.
        static let all = ReloadParts(rawValue: 2)
        /// Фото карты за период фильтра (без фильтра это тот же список, что `all`).
        static let period = ReloadParts(rawValue: 4)
        static let everything: ReloadParts = [.dates, .all, .period]
        static let each: [ReloadParts] = [.dates, .all, .period]
    }

    /// Перечитать БД в фоне — только части `parts`; данные заменяются, только если изменились.
    /// `force` — список вкладки дат перечитывается, даже если даты и фото карты те же
    /// (изменены сами фото).
    func reload(_ parts: ReloadParts = .everything, keepStatus: Bool = false, force: Bool = false) {
        // Флаг живёт до применения: более позднее чтение заменит это, но список всё равно перечитается.
        forceDateReload = forceDateReload || force
        var stamps: [Int: Int] = [:]
        for part in ReloadParts.each where parts.contains(part) {
            generations[part.rawValue, default: 0] += 1
            stamps[part.rawValue] = generations[part.rawValue]
        }
        let period = mapPeriod
        let filter = dateFilter
        let shown = (all: allGeotagged, geo: geotagged)
        // Фото, записанные до этого запроса: после него в списках будут их новые значения.
        let refreshed = parts.contains(.all) ? awaitingRefresh : []
        Task.detached(priority: .userInitiated) {
            var loaded = Loaded(parts: parts, stamps: stamps, period: period, refreshed: refreshed)
            if parts.contains(.dates) {
                loaded.counts = Core.dateCounts(filter)
                loaded.total = Core.count()
                if loaded.counts == nil {
                    loaded.failed.insert(.dates)
                }
            }
            // Сравнение с показанным — здесь же, в фоне, и силами ядра: совпавший список
            // остаётся прежним объектом, и массив фото для него не собирается.
            if parts.contains(.all) {
                if let list = Core.geotagged(nil) {
                    loaded.all = list.same(as: shown.all) ? shown.all : list
                } else {
                    loaded.failed.insert(.all)
                }
            }
            if parts.contains(.period), let period {
                if let list = Core.geotagged(period) {
                    loaded.geo = list.same(as: shown.geo) ? shown.geo : list
                } else {
                    loaded.failed.insert(.period)
                }
            }
            // Список, который покажет карта, собрать заранее — не в главном потоке.
            _ = (period == nil ? loaded.all : loaded.geo)?.photos
            let result = loaded
            await MainActor.run {
                self.apply(result, keepStatus)
            }
        }
    }

    private struct Loaded: Sendable {
        let parts: ReloadParts
        /// Номера запросов по частям (`ReloadParts.rawValue`).
        let stamps: [Int: Int]
        let period: DatePeriod?
        let refreshed: Set<Photo.ID>
        var counts: [DateCount]?
        var total: Int?
        var all: PhotoList?
        var geo: PhotoList?
        /// Части, которые прочитать не удалось (БД занята): прежние данные остаются.
        var failed: ReloadParts = []
    }

    /// Даты фото в видимой области карты — пересчитываются при обновлении карты, пока открыта
    /// панель фильтра. Выбранный период остаётся, даже если его фото ушли из видимой области.
    func setMapDateCounts(_ counts: [DateCount]) {
        guard counts != mapDateCounts else { return }
        mapDateCounts = counts
        mapDates = DateNode.tree(counts)
    }

    private func apply(_ loaded: Loaded, _ keepStatus: Bool) {
        // Чтения идут параллельно и могут ждать сканер: результат более раннего, пришедший
        // после более позднего, затёр бы свежие данные (например, пустой БД до первого скана).
        // Поэтому каждая часть применяется, только если она от последнего запроса этой части.
        let current = { (part: ReloadParts) in
            loaded.parts.contains(part) && loaded.stamps[part.rawValue] == self.generations[part.rawValue]
        }
        var changed = false
        var retry: ReloadParts = []
        if current(.dates) {
            if let counts = loaded.counts {
                if forceDateReload {
                    forceDateReload = false
                    changed = true
                }
                if counts != dateCounts {
                    dateCounts = counts
                    dates = DateNode.tree(counts)
                    changed = true
                }
            } else {
                retry.insert(.dates)
            }
            if let total = loaded.total, !scanning, !keepStatus {
                status = Core.photos(total)
            }
        }
        if current(.all) {
            if let all = loaded.all {
                awaitingRefresh.subtract(loaded.refreshed)
                if all !== allGeotagged {
                    allGeotagged = all
                    // У фото могли измениться координаты — они есть и в списке вкладки дат.
                    changed = true
                }
                if mapPeriod == nil, geotagged !== allGeotagged {
                    geotagged = allGeotagged
                }
            } else {
                retry.insert(.all)
            }
        }
        if current(.period), loaded.period != nil, loaded.period == mapPeriod {
            if let geo = loaded.geo {
                if geo !== geotagged {
                    geotagged = geo
                }
            } else {
                retry.insert(.period)
            }
        }
        if changed {
            revision += 1
        }
        guard !retry.isEmpty else {
            reloadFailures = 0
            return
        }
        // БД занята. Во время сканирования её и так перечитывает таймер; иначе — повторить
        // чуть позже, но не бесконечно.
        reloadFailures += 1
        if reloadFailures > 5 {
            awaitingRefresh = []
        } else if !scanning {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    // Части, которые с тех пор запросили заново, повторять не нужно.
                    var parts: ReloadParts = []
                    for part in ReloadParts.each where retry.contains(part)
                        && loaded.stamps[part.rawValue] == self.generations[part.rawValue] {
                        parts.insert(part)
                    }
                    if !parts.isEmpty {
                        self.reload(parts, keepStatus: true)
                    }
                }
            }
        }
    }

    // MARK: Действия с фото

    /// Просмотр: программа из настроек, иначе — программа по умолчанию для типа файла.
    /// Видео — всегда в программе по умолчанию: просмотрщик из настроек — для фото.
    func open(_ photo: Photo) {
        if photo.isVideo || !Apps.open(photo.url, with: viewerID) {
            NSWorkspace.shared.open(photo.url)
        }
    }

    /// Редактирование: редактор из настроек, иначе найденный графический редактор,
    /// иначе — программа по умолчанию.
    func edit(_ photo: Photo) {
        if photo.isVideo {
            NSWorkspace.shared.open(photo.url)
        } else if !Apps.open(photo.url, with: editorID), !Apps.open(photo.url, with: Apps.findEditor(for: photo.url)) {
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
        // Фото вне периода фильтра карты: на карте его нет, и переход привёл бы на пустое
        // место — фильтр сбрасывается.
        if mapPeriod != nil, !geotagged.photos.contains(where: { $0.id == photo.id }) {
            mapPeriod = nil
        }
        map?.show(photo)
    }

    /// Есть ли среди фото те, чьи файлы сейчас изменяются; если да — сообщить об этом.
    private func refuseBusy(_ photos: [Photo]) -> Bool {
        let busy = photos.contains {
            writing.contains($0.id) || trashing.contains($0.id) || awaitingRefresh.contains($0.id)
        }
        if busy {
            alert = AlertMessage(title: Core.text("changes_in_progress"), detail: "")
        }
        return busy
    }

    /// Выполнить в главном потоке; вызовы из одного потока приходят в том же порядке.
    private nonisolated func onMain(_ work: @escaping @MainActor () -> Void) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated(work)
        }
    }

    /// Удалить фото — после подтверждения в окне (`pendingTrash`).
    func trash(_ photos: [Photo]) {
        if !photos.isEmpty, !refuseBusy(photos) {
            pendingTrash = photos
        }
    }

    /// Вопрос перед удалением `photos`.
    static func trashQuestion(_ photos: [Photo]) -> String {
        Core.confirmTrash(photos.count == 1 ? photos[0].fileName : Core.photos(photos.count))
    }

    /// Переместить файлы в корзину и убрать фото из БД и интерфейса — в фоне (сетевой диск
    /// или сотни файлов заняли бы главный поток надолго). Пока идёт перемещение, в строке
    /// состояния — его ход; по окончании — имя фото (или их число), при ошибке — первая из них.
    /// Фото остаются в сетке и в выделении до конца, но новые действия над ними отклоняются.
    func confirmTrash(_ photos: [Photo]) {
        pendingTrash = nil
        guard !photos.isEmpty, !refuseBusy(photos) else { return }
        let ids = Set(photos.map(\.id))
        trashing.formUnion(ids)
        fileQueue.async { [self] in
            var moved: [Photo] = []
            var failure: String?
            let progress = FileProgress(total: photos.count) { done, total in
                self.onMain { self.fileStatus = Core.trashing(done, total) }
            }
            for photo in photos {
                do {
                    try FileManager.default.trashItem(at: photo.url, resultingItemURL: nil)
                    Core.remove(id: photo.id)
                    moved.append(photo)
                } catch {
                    failure = failure ?? Core.trashFailed(photo.fileName, error.localizedDescription)
                }
                progress.step()
            }
            onMain { [moved, failure] in
                self.trashed(ids, moved: moved, failure: failure)
            }
        }
    }

    private func trashed(_ ids: Set<Photo.ID>, moved: [Photo], failure: String?) {
        trashing.subtract(ids)
        fileStatus = nil
        let gone = Set(moved.map(\.id))
        mapSelection.remove(gone)
        dateSelection.remove(gone)
        if let failure {
            status = failure
        } else if !moved.isEmpty {
            status = moved.count == 1 ? Core.trashed(moved[0].fileName) : Core.trashed(Core.photos(moved.count))
        }
        if !moved.isEmpty {
            reload(keepStatus: true)
        }
    }

    /// Открыть редактор EXIF для фото; видео из списка отбрасываются.
    func editExif(_ photos: [Photo]) {
        let photos = photos.filter { !$0.isVideo }
        guard !photos.isEmpty, !refuseBusy(photos) else { return }
        // Координаты — из списка всех фото с координатами, а не из выделения: оно могло
        // остаться с прежними значениями, и редактор перезаписал бы только что записанные
        // координаты без вопроса.
        let ids = Set(photos.map(\.id))
        var located: [Photo.ID: Photo] = [:]
        for photo in allGeotagged.photos where ids.contains(photo.id) {
            located[photo.id] = photo
        }
        let fresh = photos.map { located[$0.id] ?? ($0.hasCoords ? $0.withoutCoords : $0) }
        exifEdit = ExifEdit(photos: fresh, view: map?.currentView)
    }

    /// Записать изменения в файлы и в БД, затем перечитать БД. Записи идут в фоне по очереди
    /// (одна очередь с корзиной): две записи одного файла одновременно невозможны, а фото,
    /// которые ещё пишутся, для новой правки и удаления закрыты. В строке состояния — ход
    /// записи, затем имя фото (или их число), при ошибке — первая из них и окно с ней.
    func saveEdit(_ changes: ExifChanges, photos: [Photo]) {
        guard !photos.isEmpty, !refuseBusy(photos) else { return }
        let ids = Set(photos.map(\.id))
        writing.formUnion(ids)
        fileQueue.async { [self] in
            var saved: [Photo] = []
            var failure: String?
            let progress = FileProgress(total: photos.count) { done, total in
                self.onMain { self.fileStatus = Core.savingProgress(done, total) }
            }
            for photo in photos {
                if let error = Core.saveEdit(photo, changes) {
                    FileHandle.standardError.write(Data("EXIF \(photo.path): \(error)\n".utf8))
                    failure = failure ?? Core.changesFailed(photo.fileName, error)
                } else {
                    saved.append(photo)
                }
                progress.step()
            }
            onMain { [saved, failure] in
                self.editSaved(ids, saved: saved, failure: failure)
            }
        }
    }

    private func editSaved(_ ids: Set<Photo.ID>, saved: [Photo], failure: String?) {
        writing.subtract(ids)
        fileStatus = nil
        status = failure ?? Core.changesSaved(saved.count == 1 ? saved[0].fileName : Core.photos(saved.count))
        if let failure {
            alert = AlertMessage(title: failure, detail: "")
        }
        guard !saved.isEmpty else { return }
        // До перечитывания БД в списках — прежние значения этих фото.
        awaitingRefresh.formUnion(saved.map(\.id))
        // Миниатюры перезаписаны (могла смениться ориентация), данные панели «Информация» устарели.
        filesChanged(saved.compactMap(\.thumbPath))
        // Маркеры с новыми координатами карта пересоздаст сама: она сверяет место маркера.
        reload(keepStatus: true, force: true)
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

/// Ход работы с несколькими файлами для строки состояния: сообщает не чаще десяти раз в
/// секунду (и о последнем файле), для одного файла молчит.
private final class FileProgress {
    private let total: Int
    private let report: (Int, Int) -> Void
    private var done = 0
    private var reported = Date.distantPast

    init(total: Int, report: @escaping (Int, Int) -> Void) {
        self.total = total
        self.report = report
        if total > 1 {
            report(0, total)
        }
    }

    func step() {
        done += 1
        guard total > 1, done == total || Date().timeIntervalSince(reported) >= 0.1 else { return }
        reported = Date()
        report(done, total)
    }
}

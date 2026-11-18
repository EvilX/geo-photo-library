// Вкладка «Карта»: MapKit с маркерами-кластерами, строка поиска местности,
// «Моё местоположение» и список фото в видимой области.
import AppKit
import CoreLocation
import MapKit
import SwiftUI

/// Сторона миниатюры на маркере и ячейка кластеризации, в пунктах (как в версии GTK).
private let markerSize: CGFloat = 56
private let clusterCell = 72.0
/// Пауза после движения карты, прежде чем пересчитывать маркеры и список.
private let debounce: TimeInterval = 0.15
/// Предел зума при подгонке под все фото и шаг приближения по щелчку на кластере.
private let fitMaxZoom = 17.0
private let clusterZoomStep = 2.0
/// Зум для найденной точки без границ, для фото из «Найти на карте», пределы для места и местоположения.
private let placeZoom = 15.0
private let photoZoom = 16.0
private let placeMaxZoom = 17.0
private let locationMaxZoom = 16.0
/// Зум Web Mercator, на котором пункт карты MapKit равен пикселю: мир MapKit — 256·2²⁰ пунктов.
private let mapPointZoom = 20.0
/// Метров в пикселе на экваторе при зуме 0 (Web Mercator, тайл 256).
private let equatorMetersPerPixel = 156_543.033_92

struct MapTab: View {
    @Environment(AppModel.self) private var model
    @State private var finder = PlaceFinder()
    /// Ширина списка фото справа от карты.
    @State private var listWidth = 460.0

    var body: some View {
        @Bindable var model = model
        SplitPane(side: .trailing, width: $listWidth, sideRange: 300...900, minMain: 380) {
            PhotoMap(photos: model.geotagged)
                // Фильтр по датам плавает над картой под строкой поиска: открытие
                // не сдвигает и не сжимает карту.
                .overlay(alignment: .topLeading) {
                    if model.mapFilterOpen {
                        MapDateFilter()
                            .frame(width: 240)
                            .clipShape(RoundedRectangle(cornerRadius: 10))
                            .shadow(color: .black.opacity(0.3), radius: 4, y: 1)
                            .padding(EdgeInsets(top: 48, leading: 10, bottom: 10, trailing: 0))
                            .transition(.move(edge: .leading).combined(with: .opacity))
                    }
                }
                .overlay(alignment: .topLeading) {
                    MapSearchBar(finder: finder).padding(10)
                }
                .clipped()
                .animation(.easeInOut(duration: 0.2), value: model.mapFilterOpen)
        } sidePane: {
            VStack(spacing: 0) {
                PaneHeader(title: model.mapInfo)
                Divider()
                PhotoGrid(photos: model.ordered(model.mapVisible), selection: $model.mapSelection) { photo in
                    // Выбор фото в списке — центрировать карту на нём.
                    model.map?.center(on: photo)
                }
                Divider()
                ThumbScaleBar()
            }
        }
        .onAppear { DebugHooks.finder = finder }
    }
}

// MARK: Фильтр по датам

/// Плавающая панель фильтра карты: «Всё время», под ним дерево год → месяц → день, как на вкладке дат.
/// В дереве — только даты фото в видимой области карты (`AppModel.mapDates`).
struct MapDateFilter: View {
    @Environment(AppModel.self) private var model
    private static let allID = "all"

    var body: some View {
        let selection = Binding<String?>(
            get: { model.mapPeriod?.id ?? Self.allID },
            set: { id in
                // Щелчок мимо строк снимает выделение — фильтр остаётся прежним.
                guard let id else { return }
                model.mapPeriod = DateNode.find(id, in: model.mapDates)?.period
            }
        )
        List(selection: selection) {
            row(Core.text("all_time"), model.mapDates.reduce(0) { $0 + $1.count })
                .tag(Self.allID)
            OutlineGroup(model.mapDates, children: \.children) { node in
                row(node.label, node.count)
            }
        }
        .listStyle(.sidebar)
        .scrollContentBackground(.hidden)
        // Материал над картой: фон боковой панели (`SidebarBackground`) просвечивает
        // то, что за окном, а не карту.
        .background(.regularMaterial)
    }

    private func row(_ label: String, _ count: UInt32) -> some View {
        HStack {
            Text(label)
            Spacer()
            Text(String(count)).foregroundStyle(.secondary).monospacedDigit()
        }
        .padding(.vertical, 3)
    }
}

// MARK: Поиск местности и местоположение

/// Найденное место.
struct Place: Identifiable {
    let id = UUID()
    let name: String
    let detail: String
    let lat: Double
    let lon: Double
    /// Радиус области места в метрах, если поиск его вернул.
    let radius: Double?
}

/// Поиск местности (геокодер Apple, нужен интернет) и текущее местоположение (CoreLocation).
@MainActor
@Observable
final class PlaceFinder: NSObject, CLLocationManagerDelegate {
    var query = ""
    /// Найденные места (несколько — список на выбор) или сообщение вместо них.
    var places: [Place] = []
    var message: String?
    var showResults = false
    private(set) var locating = false

    /// Номер последнего запроса — ответы устаревших отбрасываются.
    private var generation = 0
    private let manager = CLLocationManager()

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
    }

    /// Запрос уходит только по Enter. Одно найденное место — сразу переход к нему.
    func search(_ model: AppModel) {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        generation += 1
        let current = generation
        let request = MKLocalSearch.Request()
        request.naturalLanguageQuery = text
        request.resultTypes = [.address, .pointOfInterest]
        Task {
            let result: Result<[Place], Error>
            do {
                let response = try await MKLocalSearch(request: request).start()
                result = .success(response.mapItems.map(Self.place))
            } catch {
                result = .failure(error)
            }
            guard current == generation else { return }
            switch result {
            case .success(let found) where found.count == 1:
                showResults = false
                go(to: found[0], model)
            case .success(let found):
                show(found, found.isEmpty ? Core.text("place_not_found") : nil)
            case .failure(let error as MKError) where error.code == .placemarkNotFound:
                show([], Core.text("place_not_found"))
            case .failure(let error):
                FileHandle.standardError.write(Data("Поиск местности: \(error)\n".utf8))
                show([], Core.text("place_search_failed"))
            }
        }
    }

    private func show(_ found: [Place], _ text: String?) {
        places = found
        message = text
        showResults = true
    }

    private static func place(_ item: MKMapItem) -> Place {
        let mark = item.placemark
        let name = item.name ?? mark.title ?? ""
        let detail = mark.title ?? ""
        return Place(
            name: name,
            detail: detail == name ? "" : detail,
            lat: mark.coordinate.latitude,
            lon: mark.coordinate.longitude,
            radius: (mark.region as? CLCircularRegion)?.radius
        )
    }

    /// Показать место целиком (по его области) или точку крупным планом.
    func go(to place: Place, _ model: AppModel) {
        showResults = false
        model.map?.go(lat: place.lat, lon: place.lon, radius: place.radius, maxZoom: placeMaxZoom)
    }

    // MARK: Моё местоположение

    private weak var model: AppModel?

    /// Разовый запрос местоположения: при первом система спросит разрешение.
    func locate(_ model: AppModel) {
        guard !locating else { return }
        self.model = model
        locating = true
        switch manager.authorizationStatus {
        case .notDetermined: manager.requestWhenInUseAuthorization()
        case .denied, .restricted: fail(denied: true)
        default: manager.requestLocation()
        }
    }

    private func fail(denied: Bool) {
        locating = false
        model?.alert = AlertMessage(
            title: Core.text("location_failed"),
            detail: Core.text(denied ? "location_denied" : "location_unavailable")
        )
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        MainActor.assumeIsolated {
            // Срабатывает и при запуске — интересен только ответ на наш запрос.
            guard locating else { return }
            switch manager.authorizationStatus {
            case .notDetermined: break
            case .denied, .restricted: fail(denied: true)
            default: manager.requestLocation()
            }
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        MainActor.assumeIsolated {
            guard locating, let fix = locations.last else { return }
            locating = false
            let (lat, lon) = (fix.coordinate.latitude, fix.coordinate.longitude)
            model?.map?.markLocation(lat: lat, lon: lon)
            // Круг точности целиком в кадре.
            model?.map?.go(lat: lat, lon: lon, radius: max(fix.horizontalAccuracy, 50), maxZoom: locationMaxZoom)
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        MainActor.assumeIsolated {
            guard locating else { return }
            FileHandle.standardError.write(Data("Местоположение: \(error)\n".utf8))
            fail(denied: (error as? CLError)?.code == .denied)
        }
    }
}

/// Строка поиска над картой и кнопка «Моё местоположение».
struct MapSearchBar: View {
    @Bindable var finder: PlaceFinder
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            // Фильтр по датам: открывает панель слева; выбранный период виден на кнопке.
            Button {
                model.mapFilterOpen.toggle()
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "calendar")
                    if let period = model.mapPeriod {
                        Text(Core.dateTitle(year: period.year, month: period.month, day: period.day))
                            .lineLimit(1)
                    }
                }
                .foregroundStyle(model.mapPeriod == nil ? Color.primary : Color.accentColor)
                .padding(.horizontal, 7)
                .frame(minWidth: 28, minHeight: 28)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 7))
            .shadow(color: .black.opacity(0.3), radius: 2, y: 1)
            .help(Core.text("filter_by_date"))

            HStack(spacing: 5) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                // Подсказка своя: системная на подложке-материале рисуется «живой»
                // (vibrancy) и выглядит размытой. Цвет из NSColor от фона не зависит.
                TextField("", text: $finder.query)
                    .textFieldStyle(.plain)
                    .accessibilityLabel(Core.text("search_place"))
                    .background(alignment: .leading) {
                        if finder.query.isEmpty {
                            Text(Core.text("search_place"))
                                .foregroundStyle(Color(nsColor: .placeholderTextColor))
                                .lineLimit(1)
                                .allowsHitTesting(false)
                        }
                    }
                    .onSubmit { finder.search(model) }
            }
            .padding(.horizontal, 8)
            .frame(width: 260, height: 28)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 7))
            .shadow(color: .black.opacity(0.3), radius: 2, y: 1)
            .popover(isPresented: $finder.showResults, arrowEdge: .bottom) {
                results
            }

            Button {
                finder.locate(model)
            } label: {
                if finder.locating {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "location")
                }
            }
            .buttonStyle(.plain)
            .frame(width: 28, height: 28)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 7))
            .shadow(color: .black.opacity(0.3), radius: 2, y: 1)
            .disabled(finder.locating)
            .help(Core.text("my_location"))
        }
    }

    @ViewBuilder
    private var results: some View {
        if let message = finder.message {
            Text(message).foregroundStyle(.secondary).padding(14)
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(finder.places) { place in
                        Button {
                            finder.go(to: place, model)
                        } label: {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(place.name)
                                if !place.detail.isEmpty {
                                    Text(place.detail).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.vertical, 5)
            }
            .frame(width: 340)
            .frame(maxHeight: 320)
        }
    }
}

// MARK: Карта

final class ClusterAnnotation: NSObject, MKAnnotation {
    let cluster: Cluster
    let coordinate: CLLocationCoordinate2D

    init(_ cluster: Cluster) {
        self.cluster = cluster
        coordinate = CLLocationCoordinate2D(latitude: cluster.lat, longitude: cluster.lon)
    }
}

/// Точка «Моё местоположение».
final class LocationAnnotation: NSObject, MKAnnotation {
    let coordinate: CLLocationCoordinate2D

    init(lat: Double, lon: Double) {
        coordinate = CLLocationCoordinate2D(latitude: lat, longitude: lon)
    }
}

/// Маркер: миниатюра первого фото кластера в белой рамке и красный счётчик.
final class ClusterView: MKAnnotationView {
    static let reuseID = "cluster"

    private let thumb = NSImageView()
    private let badge = NSTextField(labelWithString: "")
    private var path: String?

    override init(annotation: MKAnnotation?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        frame = NSRect(x: 0, y: 0, width: markerSize, height: markerSize)
        wantsLayer = true
        shadow = NSShadow()
        layer?.shadowOpacity = 0.55
        layer?.shadowRadius = 2.5
        layer?.shadowOffset = CGSize(width: 0, height: -1)

        thumb.frame = bounds
        thumb.imageScaling = .scaleAxesIndependently
        thumb.wantsLayer = true
        thumb.layer?.cornerRadius = 8
        thumb.layer?.masksToBounds = true
        thumb.layer?.borderWidth = 2
        thumb.layer?.borderColor = NSColor.white.cgColor
        thumb.layer?.backgroundColor = NSColor.gray.cgColor
        addSubview(thumb)

        badge.font = .boldSystemFont(ofSize: 11)
        badge.textColor = .white
        badge.alignment = .center
        badge.wantsLayer = true
        badge.layer?.backgroundColor = NSColor.systemRed.cgColor
        badge.layer?.masksToBounds = true
        addSubview(badge)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) не используется")
    }

    func show(_ cluster: Cluster) {
        badge.isHidden = cluster.count < 2
        badge.stringValue = String(cluster.count)
        let text = badge.intrinsicContentSize
        let (w, h) = (max(text.width + 8, 18), CGFloat(18))
        badge.frame = NSRect(x: markerSize - w + 6, y: markerSize - h + 6, width: w, height: h)
        badge.layer?.cornerRadius = h / 2

        path = cluster.cover.thumbPath
        thumb.image = path.flatMap(ThumbnailCache.shared.cached)
        guard thumb.image == nil, let path else { return }
        Task { @MainActor [weak self] in
            let image = await ThumbnailCache.shared.image(path)
            // Маркер переиспользуется: ставим картинку, только если он показывает то же фото.
            if self?.path == path {
                self?.thumb.image = image
            }
        }
    }
}

/// Синяя точка в белой рамке с ореолом.
final class LocationView: MKAnnotationView {
    static let reuseID = "location"
    private static let size: CGFloat = 20
    private static let halo: CGFloat = 6

    override init(annotation: MKAnnotation?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        let full = Self.size + 2 * Self.halo
        frame = NSRect(x: 0, y: 0, width: full, height: full)
        wantsLayer = true
        layer?.cornerRadius = full / 2
        layer?.backgroundColor = NSColor.systemBlue.withAlphaComponent(0.25).cgColor
        let dot = CALayer()
        dot.frame = bounds.insetBy(dx: Self.halo, dy: Self.halo)
        dot.cornerRadius = Self.size / 2
        dot.backgroundColor = NSColor.systemBlue.cgColor
        dot.borderColor = NSColor.white.cgColor
        dot.borderWidth = 3
        layer?.addSublayer(dot)
        toolTip = Core.text("my_location")
        // Точка — под маркерами фото.
        zPriority = .min
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) не используется")
    }
}

struct PhotoMap: NSViewRepresentable {
    let photos: PhotoList
    @Environment(AppModel.self) private var model

    func makeCoordinator() -> Coordinator {
        Coordinator(model)
    }

    func makeNSView(context: Context) -> MKMapView {
        let map = SizedMapView()
        map.delegate = context.coordinator
        // Зум считается по ширине видимой области — карта должна оставаться плоской и на север.
        map.isPitchEnabled = false
        map.isRotateEnabled = false
        map.preferredConfiguration = MKStandardMapConfiguration(elevationStyle: .flat)
        map.showsZoomControls = true
        map.register(ClusterView.self, forAnnotationViewWithReuseIdentifier: ClusterView.reuseID)
        map.register(LocationView.self, forAnnotationViewWithReuseIdentifier: LocationView.reuseID)
        map.onResize = { [weak coordinator = context.coordinator] in coordinator?.resized() }
        // Щелчок по карте закрывает панель фильтра по датам; перетаскивание — нет
        // (распознаватель щелчка не срабатывает при сдвиге). События карте не мешает.
        let click = NSClickGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.mapClicked))
        click.delaysPrimaryMouseButtonEvents = false
        map.addGestureRecognizer(click)
        context.coordinator.map = map
        model.map = context.coordinator
        return map
    }

    func updateNSView(_ map: MKMapView, context: Context) {
        context.coordinator.set(photos)
    }

    static func dismantleNSView(_ map: MKMapView, coordinator: Coordinator) {
        coordinator.saveView()
    }

    /// Карта, сообщающая о смене размера: первая раскладка окна и перетаскивание разделителя.
    final class SizedMapView: MKMapView {
        var onResize: (() -> Void)?

        override func setFrameSize(_ newSize: NSSize) {
            let changed = newSize != frame.size
            super.setFrameSize(newSize)
            if changed {
                onResize?()
            }
        }
    }

    @MainActor
    final class Coordinator: NSObject, MKMapViewDelegate, MapController {
        private let model: AppModel
        weak var map: MKMapView?
        private var photos = PhotoList.empty
        private var markers: [Cluster.Key: ClusterAnnotation] = [:]
        private var visible: [Int]?
        private var pending: Timer?
        /// Начальное положение (сохранённое или подгонка под все фото) уже выставлено.
        private var positioned = false
        /// Фото из «Найти на карте» — выделить, как только оно окажется в видимой области.
        private var pendingSelect: Int64?
        private var location: LocationAnnotation?
        private var quitObserver: NSObjectProtocol?

        init(_ model: AppModel) {
            self.model = model
            super.init()
            quitObserver = NotificationCenter.default.addObserver(
                forName: NSApplication.willTerminateNotification, object: nil, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.saveView() }
            }
        }

        deinit {
            if let quitObserver {
                NotificationCenter.default.removeObserver(quitObserver)
            }
        }

        func set(_ list: PhotoList) {
            guard list !== photos else { return }
            photos = list
            // Индексы относятся к прежнему списку — список фото собрать заново.
            visible = nil
            position()
            refresh()
        }

        func resized() {
            position()
            schedule()
        }

        // MARK: Положение

        private var hasSize: Bool {
            guard let map else { return false }
            return map.bounds.width >= 2 && map.bounds.height >= 2
        }

        /// Зум Web Mercator (тайл 256) по ширине видимой области.
        private var zoom: Double {
            guard let map, hasSize else { return 0 }
            return mapPointZoom + log2(map.bounds.width / map.visibleMapRect.size.width)
        }

        private func move(lat: Double, lon: Double, zoom: Double, animated: Bool) {
            guard let map, hasSize else { return }
            let scale = pow(2, mapPointZoom - zoom)
            let (w, h) = (map.bounds.width * scale, map.bounds.height * scale)
            let center = MKMapPoint(CLLocationCoordinate2D(latitude: lat, longitude: lon))
            let rect = MKMapRect(x: center.x - w / 2, y: center.y - h / 2, width: w, height: h)
            map.setVisibleMapRect(rect, animated: animated)
        }

        /// Один раз, когда у карты появился размер: сохранённое положение, иначе все фото.
        private func position() {
            guard !positioned, let map, hasSize else { return }
            if let saved = Core.mapView {
                move(lat: saved.lat, lon: saved.lon, zoom: saved.zoom, animated: false)
            } else if let fit = photos.fit(size: map.bounds.size, pad: 60, maxZoom: fitMaxZoom) {
                move(lat: fit.lat, lon: fit.lon, zoom: fit.zoom, animated: false)
            } else {
                // Фото ещё нет (идёт первое сканирование) — подгоним, когда появятся.
                return
            }
            positioned = true
        }

        /// Пока карта не показана, положение не сохраняется — иначе запомнилась бы нулевая точка.
        func saveView() {
            guard positioned, let map, hasSize else { return }
            let center = map.centerCoordinate
            Core.setMapView(lat: center.latitude, lon: center.longitude, zoom: zoom)
        }

        // MARK: MapController

        func center(on photo: Photo) {
            guard photo.hasCoords, let map else { return }
            map.setCenter(CLLocationCoordinate2D(latitude: photo.lat, longitude: photo.lon), animated: true)
        }

        func show(_ photo: Photo) {
            guard photo.hasCoords else { return }
            // Сохранённое положение или автоподгонка больше не нужны.
            positioned = true
            pendingSelect = photo.id
            move(lat: photo.lat, lon: photo.lon, zoom: photoZoom, animated: true)
            // Если карта уже здесь, область не изменится — обновить список явно.
            schedule()
        }

        func go(lat: Double, lon: Double, radius: Double?, maxZoom: Double) {
            guard let map, hasSize else { return }
            positioned = true
            var target = placeZoom
            if let radius, radius > 0 {
                // Круг радиуса `radius` целиком в кадре с отступом; целый зум, как в версии GTK.
                let side = max(min(map.bounds.width, map.bounds.height) - 80, 1)
                let metersPerPixel = 2 * radius / side
                target = log2(equatorMetersPerPixel * cos(lat * .pi / 180) / metersPerPixel).rounded(.down)
            }
            move(lat: lat, lon: lon, zoom: min(max(target, 3), maxZoom), animated: true)
        }

        func markLocation(lat: Double, lon: Double) {
            guard let map else { return }
            if let location {
                map.removeAnnotation(location)
            }
            let mark = LocationAnnotation(lat: lat, lon: lon)
            location = mark
            map.addAnnotation(mark)
        }

        // MARK: Маркеры и список

        private func schedule() {
            pending?.invalidate()
            pending = Timer.scheduledTimer(withTimeInterval: debounce, repeats: false) { [weak self] _ in
                MainActor.assumeIsolated { self?.refresh() }
            }
        }

        private func refresh() {
            guard let map, hasSize else { return }
            let center = map.centerCoordinate
            let snapshot = photos.snapshot(
                lat: center.latitude, lon: center.longitude, zoom: zoom, size: map.bounds.size, cell: clusterCell
            )

            // Пересоздаются только изменившиеся кластеры.
            let keep = Set(snapshot.clusters.map(\.key))
            let gone = markers.filter { !keep.contains($0.key) }
            map.removeAnnotations(Array(gone.values))
            gone.keys.forEach { markers[$0] = nil }
            let added = snapshot.clusters.filter { markers[$0.key] == nil }.map(ClusterAnnotation.init)
            added.forEach { markers[$0.cluster.key] = $0 }
            map.addAnnotations(added)

            model.setMapDateCounts(model.allGeotagged.visibleDateCounts(
                lat: center.latitude, lon: center.longitude, zoom: zoom, size: map.bounds.size
            ))
            let total = photos.photos.count
            model.mapInfo = total == 0 ? Core.text("no_geotagged") : Core.inMapArea(snapshot.visible.count, total)
            if snapshot.visible != visible {
                visible = snapshot.visible
                model.mapVisible = snapshot.visible.map { photos.photos[$0] }
                // Выбранное фото ушло из видимой области (или удалено) — выделения больше нет.
                if let selected = model.mapSelection, !model.mapVisible.contains(where: { $0.id == selected.id }) {
                    model.mapSelection = nil
                }
            }
            if let id = pendingSelect, let photo = model.mapVisible.first(where: { $0.id == id }) {
                model.mapSelection = photo
                pendingSelect = nil
            }
        }

        @objc func mapClicked() {
            model.mapFilterOpen = false
        }

        // MARK: MKMapViewDelegate

        func mapViewDidChangeVisibleRegion(_ mapView: MKMapView) {
            schedule()
        }

        func mapView(_ mapView: MKMapView, viewFor annotation: MKAnnotation) -> MKAnnotationView? {
            if let annotation = annotation as? ClusterAnnotation {
                let view = mapView.dequeueReusableAnnotationView(withIdentifier: ClusterView.reuseID, for: annotation)
                (view as? ClusterView)?.show(annotation.cluster)
                return view
            }
            if annotation is LocationAnnotation {
                return mapView.dequeueReusableAnnotationView(withIdentifier: LocationView.reuseID, for: annotation)
            }
            return nil
        }

        /// Щелчок по маркеру: одно фото — выделить в списке, кластер — приблизить.
        func mapView(_ mapView: MKMapView, didSelect view: MKAnnotationView) {
            guard let annotation = view.annotation as? ClusterAnnotation else { return }
            mapView.deselectAnnotation(annotation, animated: false)
            let cluster = annotation.cluster
            if cluster.count == 1 {
                model.mapSelection = cluster.cover
            } else {
                move(lat: cluster.lat, lon: cluster.lon, zoom: zoom + clusterZoomStep, animated: true)
            }
        }
    }
}

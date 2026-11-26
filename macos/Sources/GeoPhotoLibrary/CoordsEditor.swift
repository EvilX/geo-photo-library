// Редактор координат фото (лист над окном): метка всегда в центре карты, точку выбирают,
// двигая карту под ней (перетаскивание, щелчок, поиск местности, «Моё местоположение»),
// или вводят в поля широты и долготы.
import AppKit
import MapKit
import SwiftUI

/// Зум, с которым показывается точка фото.
private let pointZoom = 15.0
/// Вид без точки и без положения главной карты.
private let world = (lat: 30.0, lon: 0.0, zoom: 3.0)

struct CoordsEditor: View {
    let edit: CoordsEdit
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var finder = PlaceFinder()
    @State private var map = PointMapController()
    @State private var lat = ""
    @State private var lon = ""
    /// Текст полей, записанный программой (щелчок, поиск): его изменение — не ввод.
    @State private var filled = ("", "")
    @State private var confirming = false

    var body: some View {
        VStack(spacing: 10) {
            HStack(spacing: 6) {
                PlaceSearchField(finder: finder)
                    .frame(height: 28)
                    .background(RoundedRectangle(cornerRadius: 7).fill(Color(nsColor: .textBackgroundColor)))
                    .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(Color(nsColor: .separatorColor)))
                LocateButton(finder: finder)
                    .frame(width: 28, height: 28)
                    .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(Color(nsColor: .separatorColor)))
            }
            PointMap(controller: map, start: start) { centerMoved($0, $1) }
                // Метка — красная капля (отличается от синей отметки «я здесь»); остриё —
                // в центре карты, щелчки проходят к карте.
                .overlay {
                    ZStack(alignment: .top) {
                        Drop().fill(Color.red)
                        Drop().stroke(.white, lineWidth: 2)
                        Circle().fill(.white).frame(width: 10, height: 10).padding(.top, 10)
                    }
                    .frame(width: 30, height: 42)
                    .shadow(color: .black.opacity(0.4), radius: 1.5, y: 1)
                    .offset(y: -21)
                    .allowsHitTesting(false)
                }
                .clipShape(RoundedRectangle(cornerRadius: 8))
            Text(Core.text("coordinates_hint"))
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 8) {
                Text(Core.text("latitude"))
                TextField("", text: $lat).frame(width: 110)
                Text(Core.text("longitude"))
                TextField("", text: $lon).frame(width: 110)
                Spacer()
                Button(Core.text("cancel"), role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(Core.saveCount(edit.photos.count)) { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(value == nil)
            }
        }
        .padding(14)
        .frame(minWidth: 640, idealWidth: 760, minHeight: 480, idealHeight: 600)
        .onAppear {
            finder.target = { [map] in map }
            // Поля — координаты единственного фото; без них пустые, пока карту не сдвинут.
            if let photo = single, photo.hasCoords {
                fill(photo.lat, photo.lon)
            }
        }
        .onChange(of: lat) { fieldsChanged(lat) }
        .onChange(of: lon) { fieldsChanged(lon) }
        // Перезапись координат одного фото и сохранение для нескольких — с подтверждением.
        .alert(confirmTitle, isPresented: $confirming) {
            Button(Core.text("cancel"), role: .cancel) {}
            Button(edit.photos.count > 1 ? Core.text("save") : Core.text("update")) { commit() }
                .keyboardShortcut(.defaultAction)
        } message: {
            Text(confirmDetail)
        }
        .alert(
            finder.alert?.title ?? "",
            isPresented: Binding(get: { finder.alert != nil }, set: { if !$0 { finder.alert = nil } }),
            presenting: finder.alert
        ) { _ in
            Button(Core.text("ok")) {}
        } message: { alert in
            Text(alert.detail)
        }
    }

    /// Единственное редактируемое фото.
    private var single: Photo? {
        edit.photos.count == 1 ? edit.photos[0] : nil
    }

    private var start: (lat: Double, lon: Double, zoom: Double) {
        if let photo = single, photo.hasCoords {
            return (photo.lat, photo.lon, pointZoom)
        }
        return edit.view ?? world
    }

    /// Координаты из полей, если они допустимы.
    private var value: (lat: Double, lon: Double)? {
        guard let a = Core.parseCoord(lat), let b = Core.parseCoord(lon), Core.validCoords(lat: a, lon: b) else {
            return nil
        }
        return (a, b)
    }

    private var confirmTitle: String {
        edit.photos.count > 1 ? Core.confirmSaveCoordinates(edit.photos.count) : Core.text("confirm_update_coordinates")
    }

    private var confirmDetail: String {
        if let photo = single {
            return Core.confirmUpdateCoordinatesDetail(photo.fileName)
        }
        return Core.text("confirm_save_coordinates_detail")
    }

    /// Записать координаты в поля.
    private func fill(_ a: Double, _ b: Double) {
        filled = (Core.formatCoord(a), Core.formatCoord(b))
        (lat, lon) = filled
    }

    /// Центр карты сдвинулся: точка под меткой — в поля. Если центр уже совпадает с полями
    /// (карту сдвинул ввод), поля не трогать — иначе вводимый текст переформатировался бы.
    private func centerMoved(_ a: Double, _ b: Double) {
        if let v = value, abs(v.lat - a) < 1e-7, abs(v.lon - b) < 1e-7 {
            return
        }
        fill(a, b)
    }

    /// Ввод в поля двигает карту; пара «широта, долгота», вставленная в одно поле, разбирается.
    private func fieldsChanged(_ text: String) {
        guard (lat, lon) != filled else { return }
        if let pair = Core.parseCoordPair(text) {
            fill(pair.lat, pair.lon)
        }
        if let v = value {
            map.center(lat: v.lat, lon: v.lon)
        }
    }

    private func save() {
        guard value != nil else { return }
        if let photo = single, !photo.hasCoords {
            commit()
        } else {
            confirming = true
        }
    }

    private func commit() {
        guard let v = value else { return }
        model.saveCoords(lat: v.lat, lon: v.lon, photos: edit.photos)
        dismiss()
    }
}

/// Капля: круг сверху и остриё внизу посередине.
struct Drop: Shape {
    func path(in rect: CGRect) -> Path {
        let r = rect.width / 2 - 1
        let center = CGPoint(x: rect.midX, y: rect.minY + r + 1)
        let tip = CGPoint(x: rect.midX, y: rect.maxY - 1)
        // Касательные из острия к окружности: половина угла при острие.
        let phi = asin(r / (tip.y - center.y))
        var path = Path()
        path.move(to: tip)
        path.addArc(center: center, radius: r, startAngle: .radians(.pi - phi), endAngle: .radians(2 * .pi + phi), clockwise: false)
        path.closeSubpath()
        return path
    }
}

/// Команды карте редактора: центр, переход к месту, отметка «я здесь».
@MainActor
final class PointMapController: PlaceTarget {
    weak var map: MKMapView?
    private var location: LocationAnnotation?

    /// Сразу, без анимации: центр должен совпасть с введённым в поля.
    func center(lat: Double, lon: Double) {
        map?.setCenter(CLLocationCoordinate2D(latitude: lat, longitude: lon), animated: false)
    }

    func go(lat: Double, lon: Double, radius: Double?, maxZoom: Double) {
        map?.go(lat: lat, lon: lon, radius: radius, maxZoom: maxZoom)
    }

    func markLocation(lat: Double, lon: Double) {
        location = map?.mark(location: location, lat: lat, lon: lon)
    }
}

/// Карта редактора: о каждом сдвиге центра сообщает `onCenter`; щелчок переносит
/// точку щелчка в центр, перетаскивание просто двигает карту.
struct PointMap: NSViewRepresentable {
    let controller: PointMapController
    let start: (lat: Double, lon: Double, zoom: Double)
    let onCenter: (Double, Double) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(start: start, onCenter: onCenter)
    }

    func makeNSView(context: Context) -> MKMapView {
        let map = PhotoMap.SizedMapView()
        map.delegate = context.coordinator
        // Плоская карта на север, как на вкладке: зум считается по ширине видимой области.
        map.isPitchEnabled = false
        map.isRotateEnabled = false
        map.preferredConfiguration = MKStandardMapConfiguration(elevationStyle: .flat)
        map.showsZoomControls = true
        map.register(LocationView.self, forAnnotationViewWithReuseIdentifier: LocationView.reuseID)
        map.onResize = { [weak map, weak coordinator = context.coordinator] in
            if let map, let coordinator {
                coordinator.resized(map)
            }
        }
        // Распознаватель щелчка не срабатывает при сдвиге мыши — перетаскивание остаётся перетаскиванием.
        let click = NSClickGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.clicked(_:)))
        click.delaysPrimaryMouseButtonEvents = false
        map.addGestureRecognizer(click)
        controller.map = map
        return map
    }

    func updateNSView(_ map: MKMapView, context: Context) {}

    @MainActor
    final class Coordinator: NSObject, MKMapViewDelegate {
        let start: (lat: Double, lon: Double, zoom: Double)
        let onCenter: (Double, Double) -> Void
        private var positioned = false
        /// Точка под меткой. Сдвиг центра без её изменения (раскладка, смена размера листа)
        /// — не выбор точки: поля, пустые для фото без координат, не трогаются.
        private var anchor = CLLocationCoordinate2D()

        init(start: (lat: Double, lon: Double, zoom: Double), onCenter: @escaping (Double, Double) -> Void) {
            self.start = start
            self.onCenter = onCenter
        }

        /// Первый размер — начальное положение; дальше при смене размера точка под меткой
        /// остаётся в центре (MapKit сохраняет угол области, и центр бы уехал).
        func resized(_ map: MKMapView) {
            guard map.hasSize else { return }
            if !positioned {
                positioned = true
                anchor = CLLocationCoordinate2D(latitude: start.lat, longitude: start.lon)
                map.move(lat: start.lat, lon: start.lon, zoom: start.zoom, animated: false)
            }
            map.setCenter(anchor, animated: false)
        }

        @objc func clicked(_ recognizer: NSClickGestureRecognizer) {
            guard let map = recognizer.view as? MKMapView else { return }
            let c = map.convert(recognizer.location(in: map), toCoordinateFrom: map)
            map.setCenter(c, animated: true)
        }

        func mapViewDidChangeVisibleRegion(_ mapView: MKMapView) {
            guard positioned else { return }
            let c = mapView.centerCoordinate
            guard abs(c.latitude - anchor.latitude) > 1e-6 || abs(c.longitude - anchor.longitude) > 1e-6 else { return }
            anchor = c
            onCenter(c.latitude, c.longitude)
        }

        func mapView(_ mapView: MKMapView, viewFor annotation: MKAnnotation) -> MKAnnotationView? {
            if annotation is LocationAnnotation {
                return mapView.dequeueReusableAnnotationView(withIdentifier: LocationView.reuseID, for: annotation)
            }
            return nil
        }
    }
}

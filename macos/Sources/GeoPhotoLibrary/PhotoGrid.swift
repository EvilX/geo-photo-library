// Сетка миниатюр (общая для карты и дат), её заголовок и слайдер масштаба, кэш изображений.
import AppKit
import ImageIO
import SwiftUI

/// Миниатюры в памяти: до 128 МБ, загрузка и декодирование — вне главного потока.
final class ThumbnailCache: @unchecked Sendable {
    static let shared = ThumbnailCache()

    private let cache = NSCache<NSString, NSImage>()

    private init() {
        cache.totalCostLimit = 128 << 20
    }

    func cached(_ path: String) -> NSImage? {
        cache.object(forKey: path as NSString)
    }

    /// Убрать из кэша: файл миниатюры перезаписан (например, сменилась ориентация).
    func forget(_ path: String) {
        cache.removeObject(forKey: path as NSString)
    }

    func image(_ path: String) async -> NSImage? {
        if let image = cached(path) {
            return image
        }
        return await Task.detached(priority: .utility) { [cache] () -> NSImage? in
            let options = [kCGImageSourceShouldCacheImmediately: true] as CFDictionary
            guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
                  let cg = CGImageSourceCreateImageAtIndex(source, 0, options)
            else { return nil }
            let image = NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
            cache.setObject(image, forKey: path as NSString, cost: cg.width * cg.height * 4)
            return image
        }.value
    }
}

/// Квадратная миниатюра фото; пока она грузится (или её нет) — серая плашка.
struct Thumbnail: View {
    let path: String?
    @State private var image: NSImage?

    var body: some View {
        Color.gray.opacity(0.25)
            .aspectRatio(1, contentMode: .fit)
            .overlay {
                if let image {
                    Image(nsImage: image).resizable().scaledToFill()
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 6))
            // Ячейки переиспользуются: задача привязана к пути и отменяется при его смене.
            .task(id: path) {
                image = path.flatMap(ThumbnailCache.shared.cached)
                if image == nil, let path {
                    image = await ThumbnailCache.shared.image(path)
                }
            }
    }
}

/// Дата съёмки в формате системной локали, без секунд.
@MainActor
enum PhotoDate {
    private static let parser: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return f
    }()

    private static let display: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .short
        f.timeStyle = .short
        return f
    }()

    static func string(_ takenAt: String) -> String {
        parser.date(from: takenAt).map(display.string) ?? takenAt
    }
}

struct PhotoGrid: View {
    /// Сторона миниатюры при масштабе 100%.
    private static let baseSize = 150.0

    /// Фото уже в порядке показа.
    let photos: [Photo]
    @Binding var selection: PhotoSelection
    /// Пользователь выбрал фото обычным щелчком (без ⌘ и ⇧; а не программа выделила его сама).
    var onPick: (Photo) -> Void = { _ in }
    @Environment(AppModel.self) private var model

    /// Промежуток между ячейками и отступ от краёв.
    private static let spacing = 10.0
    private static let space = "photo-grid"

    /// Высота ячейки (миниатюра и две строки подписи) — по ней рамка находит фото.
    @State private var cellHeight = 0.0
    /// Рамка выделения мышью в координатах сетки.
    @State private var marquee: CGRect?
    /// Выделение до начала рамки: с ⌘ или ⇧ рамка добавляет к нему.
    @State private var marqueeBase: PhotoSelection?
    /// Фото, от которого ходят стрелки (с ⇧ — конец диапазона от опорного); `nil` — опорное.
    @State private var cursor: Photo.ID?
    @FocusState private var focused: Bool

    var body: some View {
        let size = Self.baseSize * model.thumbScale / 100
        // Колонки фиксированной ширины, их число — сколько влезает. С адаптивными колонками
        // (`.adaptive`) ячейки растягиваются на всю ширину, и размер менялся только
        // при смене числа колонок — слайдер масштаба работал скачками.
        GeometryReader { geometry in
            let available = geometry.size.width - 2 * Self.spacing
            let count = max(1, Int((available + Self.spacing) / (size + Self.spacing)))
            let cellWidth = min(size, available)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVGrid(
                        columns: Array(repeating: GridItem(.fixed(cellWidth), spacing: Self.spacing), count: count),
                        spacing: Self.spacing
                    ) {
                        ForEach(photos) { photo in
                            cell(photo)
                        }
                    }
                    .coordinateSpace(name: Self.space)
                    .overlay(alignment: .topLeading) {
                        if let marquee {
                            Rectangle()
                                .fill(Color.accentColor.opacity(0.15))
                                .stroke(Color.accentColor.opacity(0.7))
                                .frame(width: marquee.width, height: marquee.height)
                                .offset(x: marquee.minX, y: marquee.minY)
                                .allowsHitTesting(false)
                        }
                    }
                    .padding(Self.spacing)
                    // Пустое место под сеткой тоже ловит щелчок (снять выделение) и рамку.
                    .frame(maxWidth: .infinity, minHeight: geometry.size.height, alignment: .top)
                    .contentShape(Rectangle())
                    .onTapGesture {
                        focused = true
                        selection = PhotoSelection()
                    }
                    .gesture(marqueeGesture(columns: count, cellWidth: cellWidth))
                }
                // Управление с клавиатуры: стрелки, Page Up/Down, Home/End; с ⇧ — диапазон.
                .focusable()
                .focusEffectDisabled()
                .focused($focused)
                .onKeyPress(keys: [.upArrow, .downArrow, .leftArrow, .rightArrow, .pageUp, .pageDown, .home, .end],
                            phases: [.down, .repeat]) { press in
                    let rows = cellHeight > 0 ? Int((geometry.size.height - Self.spacing) / (cellHeight + Self.spacing)) : 1
                    guard let target = move(press.key, columns: count, pageRows: max(1, rows)) else {
                        return .ignored
                    }
                    select(target, extend: press.modifiers.contains(.shift))
                    proxy.scrollTo(target.id)
                    return .handled
                }
                .onChange(of: selection.anchor) { _, id in
                    if let id, marquee == nil {
                        withAnimation { proxy.scrollTo(id) }
                    }
                }
            }
        }
        .overlay {
            if photos.isEmpty {
                Text(Core.text("no_photos")).foregroundStyle(.secondary)
            }
        }
        .background(Color(nsColor: .controlBackgroundColor))
    }

    /// Рамка мышью: выделяет фото, ячейки которых она задевает. Ячейки не меряются по одной —
    /// LazyVGrid создаёт только видимые, — их места считаются по колонкам и высоте ячейки.
    private func marqueeGesture(columns: Int, cellWidth: Double) -> some Gesture {
        DragGesture(minimumDistance: 4, coordinateSpace: .named(Self.space))
            .onChanged { drag in
                if marqueeBase == nil {
                    let flags = NSEvent.modifierFlags
                    marqueeBase = flags.contains(.command) || flags.contains(.shift) ? selection : PhotoSelection()
                }
                let rect = CGRect(
                    x: min(drag.startLocation.x, drag.location.x), y: min(drag.startLocation.y, drag.location.y),
                    width: abs(drag.location.x - drag.startLocation.x), height: abs(drag.location.y - drag.startLocation.y)
                )
                marquee = rect
                var next = marqueeBase ?? PhotoSelection()
                next.formUnion(PhotoSelection(hits(rect, columns: columns, cellWidth: cellWidth)))
                if next != selection {
                    selection = next
                }
            }
            .onEnded { _ in
                marquee = nil
                marqueeBase = nil
                cursor = nil
            }
    }

    /// Фото, ячейки которых пересекает прямоугольник `rect` (в координатах сетки).
    private func hits(_ rect: CGRect, columns: Int, cellWidth: Double) -> [Photo] {
        guard cellHeight > 0, !photos.isEmpty else { return [] }
        let (stepX, stepY) = (cellWidth + Self.spacing, cellHeight + Self.spacing)
        let rows = (photos.count + columns - 1) / columns
        let firstRow = max(0, Int(rect.minY / stepY))
        let lastRow = min(rows - 1, Int(rect.maxY / stepY))
        let firstCol = max(0, Int(rect.minX / stepX))
        let lastCol = min(columns - 1, Int(rect.maxX / stepX))
        guard firstRow <= lastRow, firstCol <= lastCol else { return [] }
        var found: [Photo] = []
        for row in firstRow...lastRow {
            for col in firstCol...lastCol {
                let index = row * columns + col
                let cell = CGRect(x: Double(col) * stepX, y: Double(row) * stepY, width: cellWidth, height: cellHeight)
                if index < photos.count, cell.intersects(rect) {
                    found.append(photos[index])
                }
            }
        }
        return found
    }

    /// Фото, к которому ведёт клавиша `key` от текущего (`cursor` или опорного). Без выделения —
    /// первое фото (End — последнее). По краям сетки — крайнее фото, а не выход за неё.
    private func move(_ key: KeyEquivalent, columns: Int, pageRows: Int) -> Photo? {
        guard !photos.isEmpty else { return nil }
        let last = photos.count - 1
        guard let id = cursor ?? selection.anchor, let index = photos.firstIndex(where: { $0.id == id }) else {
            return photos[key == .end ? last : 0]
        }
        let target = switch key {
        case .leftArrow: index - 1
        case .rightArrow: index + 1
        case .upArrow: index - columns
        case .downArrow: index + columns
        case .pageUp: index - columns * pageRows
        case .pageDown: index + columns * pageRows
        case .home: 0
        default: last
        }
        return photos[min(max(target, 0), last)]
    }

    /// Выделить фото с клавиатуры: без ⇧ — только его, с ⇧ — диапазон от опорного до него.
    /// Карту не центрирует (`onPick` — только щелчок), как и в GTK: иначе стрелки на вкладке
    /// карты меняли бы сам список, по которому идут.
    private func select(_ photo: Photo, extend: Bool) {
        cursor = photo.id
        if extend, let anchor = selection.anchor, let from = photos.firstIndex(where: { $0.id == anchor }),
           let to = photos.firstIndex(where: { $0.id == photo.id }) {
            selection = PhotoSelection(photos[min(from, to)...max(from, to)], anchor: anchor)
        } else {
            selection = PhotoSelection(photo)
        }
    }

    /// Щелчок по фото: ⌘ — добавить или убрать его, ⇧ — диапазон от опорного фото
    /// (с ⌘⇧ — добавить диапазон), без них — выделить только его.
    private func click(_ photo: Photo) {
        focused = true
        cursor = photo.id
        let flags = NSEvent.modifierFlags
        if flags.contains(.shift), let anchor = selection.anchor,
           let from = photos.firstIndex(where: { $0.id == anchor }),
           let to = photos.firstIndex(where: { $0.id == photo.id }) {
            var range = PhotoSelection(photos[min(from, to)...max(from, to)], anchor: anchor)
            if flags.contains(.command) {
                range.formUnion(selection)
            }
            selection = range
        } else if flags.contains(.command) || flags.contains(.shift) {
            selection.toggle(photo)
        } else {
            selection = PhotoSelection(photo)
            onPick(photo)
        }
    }

    private func cell(_ photo: Photo) -> some View {
        VStack(spacing: 4) {
            Thumbnail(path: photo.thumbPath)
            Text(photo.fileName).font(.caption).lineLimit(1).truncationMode(.middle)
            Text(PhotoDate.string(photo.takenAt)).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
        .padding(5)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(selection.contains(photo) ? Color.accentColor.opacity(0.3) : .clear)
        )
        .onGeometryChange(for: Double.self) { $0.size.height } action: { height in
            if abs(height - cellHeight) > 0.5 {
                cellHeight = height
            }
        }
        .contentShape(Rectangle())
        // Двойной щелчок — просмотр, с ⌘ — редактирование.
        .onTapGesture(count: 2) {
            selection = PhotoSelection(photo)
            if NSEvent.modifierFlags.contains(.command) {
                model.edit(photo)
            } else {
                model.open(photo)
            }
        }
        .onTapGesture { click(photo) }
        .contextMenu {
            // Правый щелчок по одному из нескольких выделенных — действия над всеми ними.
            if selection.count > 1, selection.contains(photo) {
                let all = selection.all
                Button(Core.text("copy"), systemImage: "doc.on.doc") { model.copy(all) }
                Divider()
                Button(Core.text("edit_exif"), systemImage: "pencil.and.list.clipboard") { model.editExif(all) }
                Divider()
                Button(Core.text("delete"), systemImage: "trash", role: .destructive) { model.trash(all) }
            } else {
                Button(Core.text("copy"), systemImage: "doc.on.doc") { model.copy([photo]) }
                Divider()
                Button(Core.text("view"), systemImage: "eye") { model.open(photo) }
                Button(Core.text("edit"), systemImage: "pencil") { model.edit(photo) }
                Button(Core.text("show_in_finder"), systemImage: "folder") { model.showInFinder(photo) }
                Divider()
                Button(Core.text("info"), systemImage: "info.circle") {
                    selection = PhotoSelection(photo)
                    model.infoOpen = true
                }
                Button(Core.text("show_on_map"), systemImage: "mappin.and.ellipse") { model.showOnMap(photo) }
                Button(Core.text("edit_exif"), systemImage: "pencil.and.list.clipboard") { model.editExif([photo]) }
                Divider()
                Button(Core.text("delete"), systemImage: "trash", role: .destructive) { model.trash([photo]) }
            }
        }
        .id(photo.id)
    }
}

/// Заголовок панели с фото: подпись слева, выбор сортировки справа.
struct PaneHeader: View {
    let title: String
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        HStack {
            Text(title).font(.headline).lineLimit(1).truncationMode(.tail)
            Spacer()
            // Меню с явной подписью: выпадающий список (`Picker`) в этой строке временами
            // сжимался до одной буквы.
            Menu(Core.text(model.newestFirst ? "sort_newest" : "sort_oldest")) {
                Picker("", selection: $model.newestFirst) {
                    Text(Core.text("sort_newest")).tag(true)
                    Text(Core.text("sort_oldest")).tag(false)
                }
                .pickerStyle(.inline)
                .labelsHidden()
            }
            .fixedSize()
            .help(Core.text("sort_tooltip"))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }
}

/// Нижняя строка панели: слайдер масштаба миниатюр между кнопками «−» и «+».
struct ThumbScaleBar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        let range = AppModel.thumbScaleRange
        HStack(spacing: 6) {
            Spacer()
            Button(Core.text("thumbs_smaller"), systemImage: "minus.magnifyingglass") {
                model.stepThumbScale(-AppModel.thumbScaleButtonStep)
            }
            .disabled(model.thumbScale <= range.lowerBound)
            .help(Core.text("thumbs_smaller"))
            // Без `step:`: с ним macOS рисует под слайдером отметки. Значение округляется
            // до шага здесь.
            Slider(value: Binding(
                get: { model.thumbScale },
                set: { model.thumbScale = ($0 / AppModel.thumbScaleStep).rounded() * AppModel.thumbScaleStep }
            ), in: range) { editing in
                if !editing {
                    model.saveThumbScale()
                }
            }
            .frame(width: 150)
            .help(Core.text("thumb_size_tooltip"))
            Button(Core.text("thumbs_larger"), systemImage: "plus.magnifyingglass") {
                model.stepThumbScale(AppModel.thumbScaleButtonStep)
            }
            .disabled(model.thumbScale >= range.upperBound)
            .help(Core.text("thumbs_larger"))
            Text("\(Int(model.thumbScale))%").monospacedDigit().foregroundStyle(.secondary).frame(width: 44, alignment: .trailing)
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
    }
}

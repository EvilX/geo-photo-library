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
    @Binding var selection: Photo?
    /// Пользователь выбрал фото щелчком (а не программа выделила его сама).
    var onPick: (Photo) -> Void = { _ in }
    @Environment(AppModel.self) private var model

    var body: some View {
        let size = Self.baseSize * model.thumbScale / 100
        ScrollViewReader { proxy in
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: size, maximum: size * 1.4), spacing: 10)], spacing: 10) {
                    ForEach(photos) { photo in
                        cell(photo)
                    }
                }
                .padding(10)
            }
            .onChange(of: selection?.id) { _, id in
                if let id {
                    withAnimation { proxy.scrollTo(id) }
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

    private func cell(_ photo: Photo) -> some View {
        VStack(spacing: 4) {
            Thumbnail(path: photo.thumbPath)
            Text(photo.fileName).font(.caption).lineLimit(1).truncationMode(.middle)
            Text(PhotoDate.string(photo.takenAt)).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
        .padding(5)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(selection?.id == photo.id ? Color.accentColor.opacity(0.3) : .clear)
        )
        .contentShape(Rectangle())
        // Двойной щелчок — просмотр, с ⌘ — редактирование.
        .onTapGesture(count: 2) {
            selection = photo
            if NSEvent.modifierFlags.contains(.command) {
                model.edit(photo)
            } else {
                model.open(photo)
            }
        }
        .onTapGesture {
            selection = photo
            onPick(photo)
        }
        .contextMenu {
            Button(Core.text("copy"), systemImage: "doc.on.doc") { model.copy(photo) }
            Divider()
            Button(Core.text("view"), systemImage: "eye") { model.open(photo) }
            Button(Core.text("edit"), systemImage: "pencil") { model.edit(photo) }
            Button(Core.text("show_in_finder"), systemImage: "folder") { model.showInFinder(photo) }
            Divider()
            Button(Core.text("info"), systemImage: "info.circle") {
                selection = photo
                model.infoOpen = true
            }
            Button(Core.text("show_on_map"), systemImage: "mappin.and.ellipse") { model.showOnMap(photo) }
            Divider()
            Button(Core.text("delete"), systemImage: "trash", role: .destructive) { model.trash(photo) }
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
            Slider(value: $model.thumbScale, in: range, step: AppModel.thumbScaleStep) { editing in
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

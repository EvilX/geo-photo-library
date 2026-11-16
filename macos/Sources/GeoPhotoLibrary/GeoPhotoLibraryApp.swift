// Приложение macOS: окно с вкладками «Карта» и «По датам», настройки, «О программе», отладочные хуки.
import AppKit
import SwiftUI

@main
struct GeoPhotoLibraryApp: App {
    @State private var model: AppModel

    init() {
        Core.start()
        _model = State(initialValue: AppModel())
    }

    var body: some Scene {
        Window("Geo photo library", id: "main") {
            ContentView()
                .environment(model)
                .frame(minWidth: 900, minHeight: 540)
        }
        .defaultSize(width: 1280, height: 800)
        .commands {
            CommandGroup(replacing: .appInfo) {
                Button(Core.text("about")) { showAbout() }
            }
            CommandGroup(after: .toolbar) {
                Button(Core.text("rescan")) { model.rescan() }
                    .keyboardShortcut("r")
                    .disabled(model.libraryPath == nil)
                Button(Core.text("info")) { model.infoOpen.toggle() }
                    .keyboardShortcut("i")
            }
        }

        Settings {
            SettingsView().environment(model)
        }
    }
}

/// Стандартное окно «О программе» с кратким описанием.
@MainActor
func showAbout() {
    NSApplication.shared.orderFrontStandardAboutPanel(options: [
        .credits: NSAttributedString(
            string: Core.text("app_summary"),
            attributes: [.foregroundColor: NSColor.labelColor, .font: NSFont.systemFont(ofSize: 11)]
        ),
    ])
    NSApplication.shared.activate()
}

struct ContentView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        @Bindable var model = model
        VStack(spacing: 0) {
            if model.libraryPath == nil {
                ContentUnavailableView {
                    Label(Core.text("photo_folder"), systemImage: "photo.on.rectangle.angled")
                } description: {
                    Text(Core.text("folder_hint"))
                } actions: {
                    Button(Core.text("browse")) { chooseLibrary(model) }
                }
            } else {
                // Обе вкладки живут постоянно: карта не теряет положение при переключении.
                ZStack {
                    MapTab().opacity(model.tab == .map ? 1 : 0).allowsHitTesting(model.tab == .map)
                    DatesTab().opacity(model.tab == .dates ? 1 : 0).allowsHitTesting(model.tab == .dates)
                }
            }
            Divider()
            HStack(spacing: 10) {
                Text(model.status).font(.callout).lineLimit(1)
                Spacer()
                if model.scanning {
                    ProgressView(value: model.progress).frame(width: 180)
                }
            }
            .padding(.horizontal, 10)
            .frame(height: 26)
        }
        .inspector(isPresented: $model.infoOpen) {
            InfoPanel(photo: model.selectedPhoto)
                .inspectorColumnWidth(min: 260, ideal: 320, max: 460)
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("", selection: $model.tab) {
                    ForEach(AppTab.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            }
            // Вкладки — по центру заголовка, кнопки — у правого края окна: без растяжки
            // система ставит их вплотную к вкладкам.
            ToolbarItem(placement: .primaryAction) {
                Spacer()
            }
            ToolbarItemGroup(placement: .primaryAction) {
                Button(Core.text("rescan"), systemImage: "arrow.clockwise") { model.rescan() }
                    .help(Core.text("rescan"))
                    .disabled(model.libraryPath == nil)
                Toggle(isOn: $model.infoOpen) {
                    Label(Core.text("info"), systemImage: "info.circle")
                }
                .help(Core.text("info"))
            }
        }
        .alert(
            model.alert?.title ?? "",
            isPresented: Binding(get: { model.alert != nil }, set: { if !$0 { model.alert = nil } }),
            presenting: model.alert
        ) { _ in
            Button(Core.text("ok")) {}
        } message: { alert in
            Text(alert.detail)
        }
        .onAppear {
            // Запущенный не из .app (swift run) процесс сам не становится обычным приложением.
            NSApplication.shared.setActivationPolicy(.regular)
            NSApplication.shared.activate()
            model.start()
            DebugHooks.install(model) { openSettings() }
        }
    }
}

/// Системный диалог выбора папки библиотеки.
@MainActor
func chooseLibrary(_ model: AppModel) {
    let panel = NSOpenPanel()
    panel.canChooseDirectories = true
    panel.canChooseFiles = false
    panel.allowsMultipleSelection = false
    panel.message = Core.text("photo_folder")
    if let current = model.libraryPath {
        panel.directoryURL = URL(fileURLWithPath: current)
    }
    if panel.runModal() == .OK, let url = panel.url {
        model.setLibrary(url.path)
    }
}

/// Настройки (⌘,): папка библиотеки, просмотрщик и редактор. Применяются сразу, как принято в macOS.
struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var apps: [Apps.Choice] = []

    var body: some View {
        @Bindable var model = model
        Form {
            Section {
                LabeledContent(Core.text("photo_folder")) {
                    HStack {
                        Text(model.libraryPath ?? "—")
                            .lineLimit(1)
                            .truncationMode(.head)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button(Core.text("browse")) { chooseLibrary(model) }
                    }
                }
            } footer: {
                hint("folder_hint")
            }
            Section {
                appPicker("photo_viewer", $model.viewerID)
                appPicker("photo_editor", $model.editorID)
            } footer: {
                hint("apps_hint")
            }
        }
        .formStyle(.grouped)
        .frame(width: 600)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { apps = Apps.imageApps() }
    }

    private func hint(_ key: String) -> some View {
        Text(Core.text(key))
            .font(.callout)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func appPicker(_ title: String, _ selection: Binding<String?>) -> some View {
        Picker(Core.text(title), selection: selection) {
            Text(Core.text("automatic")).tag(String?.none)
            Divider()
            ForEach(apps) { app in
                Text(app.name).tag(String?.some(app.id))
            }
            // Выбранной программы больше нет в системе — показать её идентификатор.
            if let id = selection.wrappedValue, !apps.contains(where: { $0.id == id }) {
                Text(id).tag(String?.some(id))
            }
        }
    }
}

/// Проверка без участия человека (см. AGENTS.md), переменные `GEO_PHOTO_LIBRARY_*`:
/// `SCREENSHOT=out.png` — через `DELAY` секунд (по умолчанию 6) сохранить окно в PNG и выйти;
/// `ACTION=settings|about`, `INFO=1`, `SEARCH=<запрос>`, `LOCATE=1` — как в версии GTK;
/// `PHOTO_ACTION=copy|show-on-map|trash` — через 3 с выполнить пункт меню для первого фото вкладки.
@MainActor
enum DebugHooks {
    /// Поиск вкладки карты (регистрируется ею самой).
    static weak var finder: PlaceFinder?
    private static var installed = false

    static func install(_ model: AppModel, openSettings: @escaping () -> Void) {
        guard !installed else { return }
        installed = true
        let env = ProcessInfo.processInfo.environment
        let after = { (delay: Double, action: @escaping @MainActor () -> Void) in
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { action() }
        }
        switch env["GEO_PHOTO_LIBRARY_ACTION"] {
        case "settings": after(1) { openSettings() }
        case "about": after(1) { showAbout() }
        default: break
        }
        // Первое фото открытой вкладки (список дат принадлежит самой вкладке — там берём
        // самое новое фото с координатами).
        let firstPhoto = { () -> Photo? in
            model.tab == .map ? model.ordered(model.mapVisible).first : model.geotagged.photos.last
        }
        if env["GEO_PHOTO_LIBRARY_INFO"] != nil {
            after(3) {
                if let photo = firstPhoto() {
                    model.showInfo(photo)
                }
            }
        }
        if let action = env["GEO_PHOTO_LIBRARY_PHOTO_ACTION"] {
            after(3) {
                guard let photo = firstPhoto() else { return }
                switch action {
                case "copy": model.copy(photo)
                case "show-on-map": model.showOnMap(photo)
                case "trash": model.trash(photo)
                default: break
                }
            }
        }
        if let query = env["GEO_PHOTO_LIBRARY_SEARCH"] {
            after(2) {
                finder?.query = query
                finder?.search(model)
            }
        }
        if env["GEO_PHOTO_LIBRARY_LOCATE"] != nil {
            after(2) { finder?.locate(model) }
        }
        if let path = env["GEO_PHOTO_LIBRARY_SCREENSHOT"] {
            after(env["GEO_PHOTO_LIBRARY_DELAY"].flatMap(Double.init) ?? 6) {
                screenshot(to: path)
                NSApplication.shared.terminate(nil)
            }
        }
    }

    /// Снимается активное окно: если открыт диалог (настройки, «О программе») — он.
    private static func screenshot(to path: String) {
        let app = NSApplication.shared
        guard let window = app.keyWindow ?? app.windows.first(where: { $0.isVisible && $0.contentView != nil }),
              let frame = window.contentView?.superview ?? window.contentView
        else { return }
        // Снимок через сервер окон: в нём есть и карта (она рисуется Metal мимо обычной отрисовки видов).
        let image = CGWindowListCreateImage(
            .null, .optionIncludingWindow, CGWindowID(window.windowNumber), [.boundsIgnoreFraming, .bestResolution]
        )
        let rep: NSBitmapImageRep?
        if let image {
            rep = NSBitmapImageRep(cgImage: image)
        } else {
            rep = frame.bitmapImageRepForCachingDisplay(in: frame.bounds)
            rep.map { frame.cacheDisplay(in: frame.bounds, to: $0) }
        }
        guard let data = rep?.representation(using: .png, properties: [:]) else { return }
        do {
            try data.write(to: URL(fileURLWithPath: path))
        } catch {
            FileHandle.standardError.write(Data("Снимок окна не сохранён: \(error)\n".utf8))
        }
    }
}

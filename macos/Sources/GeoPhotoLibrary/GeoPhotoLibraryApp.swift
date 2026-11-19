// Приложение macOS: окно с вкладками «Карта» и «По датам», настройки, «О программе», отладочные хуки.
import AppKit
import SwiftUI

@main
struct GeoPhotoLibraryApp: App {
    @State private var model: AppModel

    init() {
        // Меню «Правка» — только действия с фото: AppKit сам добавляет в него
        // автозаполнение, диктовку и «Эмодзи и символы», а вводить здесь нечего, кроме
        // названия места. Ключи читаются при запуске, поэтому — до создания меню.
        UserDefaults.standard.register(defaults: [
            "NSAutoFillSystemInsertMenuEnabled": false,
            "NSDisabledDictationMenuItem": true,
            "NSDisabledCharacterPaletteMenuItem": true,
        ])
        Core.start()
        TextEditKeys.install()
        _model = State(initialValue: AppModel())
    }

    var body: some Scene {
        Window("GeoPhotoLibrary", id: "main") {
            ContentView()
                .environment(model)
                .frame(minWidth: 900, minHeight: 540)
        }
        .defaultSize(width: 1280, height: 800)
        .commands {
            CommandGroup(replacing: .appInfo) {
                Button(Core.text("about")) { showAbout() }
            }
            // Меню «Правка» — действия с выбранными фото, как в их контекстном меню:
            // копирование и удаление — над всеми выделенными, остальное — только над одним.
            // Правку текста в строке поиска без этих пунктов обслуживает TextEditKeys.
            CommandGroup(replacing: .undoRedo) {}
            CommandGroup(replacing: .pasteboard) {
                let selected = model.selection
                let photo = selected.single
                Button(Core.text("copy")) { model.copy(selected.all) }
                    .keyboardShortcut("c")
                    .disabled(selected.isEmpty)
                Button(Core.text("delete")) { model.trash(selected.all) }
                    .keyboardShortcut(.delete)
                    .disabled(selected.isEmpty)
                Button(Core.text("select_all")) { model.selectAll() }
                    .keyboardShortcut("a")
                Divider()
                Button(Core.text("view")) { photo.map(model.open) }
                    .keyboardShortcut("o")
                    .disabled(photo == nil)
                Button(Core.text("edit")) { photo.map(model.edit) }
                    .keyboardShortcut("e")
                    .disabled(photo == nil)
                Button(Core.text("show_in_finder")) { photo.map(model.showInFinder) }
                    .disabled(photo == nil)
            }
            CommandGroup(after: .toolbar) {
                Button(Core.text("rescan")) { model.rescan() }
                    .keyboardShortcut("r")
                    .disabled(model.libraryPath == nil)
                Button(Core.text("info")) { model.infoOpen.toggle() }
                    .keyboardShortcut("i")
            }
            // Справки у приложения нет: системный пункт открыл бы пустое окно.
            CommandGroup(replacing: .help) {}
        }

        Settings {
            SettingsView().environment(model)
        }
    }
}

/// Сочетания правки текста для строки поиска. В меню «Правка» нет пунктов «Вырезать»,
/// «Вставить», «Отменить» и т. п., а без них AppKit не передаёт ⌘X/⌘V/⌘Z полю ввода;
/// ⌘C и ⌘⌫ в меню заняты действиями с фото. Поэтому, пока ввод идёт в текстовое поле,
/// эти сочетания перехватываются до меню и отдаются ему.
@MainActor
enum TextEditKeys {
    static func install() {
        NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard let text = NSApplication.shared.keyWindow?.firstResponder as? NSTextView, text.isEditable else {
                return event
            }
            let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            let shift = flags == [.command, .shift]
            guard flags == .command || shift else { return event }
            let action: Selector
            switch (event.charactersIgnoringModifiers?.lowercased(), shift) {
            case ("x", false): action = #selector(NSText.cut(_:))
            case ("c", false): action = #selector(NSText.copy(_:))
            case ("v", false): action = #selector(NSText.paste(_:))
            case ("a", false): action = #selector(NSText.selectAll(_:))
            case ("z", false): action = Selector(("undo:"))
            case ("z", true): action = Selector(("redo:"))
            default:
                // ⌘⌫ — удалить до начала строки, как обычно в поле ввода.
                guard event.keyCode == 51, !shift else { return event }
                text.doCommand(by: #selector(NSResponder.deleteToBeginningOfLine(_:)))
                return nil
            }
            NSApplication.shared.sendAction(action, to: text, from: nil)
            return nil
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
    @State private var infoWidth = 320.0

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
                // Панель «Информация» — своя, под заголовком окна. Системный `.inspector`
                // с Liquid Glass занимает правую часть заголовка, и вкладки по центру
                // сдвигались влево при каждом открытии панели.
                SplitPane(
                    side: .trailing, width: $infoWidth, sideRange: 260...460, minMain: 500,
                    sideVisible: model.infoOpen
                ) {
                    // Обе вкладки живут постоянно: карта не теряет положение при переключении.
                    ZStack {
                        MapTab().opacity(model.tab == .map ? 1 : 0).allowsHitTesting(model.tab == .map)
                        DatesTab().opacity(model.tab == .dates ? 1 : 0).allowsHitTesting(model.tab == .dates)
                    }
                } sidePane: {
                    // Несколько выделенных фото — панель пуста.
                    InfoPanel(photo: model.selectedPhoto, multiple: model.selection.count > 1)
                        .background(Color(nsColor: .windowBackgroundColor))
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
        // Подтверждение удаления; «Отмена» — кнопка по умолчанию (Esc и Enter).
        .alert(
            model.pendingTrash.map(AppModel.trashQuestion) ?? "",
            isPresented: Binding(get: { model.pendingTrash != nil }, set: { if !$0 { model.pendingTrash = nil } }),
            presenting: model.pendingTrash
        ) { photos in
            Button(Core.text("cancel"), role: .cancel) {}
            Button(Core.text("move_to_trash"), role: .destructive) { model.confirmTrash(photos) }
        } message: { _ in
            Text(Core.text("confirm_trash_detail"))
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
/// `APPEARANCE=dark|light` — оформление окна независимо от системного.
/// `DATE_FILTER=open|<год>` — через 3 с открыть фильтр карты по датам (и выбрать год).
@MainActor
enum DebugHooks {
    /// Поиск вкладки карты (регистрируется ею самой).
    static weak var finder: PlaceFinder?
    private static var installed = false

    static func install(_ model: AppModel, openSettings: @escaping () -> Void) {
        guard !installed else { return }
        installed = true
        let env = ProcessInfo.processInfo.environment
        switch env["GEO_PHOTO_LIBRARY_APPEARANCE"] {
        case "dark": NSApplication.shared.appearance = NSAppearance(named: .darkAqua)
        case "light": NSApplication.shared.appearance = NSAppearance(named: .aqua)
        default: break
        }
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
                case "copy": model.copy([photo])
                case "show-on-map": model.showOnMap(photo)
                case "trash": model.trash([photo])
                default: break
                }
            }
        }
        if let n = env["GEO_PHOTO_LIBRARY_SELECT"].flatMap(Int.init) {
            after(4) {
                let photos = model.tab == .map ? model.ordered(model.mapVisible) : model.ordered(model.datePhotos)
                model.selection = PhotoSelection(photos.prefix(n))
            }
        }
        if let query = env["GEO_PHOTO_LIBRARY_SEARCH"] {
            after(2) {
                finder?.query = query
                finder?.search(model)
            }
        }
        if let filter = env["GEO_PHOTO_LIBRARY_DATE_FILTER"] {
            after(3) {
                // Год, год-месяц или год-месяц-день. Период ставится до открытия панели —
                // как после её закрытия: дерево должно раскрыться до выбранной строки.
                let parts = filter.split(separator: "-").compactMap { Int($0) }
                if let year = parts.first {
                    model.mapPeriod = DatePeriod(
                        year: Int32(year),
                        month: UInt32(parts.count > 1 ? parts[1] : 0),
                        day: UInt32(parts.count > 2 ? parts[2] : 0)
                    )
                }
                model.mapFilterOpen = true
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

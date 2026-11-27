// Вкладка «По датам»: дерево год → месяц → день и сетка фото выбранного периода.
import AppKit
import SwiftUI

/// Серый полупрозрачный фон боковой панели, как в Finder. Сам по себе стиль списка
/// `.sidebar` его не рисует: дерево лежит не в `NavigationSplitView`, а в своём `SplitPane`.
struct SidebarBackground: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .sidebar
        view.blendingMode = .behindWindow
        view.state = .followsWindowActiveState
        return view
    }

    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}

/// Выпадающий список фильтра по координатам во всю ширину (системный NSPopUpButton).
struct GeoFilterPopUp: NSViewRepresentable {
    @Binding var selection: GeoFilter

    func makeCoordinator() -> Coordinator {
        Coordinator(selection: $selection)
    }

    func makeNSView(context: Context) -> NSPopUpButton {
        let button = NSPopUpButton(frame: .zero, pullsDown: false)
        button.addItems(withTitles: GeoFilter.allCases.map(\.title))
        button.target = context.coordinator
        button.action = #selector(Coordinator.changed(_:))
        // Ширину задаёт панель, а не самый длинный пункт.
        button.setContentHuggingPriority(.defaultLow, for: .horizontal)
        button.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return button
    }

    func updateNSView(_ button: NSPopUpButton, context: Context) {
        context.coordinator.selection = $selection
        button.selectItem(at: GeoFilter.allCases.firstIndex(of: selection) ?? 0)
    }

    @MainActor
    final class Coordinator: NSObject {
        var selection: Binding<GeoFilter>

        init(selection: Binding<GeoFilter>) {
            self.selection = selection
        }

        @objc func changed(_ button: NSPopUpButton) {
            let all = GeoFilter.allCases
            if all.indices.contains(button.indexOfSelectedItem) {
                selection.wrappedValue = all[button.indexOfSelectedItem]
            }
        }
    }
}

struct DatesTab: View {
    @Environment(AppModel.self) private var model
    @State private var selected: DateNode.ID?
    /// Ширина дерева дат.
    @State private var treeWidth = 230.0

    var body: some View {
        @Bindable var model = model
        SplitPane(side: .leading, width: $treeWidth, sideRange: 180...420, minMain: 400) {
            VStack(spacing: 0) {
                PaneHeader(title: find(selected)?.title ?? "")
                Divider()
                PhotoGrid(photos: model.dateList, selection: $model.dateSelection)
                Divider()
                ThumbScaleBar()
            }
        } sidePane: {
            // Как боковая панель Finder: без линий между строками, выделение — плашкой,
            // фон — системный материал боковой панели вместо белого.
            VStack(spacing: 0) {
                // Во всю ширину панели: Picker в стиле меню SwiftUI не растягивает.
                GeoFilterPopUp(selection: $model.dateFilter)
                    .help(Core.text("geo_filter_tooltip"))
                    .padding(.horizontal, 10)
                .padding(.vertical, 8)
                List(model.dates, children: \.children, selection: $selected) { node in
                    HStack {
                        Text(node.label)
                        Spacer()
                        Text(String(node.count)).foregroundStyle(.secondary).monospacedDigit()
                    }
                    .padding(.vertical, 3)
                }
                .listStyle(.sidebar)
                .scrollContentBackground(.hidden)
            }
            .background(SidebarBackground())
        }
        // Смена периода и изменения в БД (идёт сканирование) перечитывают список.
        .task(id: Query(node: selected, filter: model.dateFilter, revision: model.revision)) {
            await load()
        }
    }

    private struct Query: Hashable {
        let node: DateNode.ID?
        let filter: GeoFilter
        let revision: Int
    }

    private func find(_ id: DateNode.ID?) -> DateNode? {
        DateNode.find(id, in: model.dates)
    }

    private func load() async {
        // Ничего не выбрано или выбранный период исчез — показать последний год.
        if find(selected) == nil {
            selected = model.dates.first?.id
        }
        guard let node = find(selected) else {
            model.datePhotos = []
            model.dateSelection = PhotoSelection()
            return
        }
        let filter = model.dateFilter
        // БД занята сканером — прежний список остаётся; несколько попыток с паузой.
        var loaded: [Photo]?
        for attempt in 0..<4 where loaded == nil {
            if attempt > 0 {
                try? await Task.sleep(for: .seconds(1))
            }
            // Пока ждали, могли выбрать другой период.
            guard !Task.isCancelled else { return }
            loaded = await Task.detached(priority: .userInitiated) {
                Core.photos(year: node.year, month: node.month, day: node.day, filter: filter)
            }.value
        }
        guard !Task.isCancelled, let loaded, loaded != model.datePhotos else { return }
        model.datePhotos = loaded
        // Фото другого периода (или удалённые) из выделения выпадают; у оставшихся
        // значения — из нового списка.
        model.dateSelection.sync(with: loaded)
    }
}

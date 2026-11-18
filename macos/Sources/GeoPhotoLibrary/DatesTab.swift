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

struct DatesTab: View {
    @Environment(AppModel.self) private var model
    @State private var selected: DateNode.ID?
    /// Фото периода по дате съёмки.
    @State private var photos: [Photo] = []
    /// Ширина дерева дат.
    @State private var treeWidth = 230.0

    var body: some View {
        @Bindable var model = model
        SplitPane(side: .leading, width: $treeWidth, sideRange: 180...420, minMain: 400) {
            VStack(spacing: 0) {
                PaneHeader(title: find(selected)?.title ?? "")
                Divider()
                PhotoGrid(photos: model.ordered(photos), selection: $model.dateSelection)
                Divider()
                ThumbScaleBar()
            }
        } sidePane: {
            // Как боковая панель Finder: без линий между строками, выделение — плашкой,
            // фон — системный материал боковой панели вместо белого.
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
            .background(SidebarBackground())
        }
        // Смена периода и изменения в БД (идёт сканирование) перечитывают список.
        .task(id: Query(node: selected, revision: model.revision)) {
            await load()
        }
    }

    private struct Query: Hashable {
        let node: DateNode.ID?
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
            photos = []
            model.dateSelection = nil
            return
        }
        let loaded = await Task.detached(priority: .userInitiated) {
            Core.photos(year: node.year, month: node.month, day: node.day)
        }.value
        // Пока грузили, могли выбрать другой период.
        guard !Task.isCancelled, loaded != photos else { return }
        photos = loaded
        if let chosen = model.dateSelection, !loaded.contains(where: { $0.id == chosen.id }) {
            model.dateSelection = nil
        }
    }
}

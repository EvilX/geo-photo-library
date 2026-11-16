// Панель «Информация» справа: данные файла и EXIF выбранного фото.
// EXIF читается в фоне; пока панель скрыта, её содержимого нет и ничего не читается.
import SwiftUI

struct InfoPanel: View {
    let photo: Photo?
    @State private var info: PhotoInfo?
    @State private var allTagsOpen = false

    var body: some View {
        ScrollView {
            if let info, photo != nil {
                VStack(alignment: .leading, spacing: 16) {
                    // Все разделы — в одной сетке, чтобы значения стояли в одну колонку.
                    Grid(alignment: .topLeading, horizontalSpacing: 12, verticalSpacing: 5) {
                        ForEach(Array(info.sections.enumerated()), id: \.offset) { index, section in
                            Text(section.title)
                                .font(.headline)
                                .gridCellColumns(2)
                                .padding(.top, index == 0 ? 0 : 10)
                            rows(section.rows)
                        }
                    }
                    if let note = info.note {
                        Text(note).foregroundStyle(.secondary)
                    }
                    if let all = info.all, !all.rows.isEmpty {
                        DisclosureGroup(all.title, isExpanded: $allTagsOpen) {
                            Grid(alignment: .topLeading, horizontalSpacing: 12, verticalSpacing: 5) {
                                rows(all.rows)
                            }
                            .padding(.top, 6)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
            } else {
                Text(Core.text(photo == nil ? "select_photo" : "info"))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 36)
                    .opacity(photo == nil ? 1 : 0)
            }
        }
        // Панель следует за выбранным фото; результат устаревшего чтения отбрасывается.
        .task(id: photo?.id) {
            info = nil
            guard let photo else { return }
            let loaded = await Task.detached(priority: .userInitiated) { Core.info(photo) }.value
            if !Task.isCancelled {
                info = loaded
            }
        }
    }

    /// Строки «подпись — значение»; значение можно выделить и скопировать.
    private func rows(_ rows: [PhotoInfo.Row]) -> some View {
        ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
            GridRow {
                Text(row.key)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: 110, alignment: .leading)
                Text(value(row))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.callout)
        }
    }

    /// Дата съёмки — по системной локали, часовой пояс — в скобках.
    private func value(_ row: PhotoInfo.Row) -> String {
        guard let date = row.date else { return row.value }
        let text = PhotoDate.string(date)
        return row.value.isEmpty ? text : "\(text) (\(row.value))"
    }
}

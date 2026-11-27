// Панель «Информация» справа: данные файла и EXIF выбранного фото.
// EXIF читается в фоне; пока панель скрыта, её содержимого нет и ничего не читается.
import SwiftUI

struct InfoPanel: View {
    let photo: Photo?
    /// Выделено несколько фото — показывать нечего, даже подсказку.
    var multiple = false
    @Environment(AppModel.self) private var model
    @State private var info: PhotoInfo?
    /// Фото, данные которого показаны в `info`.
    @State private var shown: Photo.ID?
    @State private var allTagsOpen = false

    var body: some View {
        VStack(spacing: 0) {
            // Заголовок панели — на одной высоте с заголовком сетки фото слева.
            Text(Core.text("info"))
                .font(.headline)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .frame(minHeight: 28)
            Divider()
            content
        }
    }

    private var content: some View {
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
            } else if !multiple {
                Text(Core.text(photo == nil ? "select_photo" : "info"))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 36)
                    .opacity(photo == nil ? 1 : 0)
            }
        }
        // Панель следует за выбранным фото; результат устаревшего чтения отбрасывается.
        // Ключ — само фото (после правки у него другие координаты) и номер изменения файлов:
        // правка тегов EXIF полей фото не меняет, а данные панели — меняет.
        .task(id: Source(photo: photo, revision: model.filesRevision)) {
            // То же фото перечитывается без мигания: прежние данные видны до новых.
            if shown != photo?.id {
                info = nil
                shown = photo?.id
            }
            guard let photo else { return }
            let loaded = await Task.detached(priority: .userInitiated) { Core.info(photo) }.value
            if !Task.isCancelled {
                info = loaded
            }
        }
    }

    private struct Source: Hashable {
        let photo: Photo?
        let revision: Int
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

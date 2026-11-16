// Две панели с перетаскиваемым разделителем.
//
// Вместо `HSplitView`: вкладки лежат друг на друге и живут постоянно, а разделитель
// системного NSSplitView на скрытой вкладке продолжает управлять курсором — после
// перетаскивания тот оставался «стрелками» и над соседними панелями. Здесь курсором
// управляет сам разделитель, и только пока он принимает события мыши.
import AppKit
import SwiftUI

struct SplitPane<Main: View, Side: View>: View {
    enum Edge {
        case leading, trailing
    }

    /// С какой стороны боковая панель — её ширина фиксирована, основная занимает остальное.
    let side: Edge
    @Binding var width: Double
    let sideRange: ClosedRange<Double>
    let minMain: Double
    @ViewBuilder let mainPane: Main
    @ViewBuilder let sidePane: Side

    /// Ширина боковой панели в начале перетаскивания.
    @State private var dragStart: Double?

    var body: some View {
        GeometryReader { geo in
            let shown = clamp(width, total: geo.size.width)
            HStack(spacing: 0) {
                if side == .leading {
                    sidePane.frame(width: shown)
                    divider(total: geo.size.width)
                    mainPane.frame(maxWidth: .infinity)
                } else {
                    mainPane.frame(maxWidth: .infinity)
                    divider(total: geo.size.width)
                    sidePane.frame(width: shown)
                }
            }
        }
    }

    /// Ширина в допустимых пределах: основной панели остаётся не меньше `minMain`.
    private func clamp(_ value: Double, total: Double) -> Double {
        let upper = max(min(sideRange.upperBound, total - minMain), sideRange.lowerBound)
        return min(max(value, sideRange.lowerBound), upper)
    }

    private func divider(total: Double) -> some View {
        Rectangle()
            .fill(Color(nsColor: .separatorColor))
            .frame(width: 1)
            // Полоса захвата шире видимой линии.
            .overlay {
                Color.clear
                    .frame(width: 9)
                    .contentShape(Rectangle())
                    .resizeCursor()
                    .gesture(
                        // Глобальные координаты: сам разделитель движется вместе с мышью.
                        DragGesture(minimumDistance: 1, coordinateSpace: .global)
                            .onChanged { drag in
                                let start = dragStart ?? clamp(width, total: total)
                                dragStart = start
                                let delta = side == .leading ? drag.translation.width : -drag.translation.width
                                width = clamp(start + delta, total: total)
                            }
                            .onEnded { _ in dragStart = nil }
                    )
            }
            .zIndex(1)
    }
}

private extension View {
    /// Курсор изменения ширины над видом. В macOS 15+ им управляет система; в macOS 14 —
    /// пара push/pop строго по входу и выходу мыши.
    @ViewBuilder
    func resizeCursor() -> some View {
        if #available(macOS 15.0, *) {
            pointerStyle(.columnResize)
        } else {
            modifier(HoverCursor())
        }
    }
}

private struct HoverCursor: ViewModifier {
    @State private var pushed = false

    func body(content: Content) -> some View {
        content
            .onHover { inside in
                guard inside != pushed else { return }
                pushed = inside
                if inside {
                    NSCursor.resizeLeftRight.push()
                } else {
                    NSCursor.pop()
                }
            }
            .onDisappear {
                if pushed {
                    pushed = false
                    NSCursor.pop()
                }
            }
    }
}

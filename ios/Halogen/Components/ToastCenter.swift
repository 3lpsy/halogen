import Observation
import SwiftUI

/// Lightweight global toast queue — the native counterpart of the web's
/// ui-toast surface for actions that have no inline error/result slot
/// (background mutations, toolbar triggers, admin actions).
@MainActor
@Observable
final class ToastCenter {
    struct Toast: Identifiable, Equatable {
        let id = UUID()
        let message: String
        let isError: Bool
    }

    static let shared = ToastCenter()

    private(set) var toasts: [Toast] = []

    func success(_ message: String) { push(Toast(message: message, isError: false)) }
    func error(_ message: String) { push(Toast(message: message, isError: true)) }

    func dismiss(_ id: UUID) {
        toasts.removeAll { $0.id == id }
    }

    private func push(_ toast: Toast) {
        // Dedupe + cap: a burst (e.g. a drained batch dead-lettering) must
        // not stack the screen full of identical capsules.
        guard !toasts.contains(where: { $0.message == toast.message }) else { return }
        if toasts.count >= 4 { toasts.removeFirst() }
        toasts.append(toast)
        Task { [id = toast.id] in
            try? await Task.sleep(for: .seconds(4))
            dismiss(id)
        }
    }
}

/// Bottom-stacked toast overlay; attach once at the root (RootView).
struct ToastOverlay: ViewModifier {
    @State private var center = ToastCenter.shared

    func body(content: Content) -> some View {
        content.overlay(alignment: .bottom) {
            VStack(spacing: 8) {
                ForEach(center.toasts) { toast in
                    Text(toast.message)
                        .font(.footnote)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                        .background(
                            Capsule().fill(
                                toast.isError
                                    ? Color.red.opacity(0.92)
                                    : Color(.secondarySystemBackground))
                        )
                        .foregroundStyle(toast.isError ? Color.white : Color.primary)
                        .shadow(radius: 4)
                        .onTapGesture { center.dismiss(toast.id) }
                }
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 72)
            .animation(.easeInOut(duration: 0.2), value: center.toasts)
        }
    }
}

extension View {
    func toastOverlay() -> some View { modifier(ToastOverlay()) }
}

import HaryXCore
import SwiftUI

/// The menu bar panel's "Update to vX" row: shown while a release is available (and not skipped)
/// or downloaded and waiting. Opens HaryX › About & Updates, or restarts into a ready update.
struct UpdateLine: View {
    let updates: UpdateController

    var body: some View {
        switch updates.phase {
        case .ready(let version):
            row("HaryX \(version) is ready", systemImage: "checkmark.circle.fill", action: "Restart") {
                updates.restartNow()
            }
        case .downloading:
            Label("Downloading the HaryX update…", systemImage: "arrow.down.circle")
                .font(.caption).foregroundStyle(.secondary).padding(.top, 8)
        default:
            if let release = updates.available, let version = release.latest {
                row("HaryX \(version) is available", systemImage: "arrow.down.circle.fill", action: "Details") {
                    DashboardRouter.shared.open(.about)
                }
            }
        }
    }

    private func row(_ title: String, systemImage: String, action: String,
                     perform: @escaping () -> Void) -> some View {
        HStack {
            Label(title, systemImage: systemImage).font(.caption.weight(.medium)).foregroundStyle(.tint)
            Spacer()
            Button(action, action: perform).controlSize(.small)
        }
        .padding(.top, 8)
    }
}

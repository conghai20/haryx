import HaryXCore
import SwiftUI

extension Color {
    /// Brand cyan → warning → red as a quota fills (flat, no gradients).
    static func usage(_ pct: Double?) -> Color {
        Brand.usageFill(pct.map { UsageWindow(pct: $0) })
    }
}

struct UsageBar: View {
    let label: String
    let window: UsageWindow?
    var compact = false

    var body: some View {
        let pct = window?.pct
        VStack(alignment: .leading, spacing: 2) {
            if !compact {
                HStack {
                    Text(label).font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                    Spacer()
                    if let reset = resetText { Text(reset).font(.caption2).foregroundStyle(.tertiary) }
                }
            }
            HStack(spacing: 6) {
                if compact { Text(label).font(.system(size: 9, weight: .semibold)).foregroundStyle(.secondary).frame(width: 14, alignment: .leading) }
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Capsule().fill(.quaternary)
                        if let expected = window?.expectedPct {
                            Rectangle().fill(.secondary.opacity(0.6))
                                .frame(width: 1.5)
                                .offset(x: geo.size.width * min(1, expected / 100))
                        }
                        Capsule().fill(Brand.usageFill(window))
                            .frame(width: max(compact ? 3 : 4, geo.size.width * min(1, (pct ?? 0) / 100)))
                    }
                }
                .frame(height: compact ? 4 : 6)
                Text(pct.map { "\(Int($0))%" } ?? "—")
                    .font(.system(size: compact ? 10 : 11, weight: .medium).monospacedDigit())
                    .foregroundStyle(pct == nil ? .tertiary : .primary)
                    .frame(width: 34, alignment: .trailing)
            }
            if compact, let c = window?.countdown {
                Label(c, systemImage: "arrow.clockwise")
                    .labelStyle(.titleAndIcon)
                    .font(.system(size: 9).monospacedDigit())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .padding(.leading, 20)
                    .help(window?.clock.map { "Resets \($0)" } ?? "")
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(label) usage \(pct.map { "\(Int($0)) percent" } ?? "unknown")\(window?.countdown.map { ", resets in \($0)" } ?? "")")
    }

    /// "resets in 3h 18m · 03:20"
    private var resetText: String? {
        guard let c = window?.countdown else { return nil }
        return window?.clock.map { "resets in \(c) · \($0)" } ?? "resets in \(c)"
    }
}

struct PlanBadge: View {
    let text: String
    var tint: Color = Brand.cyan

    var body: some View {
        Text(text)
            .font(.system(size: 9, weight: .bold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(tint.opacity(0.18), in: Capsule())
            .foregroundStyle(tint)
    }
}

struct RunningBadge: View {
    let count: Int
    var body: some View {
        HStack(spacing: 3) {
            Circle().fill(Brand.cyan).frame(width: 6, height: 6)
            Text(count > 1 ? "\(count) running" : "running")
        }
        .font(.system(size: 9, weight: .semibold))
        .foregroundStyle(Brand.cyanText)
    }
}

/// Account badges shared by rows and cards.
struct AccountBadges: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        HStack(spacing: 4) {
            if account.active { PlanBadge(text: "ACTIVE", tint: Brand.cyanText) }
            if let plan = account.plan?.label { PlanBadge(text: plan.uppercased(), tint: .secondary) }
            if account.isDisabled { PlanBadge(text: "OFF", tint: .gray) }
            if let n = store.sessions[account.number]?.count, n > 0 { RunningBadge(count: n) }
        }
    }
}

struct BannerView: View {
    let banner: Banner
    var body: some View {
        Label(banner.text, systemImage: banner.isError ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
            .font(.caption)
            .foregroundStyle(banner.isError ? Brand.limit : Brand.cyanText)
            .lineLimit(3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
            .glassEffect(.regular.tint((banner.isError ? Brand.limit : Brand.cyan).opacity(0.12)), in: .rect(cornerRadius: 10))
            .transition(.move(edge: .bottom).combined(with: .opacity))
    }
}

/// Terminal picker used by menus: "Run in ▸ Terminal / Warp / …".
struct TerminalMenuItems: View {
    let action: (TerminalApp) -> Void
    var body: some View {
        ForEach(TerminalApp.installed) { app in
            Button {
                action(app)
            } label: {
                if let icon = app.icon { Label { Text(app.title) } icon: { Image(nsImage: icon) } } else { Text(app.title) }
            }
        }
    }
}

/// Per-account context menu, shared by the menu bar rows and dashboard cards.
/// "Sign In Again" for an account that needs it, or a spinner while its sign-in is pending.
struct SignInAgainButton: View {
    @Environment(AppStore.self) private var store
    let account: Account
    var prominent = true

    var body: some View {
        if store.loginPending(for: account) {
            HStack(spacing: 6) {
                ProgressView().controlSize(.mini)
                Text("Waiting for sign-in…").font(.caption).foregroundStyle(.secondary)
            }
            .help("Finish the sign-in in its window and your browser")
        } else if prominent {
            Button("Sign In Again") { store.signInAgain(account) }
                .buttonStyle(.glassProminent).tint(.orange)
                .help("Open Claude Code's sign-in for \(account.email)")
        } else {
            Button("Sign In Again") { store.signInAgain(account) }
                .buttonStyle(.glass)
                .help("Renew \(account.email)'s login before it expires")
        }
    }
}

extension Account {
    /// Whether the UI should offer "Sign In Again" now.
    var offersSignIn: Bool { canRelogin && (needsRelogin || loginDue()) }
}

struct AccountContextMenu: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        if !account.active {
            Button("Switch to \(account.displayName)") { Task { await store.switchTo(account) } }
        }
        Menu("Run Claude as this account") {
            TerminalMenuItems { store.runClaude(as: account, app: $0) }
        }
        Button("Run Claude in Folder…") {
            if let dir = AppStore.chooseFolder(prompt: "Run Claude Here") { store.runClaude(as: account, folder: dir) }
        }
        if account.canRelogin {
            Button("Sign In Again…") { store.signInAgain(account) }
                .disabled(store.loginPending(for: account))
        }
        Divider()
        Button(account.isDisabled ? "Enable in Rotation" : "Disable in Rotation") {
            Task { await store.setDisabled(account, !account.isDisabled) }
        }
        Button("Copy Email") {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(account.email, forType: .string)
        }
        Divider()
        Button("Show in Dashboard") { DashboardRouter.shared.reveal(account.number) }
    }
}

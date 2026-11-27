import HaryXCore
import SwiftUI

/// The account in use, above the grid: wider, a larger mascot and figure, and a cyan "In use" next to
/// its name (cyan is the live colour). The grid below holds only accounts you can switch to.
struct ActiveAccountCard: View {
    @Environment(AppStore.self) private var store
    let account: Account
    let selected: Bool

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 24) {
                identity.frame(minWidth: 260, maxWidth: 420, alignment: .leading)
                reading.frame(minWidth: 280, maxWidth: .infinity)
            }
            VStack(alignment: .leading, spacing: 14) {
                identity
                reading
            }
        }
        .padding(18)
        .background(selected ? AnyShapeStyle(Color.accentColor.opacity(0.18)) : AnyShapeStyle(.fill.quaternary),
                    in: .rect(cornerRadius: 16))
        .contentShape(.rect(cornerRadius: 16))
        .contextMenu { AccountContextMenu(account: account) }
    }

    private var identity: some View {
        HStack(alignment: .top, spacing: 14) {
            MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 96, phase: store.phase(for: account))
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(account.displayName).font(.title3.weight(.semibold)).lineLimit(1).truncationMode(.middle)
                    InUseLabel()
                }
                CardMeta(account: account)
                AccountBadges(account: account)
                Spacer(minLength: 10)
                HStack(spacing: 8) {
                    if account.offersSignIn { SignInAgainButton(account: account, prominent: account.needsRelogin) }
                    DesktopButton(account: account, showsTitle: true)
                    TerminalButton(account: account)
                }
                .controlSize(.small)
            }
        }
    }

    private var reading: some View {
        VStack(alignment: .leading, spacing: 6) {
            CardTide(account: account, figureSize: 44)
            CardNote(account: account)
        }
    }
}

/// "In use": the live account's marker, a cyan dot and label.
private struct InUseLabel: View {
    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(Brand.cyan).frame(width: 7, height: 7)
            Text("In use").font(.subheadline.weight(.semibold)).foregroundStyle(Brand.cyanText)
        }
        .accessibilityElement(children: .combine)
    }
}

/// An account you can switch to. An Off account (held out of rotation) steps back: a lighter fill,
/// a pause mark before its name, a dimmed mascot and Turn On in place of Switch. Its usage is still
/// tracked, so it keeps the full tide reading; "Off: held out of rotation" takes the note line.
struct AccountCard: View {
    @Environment(AppStore.self) private var store
    let account: Account
    let selected: Bool

    private var isOff: Bool { account.isDisabled }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 72, phase: store.phase(for: account))
                    .opacity(account.isResting ? 0.55 : 1)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 5) {
                        if isOff {
                            Image(systemName: "pause.circle.fill").foregroundStyle(.secondary)
                                .accessibilityLabel("Off")
                        }
                        Text(account.displayName).font(.headline).lineLimit(1).truncationMode(.middle)
                            .foregroundStyle(isOff ? .secondary : .primary)
                    }
                    CardMeta(account: account)
                    AccountBadges(account: account)
                }
                Spacer(minLength: 0)
            }
            CardTide(account: account)
            CardNote(account: account)
            HStack {
                if account.offersSignIn {
                    SignInAgainButton(account: account, prominent: account.needsRelogin)
                } else if isOff {
                    Button("Turn On") { Task { await store.setDisabled(account, false) } }
                        .buttonStyle(.glass)
                        .help("Put this account back in rotation")
                } else {
                    Button("Switch") { Task { await store.switchTo(account) } }.buttonStyle(.glass)
                }
                Spacer()
                DesktopButton(account: account, showsTitle: false)
                TerminalButton(account: account)
            }
            .controlSize(.small)
        }
        .padding(14)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(background, in: .rect(cornerRadius: 16))
        // The whole rounded card is clickable, padding and empty space included (not only its views).
        .contentShape(.rect(cornerRadius: 16))
        .contextMenu { AccountContextMenu(account: account) }
    }

    private var background: AnyShapeStyle {
        if selected { return AnyShapeStyle(Color.accentColor.opacity(0.18)) }
        return isOff ? AnyShapeStyle(.fill.quinary) : AnyShapeStyle(.fill.quaternary)
    }
}

/// Email (when an alias is shown), slot and organization.
private struct CardMeta: View {
    let account: Account

    var body: some View {
        Text(([account.alias != nil ? account.email : nil, "Slot \(account.number)", account.organizationName]
            .compactMap { $0 }).joined(separator: " · "))
            .font(.caption).foregroundStyle(.secondary).lineLimit(1)
    }
}

/// The status note, else "Off" for an account held out of rotation, else the wake summary. A reserved
/// line either way, so cards in a row stay level.
private struct CardNote: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        let fallback = account.isDisabled
            ? "Off: held out of rotation"
            : store.wake?.account(account.number)?.summary()
        Text(account.statusNote ?? fallback ?? " ")
            .font(.caption).foregroundStyle(account.statusNote != nil ? Brand.warn : .secondary)
            .lineLimit(1).truncationMode(.tail)
            .help(account.statusNote ?? "")
    }
}

/// Claude Desktop for this account. On grid cards an icon (the title is in its tooltip and the
/// right-click menu); on the In use card, with its title.
private struct DesktopButton: View {
    @Environment(AppStore.self) private var store
    let account: Account
    let showsTitle: Bool

    var body: some View {
        if let desktop = store.desktopAction(for: account) {
            Button { Task { await store.openDesktop(as: account) } } label: {
                if showsTitle { Label(desktop.title, systemImage: "macwindow") } else { Image(systemName: "macwindow") }
            }
            .buttonStyle(.glass)
            .disabled(store.busy != nil)
            .accessibilityLabel(desktop.title)
            .help(desktop == .setUp
                ? "Set up Claude Desktop: opens it on \(account.email)'s own folder, to sign in once"
                : "Open Claude Desktop as \(account.email)")
        }
    }
}

private struct TerminalButton: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        Menu {
            TerminalMenuItems { store.runClaude(as: account, app: $0) }
        } label: { Image(systemName: "terminal") } primaryAction: {
            store.runClaude(as: account)
        }
        .menuStyle(.button).buttonStyle(.glass).fixedSize()
        .accessibilityLabel("Run Claude as this account")
        .help("Run Claude as this account in a new terminal")
    }
}

/// The card's tide reading: the 5-hour % large, its reset, the staff, and the week.
struct CardTide: View {
    @Environment(AppStore.self) private var store
    let account: Account
    var figureSize: CGFloat = 28

    var body: some View {
        let five = account.usage?.fiveHour?.withPace(windowSeconds: UsageWindow.fiveHourSeconds)
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .lastTextBaseline) {
                Text(five?.pct.map { "\(Int($0))%" } ?? "—")
                    .font(Brand.display(figureSize, relativeTo: .title).monospacedDigit())
                    .foregroundStyle(five?.pct == nil ? .tertiary : .primary)
                Text("5-hour").font(.caption).foregroundStyle(.secondary)
                Spacer()
                if let clock = five?.clock {
                    Text("resets \(clock)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
            TideStaff(window: five, height: 5, off: account.isDisabled)
            // A reserved line, so cards in a row stay level when one has no 5-hour pace to name.
            HStack {
                if let d = five?.paceDelta, abs(d) >= UsageWindow.paceThreshold {
                    Text(d > 0 ? "\(d) pts ahead of pace" : "\(-d) pts under pace")
                        .foregroundStyle(d > 0 ? Brand.warn : .secondary)
                } else {
                    Text(verbatim: " ").accessibilityHidden(true)
                }
                Spacer()
            }
            .font(.caption)
            WeekStaff(window: account.usage?.sevenDay, off: account.isDisabled,
                      readings: store.weekReadings[account.number] ?? [])
            CardWeeklyPace(window: account.usage?.sevenDay)
        }
        .accessibilityElement(children: .combine)
    }
}

/// The week's pace under its staff: "N pts ahead of pace · runs out Sat 10:15" in amber, else
/// "N pts under pace" or "on pace" in secondary. A reserved line, so cards in a row stay level
/// while the CLI has no weekly pace yet (the first day after a reset).
private struct CardWeeklyPace: View {
    let window: UsageWindow?

    var body: some View {
        let pace = WeeklyPace(window)
        Text(pace?.text(runOut: Self.runOut) ?? " ")
            .font(.caption.monospacedDigit())
            .foregroundStyle(pace?.isAhead == true ? Brand.warn : .secondary)
            .lineLimit(1).truncationMode(.tail)
            .accessibilityLabel(pace.map { "Weekly pace: \($0.text(runOut: Self.runOut))" } ?? "")
    }

    private static func runOut(_ date: Date) -> String {
        date.formatted(.dateTime.weekday(.abbreviated).hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
    }
}

/// Account cards in rows of fixed-width cards. The width never changes with the window or the
/// details panel: only how many fit in a row does, so opening the panel moves cards to the next
/// row (animated with it) instead of stretching them. Cards in a row share the tallest one's height.
struct CardFlow: Layout {
    static let cardWidth: CGFloat = 264
    var spacing: CGFloat = 16

    private func columns(for width: CGFloat?) -> Int {
        guard let width, width.isFinite else { return 3 }
        return max(1, Int((width + spacing) / (Self.cardWidth + spacing)))
    }

    private func cardWidth(for width: CGFloat?) -> CGFloat {
        guard let width, width.isFinite else { return Self.cardWidth }
        return min(Self.cardWidth, width)
    }

    /// Each row's height: its tallest card at the card width.
    private func rowHeights(_ subviews: Subviews, columns: Int, cardWidth: CGFloat) -> [CGFloat] {
        stride(from: 0, to: subviews.count, by: columns).map { start in
            subviews[start..<min(start + columns, subviews.count)]
                .map { $0.sizeThatFits(ProposedViewSize(width: cardWidth, height: nil)).height }
                .max() ?? 0
        }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let cols = columns(for: proposal.width), w = cardWidth(for: proposal.width)
        let heights = rowHeights(subviews, columns: cols, cardWidth: w)
        let height = heights.reduce(0, +) + spacing * CGFloat(max(0, heights.count - 1))
        let used = CGFloat(min(cols, subviews.count)) * (w + spacing) - spacing
        return CGSize(width: proposal.width ?? max(0, used), height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let cols = columns(for: bounds.width), w = cardWidth(for: bounds.width)
        let heights = rowHeights(subviews, columns: cols, cardWidth: w)
        var y = bounds.minY
        for (row, height) in heights.enumerated() {
            for col in 0..<cols {
                let i = row * cols + col
                guard i < subviews.count else { break }
                let x = bounds.minX + CGFloat(col) * (w + spacing)
                subviews[i].place(at: CGPoint(x: x, y: y), anchor: .topLeading,
                                  proposal: ProposedViewSize(width: w, height: height))
            }
            y += height + spacing
        }
    }
}

import HaryXCore
import SwiftUI

/// Usage › The week: will the accounts auto-switch can use carry you to their resets? The CLI's
/// rotation verdict as one line, then an almanac row per account in switch order: name, Week
/// Tide, weekly %, pace and reset. Works without usage history (the tides then draw averages).
struct WeekSection: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            verdict
            // One almanac line per account when the pane is wide enough; otherwise each account
            // stacks (name and %, then the tide, then pace and reset), as the Windows app does.
            ViewThatFits(in: .horizontal) {
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 0) {
                    ForEach(rows) { account in
                        WeekRow(account: account, counted: counted(account), compact: false)
                        if account.id != rows.last?.id {
                            Divider().gridCellUnsizedAxes(.horizontal)
                        }
                    }
                }
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(rows) { account in
                        WeekRow(account: account, counted: counted(account), compact: true)
                        if account.id != rows.last?.id { Divider() }
                    }
                }
            }
        }
    }

    private func counted(_ account: Account) -> Bool {
        store.rotation?.accounts.contains(account.number) == true
    }

    /// Subscription accounts in switch order; API-key accounts have no week.
    private var rows: [Account] { store.accounts.filter { !$0.isAPIKey } }

    @ViewBuilder private var verdict: some View {
        if let rotation = store.rotation {
            VStack(alignment: .leading, spacing: 3) {
                Text(rotation.headline(clock: Self.clock))
                    .font(.title2.weight(.semibold))
                    .foregroundStyle(rotation.lasts ? AnyShapeStyle(.primary) : AnyShapeStyle(Brand.warn))
                Text(rotation.detail(clock: Self.clock))
                    .foregroundStyle(.secondary)
                if !rotation.unknown.isEmpty {
                    Text("Not counted yet, no weekly reading: \(rotation.unknown.map(name).joined(separator: ", ")).")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .accessibilityElement(children: .combine)
        } else {
            VStack(alignment: .leading, spacing: 3) {
                Text("The week").font(.title2.weight(.semibold))
                Text("The verdict appears once an account auto-switch can use has a weekly reading.")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func name(_ slot: Int) -> String { store.account(slot: slot)?.displayName ?? "Account \(slot)" }

    static func clock(_ date: Date) -> String {
        date.formatted(.dateTime.weekday(.abbreviated).hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
    }
}

/// One account's week: a row of the almanac. Clicking it opens the account's details.
private struct WeekRow: View {
    @Environment(AppStore.self) private var store
    let account: Account
    /// In the verdict: auto-switch may use it and it has a reading.
    let counted: Bool

    /// Stacked layout for a narrow pane.
    let compact: Bool

    private var week: UsageWindow? { account.usage?.sevenDay?.withPace(windowSeconds: UsageWindow.sevenDaySeconds) }

    var body: some View {
        if compact {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline) { who; Spacer(); percent }
                tide
                HStack(alignment: .firstTextBaseline) { paceText; Spacer(); reset }
            }
            .modifier(OpensAccount(account: account))
        } else {
            GridRow(alignment: .center) {
                who.frame(width: 230, alignment: .leading)
                tide.frame(minWidth: 160, maxWidth: 420)
                percent.frame(width: 44, alignment: .trailing)
                paceText.gridColumnAlignment(.leading)
                reset.gridColumnAlignment(.trailing)
            }
            .modifier(OpensAccount(account: account))
        }
    }

    private var who: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(account.displayName).lineLimit(1).truncationMode(.middle)
                .foregroundStyle(account.isDisabled ? .secondary : .primary)
            if let note {
                Text(note).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var tide: some View {
        WeekTideView(window: week, readings: store.weekReadings[account.number] ?? [],
                     off: account.isDisabled, height: 22)
    }

    private var percent: some View {
        Text(week?.pct.map { "\(Int($0))%" } ?? "—")
            .font(.body.monospacedDigit().weight(.medium))
            .foregroundStyle((week?.pct ?? 0) >= 85 ? AnyShapeStyle(Brand.usageFill(week)) : AnyShapeStyle(.primary))
    }

    private var paceText: some View {
        let pace = WeeklyPace(account.usage?.sevenDay)
        return Text(pace?.text(runOut: WeekSection.clock) ?? (week == nil ? "No reading yet" : "Pace from tomorrow"))
            .font(.callout.monospacedDigit())
            .foregroundStyle(pace?.isAhead == true ? Brand.warn : .secondary)
            .lineLimit(1)
    }

    private var reset: some View {
        Text(week?.resetsAt.map { "resets \(WeekSection.clock($0))" } ?? "")
            .font(.callout.monospacedDigit()).foregroundStyle(.secondary)
    }

    /// Why a row sits outside the verdict.
    private var note: String? {
        if account.isDisabled { return "Held out of rotation" }
        if account.needsRelogin { return "Signed out" }
        if !counted && account.usage?.sevenDay != nil { return "Not used by auto-switch" }
        return nil
    }
}

/// A week row's padding and click: it opens the account's details.
private struct OpensAccount: ViewModifier {
    let account: Account

    func body(content: Content) -> some View {
        content
            .padding(.vertical, 8)
            .contentShape(.rect)
            .onTapGesture {
                DashboardRouter.shared.section = .accounts
                DashboardRouter.shared.showDetails(account.number)
            }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityHint("Opens this account's details")
    }
}

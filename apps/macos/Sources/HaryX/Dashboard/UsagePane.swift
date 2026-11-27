import Charts
import HaryXCore
import SwiftUI

/// AI › Usage. First, will the week last: the CLI's rotation verdict and each account's Week Tide.
/// Then where it went: tokens (or API-price cost) per day by account and the projects and sessions
/// that used the most, from `haryx ai usage` once usage history is on. Everything stays on this Mac.
struct UsagePane: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let model = store.usage
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                WeekSection()
                Divider()
                Text("Where it went").font(.title3.weight(.semibold))
                if let status = model.status, status.needsChoice {
                    ImportChoice(model: model)
                } else if let status = model.status, !status.enabled {
                    HistoryOff(model: model)
                } else {
                    UsageContent(model: model)
                }
            }
            .padding(20)
        }
        .task { model.reload() }
    }
}

/// Usage history is off: the week above still works; the breakdown needs history.
private struct HistoryOff: View {
    @Environment(AppStore.self) private var store
    let model: UsageModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Which accounts, projects and sessions used your quota appears here once usage history is on. HaryX reads token counts from Claude Code's transcripts on this Mac; no conversation text is stored.")
                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Button("Turn On Usage History") {
                Task { await store.configSet("history.enabled", "true"); model.reload() }
            }
        }
    }
}

/// The first-open question.
private struct ImportChoice: View {
    let model: UsageModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("HaryX can read your past Claude Code transcripts to show how each account, project and session used its quota. It reads token counts only, never the conversations, and keeps them on this Mac.")
                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            HStack {
                Button("Read Past Transcripts") { model.choose(importAll: true) }.buttonStyle(.glassProminent)
                Button("Start From Now") { model.choose(importAll: false) }
            }
        }
    }
}

private struct UsageContent: View {
    @Environment(AppStore.self) private var store
    @Bindable var model: UsageModel
    @AppStorage(Prefs.weeklySummary) private var weeklySummary = false

    var body: some View {
            VStack(alignment: .leading, spacing: 20) {
                controls
                if let error = model.error {
                    Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                }
                if let report = model.byAccount {
                    totals(report)
                    DailyChart(byDay: model.byDay, showCost: model.showCost, name: name, colors: colors)
                    accountsTable(report)
                    if let projects = model.projects { topTable("Top projects", projects, folder: true) }
                    if let sessions = model.sessions { topTable("Top sessions", sessions, folder: false) }
                    footer(report)
                } else if model.loading {
                    ProgressView().frame(maxWidth: .infinity).padding(.top, 40)
                }
            }
    }

    private func name(_ n: Int) -> String { store.account(slot: n)?.displayName ?? "Account \(n)" }

    @Environment(\.colorScheme) private var scheme

    /// One fixed colour per account for every chart and table here (`UsageChartSupport.palette`).
    private var colors: AccountColors { AccountColors(light: scheme == .light, name: name) }

    private var controls: some View {
        HStack {
            Picker("Range", selection: $model.range) {
                ForEach(UsageModel.Range.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented).fixedSize()
            Picker("Account", selection: $model.account) {
                Text("All accounts").tag(Int?.none)
                ForEach(store.accounts) { Text("\($0.number). \($0.displayName)").tag(Int?.some($0.number)) }
            }
            .fixedSize()
            Spacer()
            Picker("Show", selection: $model.showCost) {
                Text("Tokens").tag(false)
                Text("API cost").tag(true)
            }
            .pickerStyle(.segmented).fixedSize()
            if model.loading { ProgressView().controlSize(.small) }
            Button { model.reload() } label: { Image(systemName: "arrow.clockwise") }
                .help("Read new transcript lines and refresh")
        }
    }

    private func totals(_ r: UsageHistory.Report) -> some View {
        HStack(spacing: 28) {
            stat("Tokens", UsageHistory.shortTokens(r.total.tokens))
            stat("At API prices", r.total.cost.map { String(format: "$%.2f", $0) } ?? "—")
            stat("Replies", "\(r.total.replies)")
            stat("Output tokens", UsageHistory.shortTokens(r.total.output))
        }
    }

    private func stat(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(value).font(.title2.weight(.semibold)).monospacedDigit()
            Text(title).font(.caption).foregroundStyle(.secondary)
        }
    }

    private func accountsTable(_ r: UsageHistory.Report) -> some View {
        GroupBox("By account") {
            VStack(spacing: 6) {
                ForEach(r.rows) { row in
                    HStack {
                        let slot = row.key.flatMap(Int.init)
                        Circle().fill(slot.map(colors.color) ?? Color.secondary).frame(width: 8, height: 8)
                            .accessibilityHidden(true)
                        Text(slot.map(name) ?? "Unknown account").lineLimit(1).truncationMode(.middle)
                        if row.approximate {
                            Text("~").foregroundStyle(.secondary).help("Account worked out from the switch log, so approximate")
                        }
                        Spacer()
                        Text(UsageHistory.shortTokens(row.tokens)).monospacedDigit().frame(width: 80, alignment: .trailing)
                        Text(row.cost.map { String(format: "$%.2f", $0) } ?? "—").monospacedDigit().frame(width: 80, alignment: .trailing)
                        Text(String(format: "%.1f%%", row.share)).monospacedDigit().foregroundStyle(.secondary).frame(width: 56, alignment: .trailing)
                    }
                }
            }
            .padding(4)
        }
    }

    private func topTable(_ title: String, _ r: UsageHistory.Report, folder: Bool) -> some View {
        GroupBox(title) {
            VStack(spacing: 6) {
                ForEach(r.rows.prefix(8)) { row in
                    HStack {
                        Text(label(row, folder: folder)).lineLimit(1).truncationMode(.head)
                        Spacer()
                        if let last = row.lastActiveDate {
                            Text(last.formatted(.relative(presentation: .named))).font(.caption).foregroundStyle(.secondary)
                        }
                        Text(model.showCost ? (row.cost.map { String(format: "$%.2f", $0) } ?? "—") : UsageHistory.shortTokens(row.tokens))
                            .monospacedDigit().frame(width: 80, alignment: .trailing)
                        if folder, let path = row.key, FileManager.default.fileExists(atPath: path) {
                            Button { store.openTerminal(folder: path) } label: { Image(systemName: "terminal") }
                                .buttonStyle(.borderless).help("Open a terminal here")
                        }
                    }
                }
            }
            .padding(4)
        }
    }

    private func label(_ row: UsageHistory.Row, folder: Bool) -> String {
        guard let key = row.key, !key.isEmpty else { return "—" }
        if folder { return (key as NSString).abbreviatingWithTildeInPath }
        let who = row.accounts.map(name).joined(separator: ", ")
        return who.isEmpty ? String(key.prefix(8)) : "\(who) · \(key.prefix(8))"
    }

    private func footer(_ r: UsageHistory.Report) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Picker("Keep history for", selection: Binding(
                    get: { model.status?.keepDays ?? 90 },
                    set: { model.setKeepDays($0) })) {
                    Text("30 days").tag(30)
                    Text("90 days").tag(90)
                    Text("1 year").tag(365)
                    Text("Forever").tag(0)
                }
                .fixedSize()
                Toggle("Weekly summary notification", isOn: $weeklySummary)
            }
            Text("Costs are what the tokens would cost at Anthropic's API prices as of \(r.pricesAsOf); plans are flat-priced, so this is the value of your usage, not a bill. ~ marks days whose account was worked out from the switch log. Read from Claude Code's transcripts on this Mac; no conversation text is stored.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// The colour each account is drawn in, shared by the charts and the By account table.
struct AccountColors {
    let light: Bool
    let name: (Int) -> String

    func color(_ slot: Int) -> Color {
        Color(hex: Int(UsageChartSupport.colorHex(account: slot, light: light).dropFirst(), radix: 16) ?? 0)
    }

    /// The legend scale for a set of accounts, so a name keeps its colour in every chart.
    func domain(_ slots: [Int]) -> [String] { slots.map(name) }
    func range(_ slots: [Int]) -> [Color] { slots.map(color) }
}

/// Tokens (or cost) per day, stacked by account.
private struct DailyChart: View {
    let byDay: [Int: UsageHistory.Report]
    let showCost: Bool
    let name: (Int) -> String
    let colors: AccountColors

    private struct Point: Identifiable {
        let day: Date
        let slot: Int
        let value: Double
        var id: String { "\(slot)-\(day.timeIntervalSince1970)" }
    }

    private var points: [Point] {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        return byDay.flatMap { n, report in
            report.rows.compactMap { row -> Point? in
                guard let key = row.key, let day = try? Date(key, strategy: parse) else { return nil }
                let value = showCost ? (row.cost ?? 0) : Double(row.tokens)
                return Point(day: day, slot: n, value: value)
            }
        }
    }

    var body: some View {
        GroupBox(showCost ? "API cost per day" : "Tokens per day") {
            let points = points
            if points.isEmpty {
                Text("No usage in this range.").foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 80)
            } else {
                let slots = Array(Set(points.map(\.slot))).sorted()
                Chart(points) { p in
                    BarMark(x: .value("Day", p.day, unit: .day), y: .value(showCost ? "USD" : "Tokens", p.value))
                        .foregroundStyle(by: .value("Account", name(p.slot)))
                }
                .chartForegroundStyleScale(domain: colors.domain(slots), range: colors.range(slots))
                .chartYAxis {
                    AxisMarks { value in
                        AxisGridLine()
                        AxisValueLabel {
                            if let v = value.as(Double.self) {
                                Text(showCost ? "$" + UsageChartSupport.compact(v) : UsageChartSupport.compact(v))
                            }
                        }
                    }
                }
                .frame(height: 180)
            }
        }
    }
}

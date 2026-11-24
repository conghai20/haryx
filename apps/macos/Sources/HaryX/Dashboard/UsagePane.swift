import Charts
import HaryXCore
import SwiftUI

/// AI › Usage: quota over time, tokens (or API-price cost) per day by account, and the projects and
/// sessions that used the most. From `haryx ai usage`; everything stays on this Mac.
struct UsagePane: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let model = store.usage
        Group {
            if let status = model.status, status.needsChoice {
                ImportChoice(model: model)
            } else if let status = model.status, !status.enabled {
                ContentUnavailableView("Usage history is off", systemImage: "chart.bar.xaxis",
                                       description: Text("Turn it on in CLI Settings (history.enabled)."))
            } else {
                UsageContent(model: model)
            }
        }
        .task { model.reload() }
    }
}

/// The first-open question.
private struct ImportChoice: View {
    let model: UsageModel

    var body: some View {
        ContentUnavailableView {
            Label("Start your usage history", systemImage: "chart.bar.xaxis")
        } description: {
            Text("HaryX can read your past Claude Code transcripts to show how each account, project and session used its quota. It reads token counts only, never the conversations, and keeps them on this Mac.")
        } actions: {
            Button("Read Past Transcripts") { model.choose(importAll: true) }.buttonStyle(.glassProminent)
            Button("Start From Now") { model.choose(importAll: false) }
        }
    }
}

private struct UsageContent: View {
    @Environment(AppStore.self) private var store
    @Bindable var model: UsageModel
    @AppStorage(Prefs.weeklySummary) private var weeklySummary = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                controls
                if let error = model.error {
                    Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                }
                if let report = model.byAccount {
                    totals(report)
                    QuotaChart(readings: model.quota?.readings ?? [], name: name)
                    DailyChart(byDay: model.byDay, showCost: model.showCost, name: name)
                    accountsTable(report)
                    if let projects = model.projects { topTable("Top projects", projects, folder: true) }
                    if let sessions = model.sessions { topTable("Top sessions", sessions, folder: false) }
                    footer(report)
                } else if model.loading {
                    ProgressView().frame(maxWidth: .infinity).padding(.top, 60)
                }
            }
            .padding(20)
        }
    }

    private func name(_ n: Int) -> String { store.account(slot: n)?.displayName ?? "Account \(n)" }

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
                        Text(row.key.flatMap(Int.init).map(name) ?? "Unknown account").lineLimit(1)
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
        return "\(key.prefix(8))  \(who)"
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

/// 5-hour and weekly % per account over time.
private struct QuotaChart: View {
    let readings: [UsageHistory.Reading]
    let name: (Int) -> String

    var body: some View {
        GroupBox("Quota over time") {
            if readings.isEmpty {
                Text("Readings appear here as HaryX reads usage.").foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 80)
            } else {
                Chart {
                    ForEach(readings) { r in
                        if let five = r.fiveHour {
                            LineMark(x: .value("Time", r.date), y: .value("%", five),
                                     series: .value("Window", "\(name(r.account)) · 5h"))
                                .foregroundStyle(by: .value("Account", name(r.account)))
                                .lineStyle(StrokeStyle(lineWidth: 1.5))
                        }
                        if let seven = r.sevenDay {
                            LineMark(x: .value("Time", r.date), y: .value("%", seven),
                                     series: .value("Window", "\(name(r.account)) · 7d"))
                                .foregroundStyle(by: .value("Account", name(r.account)))
                                .lineStyle(StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
                        }
                    }
                    RuleMark(y: .value("Limit", 100)).foregroundStyle(.red.opacity(0.5))
                }
                .chartYScale(domain: 0...105)
                .frame(height: 180)
                Text("Solid: 5-hour window · dashed: weekly").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

/// Tokens (or cost) per day, stacked by account.
private struct DailyChart: View {
    let byDay: [Int: UsageHistory.Report]
    let showCost: Bool
    let name: (Int) -> String

    private struct Point: Identifiable {
        let day: Date
        let account: String
        let value: Double
        var id: String { "\(account)-\(day.timeIntervalSince1970)" }
    }

    private var points: [Point] {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        return byDay.flatMap { n, report in
            report.rows.compactMap { row -> Point? in
                guard let key = row.key, let day = try? Date(key, strategy: parse) else { return nil }
                let value = showCost ? (row.cost ?? 0) : Double(row.tokens)
                return Point(day: day, account: name(n), value: value)
            }
        }
    }

    var body: some View {
        GroupBox(showCost ? "API cost per day" : "Tokens per day") {
            if points.isEmpty {
                Text("No usage in this range.").foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 80)
            } else {
                Chart(points) { p in
                    BarMark(x: .value("Day", p.day, unit: .day), y: .value(showCost ? "USD" : "Tokens", p.value))
                        .foregroundStyle(by: .value("Account", p.account))
                }
                .frame(height: 180)
            }
        }
    }
}

import Foundation
import Testing
@testable import HaryXCore

@Suite struct WeekTideTests {
    private let reset = Date(timeIntervalSince1970: 7 * 86_400)
    private var start: Date { reset.addingTimeInterval(-WeekTide.period) }
    private func at(hours: Double) -> Date { start.addingTimeInterval(hours * 3600) }

    @Test func noReadingsDrawsTheAverageAsOneEstimatedLine() {
        let now = at(hours: 84)
        let segments = WeekTide.segments(readings: [], resetsAt: reset, pct: 40, now: now)
        #expect(segments.count == 1)
        #expect(segments[0].estimated)
        #expect(segments[0].points.first == WeekTide.Point(x: 0, y: 0))
        #expect(segments[0].points.last == WeekTide.Point(x: 0.5, y: 0.4))
    }

    @Test func readingsFromTheStartJoinTheOrigin() {
        let readings = stride(from: 0.5, through: 10, by: 0.5).map { (at: at(hours: $0), pct: $0) }
        let segments = WeekTide.segments(readings: readings, resetsAt: reset, pct: 10.2, now: at(hours: 10.2))
        #expect(segments.count == 1)
        #expect(!segments[0].estimated)
        #expect(segments[0].points.first == WeekTide.Point(x: 0, y: 0))
    }

    @Test func historyStartedMidWeekLeadsWithAnEstimate() {
        let readings = stride(from: 80.0, through: 84, by: 0.25).map { (at: at(hours: $0), pct: $0 / 2) }
        let segments = WeekTide.segments(readings: readings, resetsAt: reset, pct: 42, now: at(hours: 84.1))
        #expect(segments.first?.estimated == true)
        #expect(segments.dropFirst().allSatisfy { !$0.estimated })
    }

    @Test func aGapIsEstimatedNotDrawnAsRead() {
        let early = stride(from: 0.25, through: 2, by: 0.25).map { (at: at(hours: $0), pct: $0) }
        let late = stride(from: 20.0, through: 22, by: 0.25).map { (at: at(hours: $0), pct: $0) }
        let segments = WeekTide.segments(readings: early + late, resetsAt: reset, pct: 22, now: at(hours: 22.1))
        #expect(segments.map(\.estimated) == [false, true, false])
    }

    @Test func readingsFromLastWeekAreIgnored() {
        let old = [(at: start.addingTimeInterval(-3600), pct: 90.0)]
        let segments = WeekTide.segments(readings: old, resetsAt: reset, pct: 5, now: at(hours: 10))
        #expect(segments.allSatisfy { $0.points.allSatisfy { $0.y <= 0.05 } })
    }
}

@Suite struct RotationTests {
    private let clock: (Date) -> String = { "T\(Int($0.timeIntervalSince1970 / 3600))" }

    @Test func lastingWeek() {
        let r = Rotation(accounts: [1, 2], headroomPct: 120, burnPctPerHour: 1,
                         nextResetAt: Date(timeIntervalSince1970: 3600 * 10), runsDryAt: nil, lasts: true)
        #expect(r.headline(clock: clock) == "The week lasts")
        #expect(r.detail(clock: clock) == "2 accounts auto-switch can use carry you past the next reset, T10.")
    }

    @Test func runsDryBeforeTheReset() {
        let r = Rotation(accounts: [1], headroomPct: 10, burnPctPerHour: 2,
                         nextResetAt: Date(timeIntervalSince1970: 3600 * 24),
                         runsDryAt: Date(timeIntervalSince1970: 3600 * 10), lasts: false)
        #expect(r.headline(clock: clock) == "Runs dry T10")
        #expect(r.detail(clock: clock) == "14 h before the next reset, T24, across 1 account auto-switch can use.")
    }

    @Test func decodesFromList() throws {
        let json = #"{"schemaVersion":1,"activeAccountNumber":1,"accounts":[],"rotation":{"accounts":[1],"unknown":[],"headroomPct":30,"burnPctPerHour":0.8,"nextResetAt":"2026-11-28T05:00:00Z","runsDryAt":null,"lasts":true}}"#
        let list = try CSwapJSON.decoder.decode(AccountList.self, from: Data(json.utf8))
        #expect(list.rotation?.lasts == true)
        #expect(list.rotation?.accounts == [1])
    }
}

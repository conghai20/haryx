import Foundation
import Testing
@testable import HaryXCore

private let runOut = Date(timeIntervalSince1970: 1_800_000_000)
private func label(_ date: Date) -> String { "Sat 10:15" }

@Test func aheadByTheCardThresholdNamesTheRunOutWhenItWillNotLast() {
    let week = UsageWindow(pct: 32, expectedPct: 17.6, aheadOfPace: false,
                           projectedExhaustionAt: runOut, willLastToReset: false)
    let pace = WeeklyPace(week)
    #expect(pace?.drift == .ahead(14))
    #expect(pace?.isAhead == true)
    #expect(pace?.text(runOut: label) == "14 pts ahead of pace · runs out Sat 10:15")
}

@Test func aheadButLastingToTheResetHasNoRunOut() {
    let week = UsageWindow(pct: 40, expectedPct: 30, projectedExhaustionAt: runOut, willLastToReset: true)
    #expect(WeeklyPace(week)?.text(runOut: label) == "10 pts ahead of pace")
}

@Test func underAndOnPaceAreNotAheadAndNeverNameARunOut() {
    let under = WeeklyPace(UsageWindow(pct: 69, expectedPct: 82.6, projectedExhaustionAt: runOut, willLastToReset: false))
    #expect(under?.text(runOut: label) == "14 pts under pace")
    #expect(under?.isAhead == false)
    let near = WeeklyPace(UsageWindow(pct: 20, expectedPct: 18, projectedExhaustionAt: runOut, willLastToReset: false))
    #expect(near?.drift == .onPace)
    #expect(near?.text(runOut: label) == "on pace")
}

@Test func saysNothingWithoutTheCLIsExpectedPace() {
    #expect(WeeklyPace(UsageWindow(pct: 20, resetsAt: runOut)) == nil)
    #expect(WeeklyPace(UsageWindow(pct: nil, expectedPct: 20)) == nil)
    #expect(WeeklyPace(nil) == nil)
}

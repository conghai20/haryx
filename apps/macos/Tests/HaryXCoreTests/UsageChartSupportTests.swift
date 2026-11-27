import Testing
@testable import HaryXCore

@Suite struct UsageChartSupportTests {
    @Test func colourIsStablePerAccountAndWraps() {
        #expect(UsageChartSupport.colorHex(account: 1) == "#A78BFA")
        #expect(UsageChartSupport.colorHex(account: 9) == UsageChartSupport.colorHex(account: 1))
        #expect(UsageChartSupport.colorHex(account: 0) == UsageChartSupport.palette.last)
    }

    @Test func paletteAvoidsReadingColours() {
        let reserved = ["#22D3EE", "#0891B2", "#FF9F0A", "#FACC15", "#A3E635", "#D64545"]
        for list in [UsageChartSupport.palette, UsageChartSupport.lightPalette] {
            #expect(list.allSatisfy { !reserved.contains($0.uppercased()) })
            #expect(!list.contains { $0.uppercased().hasPrefix("#FF") })
            #expect(Set(list).count == list.count)
        }
        #expect(UsageChartSupport.palette.count == UsageChartSupport.lightPalette.count)
        #expect(UsageChartSupport.colorHex(account: 2, light: true) == "#059669")
    }

    @Test func breaksLinesAtGaps() {
        // Readings every minute, a 40-minute hole, then every minute again.
        let times: [Double] = [0, 60, 120, 180, 2580, 2640]
        #expect(UsageChartSupport.segments(times) == [0, 0, 0, 0, 1, 1])
    }

    @Test func keepsSteadySparseReadingsJoined() {
        // Every 10 minutes is the normal spacing here, so it stays one line.
        let times: [Double] = [0, 600, 1200, 1800]
        #expect(UsageChartSupport.segments(times) == [0, 0, 0, 0])
    }

    @Test func keepsHourlyReadingsJoined() {
        let times: [Double] = [0, 3600, 7200, 10800]
        #expect(UsageChartSupport.segments(times) == [0, 0, 0, 0])
    }

    @Test func neverJoinsAcrossMoreThanTwoHours() {
        let times: [Double] = [0, 9000, 18000]
        #expect(UsageChartSupport.segments(times) == [0, 1, 2])
    }

    @Test func singleAndEmptySeries() {
        #expect(UsageChartSupport.segments([]) == [])
        #expect(UsageChartSupport.segments([5]) == [0])
    }

    @Test func compactAxisFigures() {
        #expect(UsageChartSupport.compact(400_000_000) == "400M")
        #expect(UsageChartSupport.compact(1_200_000_000) == "1.2B")
        #expect(UsageChartSupport.compact(35_000) == "35K")
        #expect(UsageChartSupport.compact(900) == "900")
        #expect(UsageChartSupport.compact(0) == "0")
    }

    @Test func hookCommandShortened() {
        let cmd = "[ ! -f '/Users/me/.claude/skills/impeccable/scripts/impeccable' ] || '/Users/me/.claude/skills/impeccable/scripts/impeccable' hook"
        #expect(HookSummary.short(cmd, home: "/Users/me") == "impeccable hook")
        #expect(HookSummary.tilde(cmd, home: "/Users/me").contains("~/.claude/skills"))
        #expect(HookSummary.short("codegraph prompt-hook", home: "/Users/me") == "codegraph prompt-hook")
    }

    @Test func hookCommandSkipsFlagValuePaths() {
        let cmd = "'/Users/me/.caveman/bin/caveman-proxy' native-hook claude --adapter '/Users/me/lib/native-hook-fast.js' --node '/Users/me/bin/node'"
        #expect(HookSummary.short(cmd, home: "/Users/me") == "caveman-proxy native-hook claude")
    }
}

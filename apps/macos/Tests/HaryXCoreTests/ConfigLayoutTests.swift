import Testing
@testable import HaryXCore

@Suite struct ConfigLayoutTests {
    private func entry(_ key: String, _ value: JSONScalar = .bool(true)) -> ConfigEntry {
        ConfigEntry(key: key, value: value, isSet: false)
    }

    @Test func groupsCLIKeysByTaskWithoutAutoSwitch() {
        let entries = ["updates.check", "usage.fableProbe", "ui.theme", "history.keepDays", "autoswitch.threshold",
                       "usage.headerProbe", "run.shareHistory", "history.enabled", "ui.orgTag", "beta.flag"].map { entry($0) }
        let groups = ConfigLayout.groups(entries)
        #expect(groups.map(\.title) == ["Usage & menu bar", "Usage history", "Run", "Command line", "Beta"])
        #expect(groups[0].entries.map(\.key) == ["usage.headerProbe", "usage.fableProbe"])
        #expect(groups[1].entries.map(\.key) == ["history.enabled", "history.keepDays"])
        #expect(groups[3].entries.map(\.key) == ["ui.orgTag", "ui.theme", "updates.check"])
    }

    @Test func fableProbeWaitsForTheHeaderProbe() {
        let off = [entry("usage.headerProbe", .bool(false))]
        let on = [entry("usage.headerProbe", .bool(true))]
        #expect(ConfigLayout.isBlocked("usage.fableProbe", in: off))
        #expect(!ConfigLayout.isBlocked("usage.fableProbe", in: on))
        #expect(!ConfigLayout.isBlocked("usage.shareBetweenMachines", in: off))
    }

    @Test func choicesReadAsWords() {
        #expect(ConfigLayout.choiceLabel("history.import", "none") == "Skip them")
        #expect(ConfigLayout.choiceLabel("ui.theme", "auto") == "Auto")
        #expect(ConfigLayout.choiceLabel("autoswitch.strategy", "weekly-first") == "weekly-first")
    }
}

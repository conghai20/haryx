import Testing
@testable import HaryXCore

@Suite struct ProfileSummaryTests {
    private func item(_ id: String, size: UInt64 = 0, files: Int = 0, status: String = "ok") -> ProfileItem {
        ProfileItem(category: "hooks", id: id, kind: "file", size: size, files: files, status: status, detail: [])
    }

    @Test func summaryCountsItemsSizeAndProblems() {
        let items = [item("a", size: 2048, files: 1), item("b", size: 2048, files: 2), item("c", status: "broken")]
        #expect(ProfileInventory.summary(items) == "3 items · 4 KB · 1 needs attention")
        #expect(ProfileInventory.summary([item("x")]) == "1 item")
    }

    @Test func repeatedLinesCollapseWithACount() {
        let counted = ProfileInventory.counted(["node", "impeccable hook", "node", "node"])
        #expect(counted.map(\.line) == ["node", "impeccable hook"])
        #expect(counted.map(\.count) == [3, 1])
    }
}

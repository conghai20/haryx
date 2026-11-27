import HaryXCore
import SwiftUI

/// The Accounts toolbar's filter: by group (only once some account has one) and by rotation
/// status. The icon fills while a filter is on.
struct FilterMenu: View {
    @Environment(AppStore.self) private var store
    @Binding var filter: AccountFilter

    var body: some View {
        let names = AccountGroups.names(store.accounts)
        Menu {
            if !names.isEmpty {
                Picker("Group", selection: $filter.group) {
                    Text("All Groups").tag(AccountFilter.Group.all)
                    ForEach(names, id: \.self) { Text($0).tag(AccountFilter.Group.named($0.lowercased())) }
                    Text("Ungrouped").tag(AccountFilter.Group.ungrouped)
                }
                .pickerStyle(.inline)
            }
            Picker("Status", selection: $filter.status) {
                ForEach(AccountFilter.Status.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.inline)
            if filter.isActive {
                Divider()
                Button("Clear Filters") { filter = AccountFilter() }
            }
        } label: {
            Label("Filter", systemImage: filter.isActive ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .help("Show accounts by group or by status")
    }
}

/// Says what the filter hides, with a way out, so filtered-away accounts aren't mistaken for gone.
struct FilterBar: View {
    @Environment(AppStore.self) private var store
    @Binding var filter: AccountFilter
    let shown: Int
    let total: Int

    private var summary: String {
        var parts: [String] = []
        switch filter.group {
        case .all: break
        case .ungrouped: parts.append("Ungrouped")
        case .named(let key):
            parts.append(AccountGroups.names(store.accounts).first { $0.lowercased() == key } ?? key)
        }
        switch filter.status {
        case .all: break
        case .on: parts.append("On")
        case .off: parts.append("Off")
        }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "line.3.horizontal.decrease.circle.fill").foregroundStyle(.secondary)
            Text("\(summary): \(shown) of \(total) accounts").font(.callout)
            Spacer()
            Button("Clear") { filter = AccountFilter() }.controlSize(.small)
        }
        .accessibilityElement(children: .contain)
    }
}

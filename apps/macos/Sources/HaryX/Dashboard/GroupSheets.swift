import HaryXCore
import SwiftUI

/// Names a group and ticks the accounts that go in it. Opened from the Accounts toolbar's
/// New Group… button and from an account's Group menu (with that account ticked).
struct NewGroupSheet: View {
    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var selection: Set<Int>

    init(initial: Set<Int>) { _selection = State(initialValue: initial) }

    private var normalized: String? { AccountGroups.normalizedName(name, among: store.accounts) }
    private var joinsExisting: Bool {
        normalized.map { n in AccountGroups.names(store.accounts).contains(n) } ?? false
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("New Group").font(.title3.bold())
            Text("An account can be in several groups. Automatic switching tries accounts that share a group with the active one first, and leaves them only when none can take over.")
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Form {
                TextField("Name", text: $name, prompt: Text("e.g. Work"))
                    .onChange(of: name) { _, new in
                        if new.count > AccountGroups.maxNameLength { name = String(new.prefix(AccountGroups.maxNameLength)) }
                    }
                if joinsExisting, let normalized {
                    Text("Adds to the existing group \(normalized).").font(.caption).foregroundStyle(.secondary)
                }
                Section("Accounts") {
                    ForEach(store.accounts) { a in
                        Toggle(isOn: Binding(get: { selection.contains(a.number) },
                                             set: { on in if on { selection.insert(a.number) } else { selection.remove(a.number) } })) {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(a.displayName)
                                Text(a.groupNames.isEmpty ? "Slot \(a.number)" : "also in \(a.groupLabel)")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .formStyle(.grouped)
            .frame(maxHeight: 420)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Create") {
                    guard let normalized else { return }
                    let members = store.accounts.filter { selection.contains($0.number) }
                    Task {
                        await store.addToGroup(members, normalized)
                        dismiss()
                    }
                }
                .buttonStyle(.glassProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(normalized == nil || selection.isEmpty || store.busy != nil)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}

/// An account's groups in a context menu: every group in use, checked where the account is in it
/// (choosing one adds or removes it), a new one, or out of all of them.
struct GroupMenu: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        Menu(account.groupNames.isEmpty ? "Groups" : "Groups: \(account.groupLabel)") {
            let names = AccountGroups.names(store.accounts)
            ForEach(names, id: \.self) { name in
                Toggle(name, isOn: Binding(get: { account.isInGroup(name) },
                                           set: { on in
                                               Task {
                                                   if on { await store.addToGroup(account, name) }
                                                   else { await store.removeFromGroup(account, name) }
                                               }
                                           }))
            }
            if !names.isEmpty { Divider() }
            Button("New Group…") { DashboardRouter.shared.newGroup([account.number]) }
            if account.groupNames.count > 1 {
                Button("Remove from All Groups") { Task { await store.clearGroups(account) } }
            }
        }
        .disabled(store.busy != nil)
    }
}

/// Rename… and Delete Group… for one group: on its section header (right-click and the … button).
struct GroupActions: View {
    let name: String

    var body: some View {
        Button("Rename Group…") { DashboardRouter.shared.renamingGroup = name }
        Button("Delete Group…", role: .destructive) { DashboardRouter.shared.deletingGroup = name }
    }
}

/// Renames a group on every account in it. A name that matches another group merges the two.
struct RenameGroupSheet: View {
    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let name: String
    @State private var newName: String

    init(name: String) {
        self.name = name
        _newName = State(initialValue: name)
    }

    private var trimmed: String { newName.trimmingCharacters(in: .whitespaces) }
    /// Another group this name would merge into, in its spelling.
    private var mergesInto: String? {
        AccountGroups.names(store.accounts).first { $0.lowercased() == trimmed.lowercased() && $0.lowercased() != name.lowercased() }
    }
    private var memberCount: Int { store.accounts.filter { $0.isInGroup(name) }.count }
    private var canRename: Bool {
        !trimmed.isEmpty && trimmed.count <= AccountGroups.maxNameLength && trimmed != name && store.busy == nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Rename \(name)").font(.title3.bold())
            Text("Renames the group on its \(memberCount) account\(memberCount == 1 ? "" : "s").")
                .font(.callout).foregroundStyle(.secondary)
            Form {
                TextField("Name", text: $newName)
                    .onChange(of: newName) { _, new in
                        if new.count > AccountGroups.maxNameLength { newName = String(new.prefix(AccountGroups.maxNameLength)) }
                    }
                if let mergesInto {
                    Text("Merges into the existing group \(mergesInto).").font(.caption).foregroundStyle(Brand.warn)
                }
            }
            .formStyle(.grouped)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button(mergesInto == nil ? "Rename" : "Merge") {
                    Task {
                        await store.renameGroup(name, to: trimmed)
                        dismiss()
                    }
                }
                .buttonStyle(.glassProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(!canRename)
            }
        }
        .padding(20)
        .frame(width: 420)
    }
}


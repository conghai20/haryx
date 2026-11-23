import AppKit
import HaryXCore
import SwiftUI

/// The sign-in window: shows a background `haryx ai login` run, its sign-in link, and a field to
/// paste the code the sign-in page shows. One window per run.
@MainActor
final class LoginWindow: NSObject, NSWindowDelegate {
    private static var open: [UUID: LoginWindow] = [:]

    private let session: LoginSession
    private var window: NSWindow?

    private init(session: LoginSession) {
        self.session = session
    }

    static func show(_ session: LoginSession, store: AppStore) {
        let controller = LoginWindow(session: session)
        let host = NSHostingController(rootView: LoginView(session: session) { [weak controller] in
            controller?.window?.close()
        }.environment(store))
        let w = NSWindow(contentViewController: host)
        w.styleMask = [.titled, .closable, .fullSizeContentView]
        w.title = session.title
        w.setContentSize(NSSize(width: 520, height: 420))
        w.isReleasedWhenClosed = false
        w.delegate = controller
        w.center()
        controller.window = w
        open[session.id] = controller
        NSApp.activate()
        w.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        session.cancel() // closing the window abandons a sign-in still waiting
        Self.open[session.id] = nil
    }
}

private struct LoginView: View {
    let session: LoginSession
    let close: () -> Void
    @State private var input = ""
    @State private var showOutput = false
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if session.state.isRunning {
                if let url = session.transcript.url { linkBox(url) }
                inputBox
            } else {
                outcome
            }
            DisclosureGroup("Terminal output", isExpanded: $showOutput) { output }
                .font(.callout)
            Spacer(minLength: 0)
            footer
        }
        .padding(20)
        .frame(minWidth: 520, minHeight: 420)
    }

    private var header: some View {
        HStack(spacing: 10) {
            if session.state.isRunning { ProgressView().controlSize(.small) }
            VStack(alignment: .leading, spacing: 2) {
                Text(session.title).font(.title3.bold())
                Text("Claude Code is signing you in. Your browser should open; sign in there with your own account.")
                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func linkBox(_ url: URL) -> some View {
        GroupBox("Sign-in link") {
            VStack(alignment: .leading, spacing: 8) {
                Text(url.absoluteString)
                    .font(.caption.monospaced()).textSelection(.enabled)
                    .lineLimit(3).truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .leading)
                HStack {
                    Button("Open in Browser") { NSWorkspace.shared.open(url) }
                    Button(copied ? "Copied" : "Copy Link") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(url.absoluteString, forType: .string)
                        copied = true
                    }
                }
                .controlSize(.small)
                Text("Only open this link yourself. Never send it to someone else to sign in for you.")
                    .font(.caption2).foregroundStyle(.secondary)
            }
            .padding(4)
        }
    }

    private var inputBox: some View {
        let transcript = session.transcript
        let label = transcript.awaitingAnswer ? "Answer (y or n)"
            : "If the sign-in page shows a code, paste it here"
        return GroupBox(label) {
            HStack {
                TextField(transcript.awaitingAnswer ? "y / n" : "Code", text: $input)
                    .textFieldStyle(.roundedBorder)
                    .font(.body.monospaced())
                    .onSubmit(submit)
                Button("Paste") {
                    input = NSPasteboard.general.string(forType: .string)?
                        .trimmingCharacters(in: .whitespacesAndNewlines) ?? input
                }
                Button("Submit", action: submit)
                    .buttonStyle(.borderedProminent)
                    .disabled(input.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            .padding(4)
        }
    }

    private var outcome: some View {
        Group {
            switch session.state {
            case .succeeded(let message):
                Label(message, systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            case .failed(let message):
                Label(message, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange)
            case .cancelled:
                Label("Sign-in cancelled. Nothing was changed.", systemImage: "xmark.circle").foregroundStyle(.secondary)
            case .running:
                EmptyView()
            }
        }
        .font(.callout)
        .fixedSize(horizontal: false, vertical: true)
    }

    private var output: some View {
        ScrollViewReader { proxy in
            ScrollView {
                Text(session.transcript.text.isEmpty ? "Starting…" : session.transcript.text)
                    .font(.caption.monospaced()).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .id("end")
            }
            .frame(height: 140)
            .onChange(of: session.transcript.text) { proxy.scrollTo("end", anchor: .bottom) }
        }
    }

    private var footer: some View {
        HStack {
            Button("Copy Output") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(session.transcript.text, forType: .string)
            }
            Spacer()
            if session.state.isRunning {
                Button("Cancel", role: .cancel) { session.cancel() }
            } else {
                Button("Done", action: close).keyboardShortcut(.defaultAction)
            }
        }
    }

    private func submit() {
        let line = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !line.isEmpty else { return }
        session.send(line)
        input = ""
    }
}

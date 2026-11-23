import HaryXCore
import Foundation
import Observation

/// Hosts `cswap auto --json` as a child process and streams its events.
// ponytail: the child is stopped on normal quit only; a force-killed app leaves it running
// until its next poll fails — add a process-group watchdog if that ever bites.
@MainActor @Observable
final class AutoSwitchRunner {
    static let maxEvents = 300
    static let stderrTail = 600

    private(set) var isRunning = false
    private(set) var events: [AutoEvent] = []
    private(set) var lastPoll: Date?
    private(set) var lastExit: String?
    var onEvent: ((AutoEvent) -> Void)?

    @ObservationIgnored private var process: Process?
    @ObservationIgnored private var buffer = Data()
    @ObservationIgnored private var errTail = Data()
    @ObservationIgnored private var stopping = false
    /// Finish only once stdout hit EOF *and* the process exited, so trailing events aren't lost.
    @ObservationIgnored private var stdoutClosed = false
    @ObservationIgnored private var exitStatus: Int32?

    func start(cli: CSwapCLI, dryRun: Bool) {
        guard process == nil else { return }
        let p = Process()
        p.executableURL = cli.executable
        p.arguments = ["auto", "--json"] + (dryRun ? ["--dry-run"] : [])
        // If HaryX dies without stopping us (killed, crashed), haryx ends its loop instead of
        // auto-switching unattended forever. cswap ignores the variable.
        p.environment = CSwapCLI.environment.merging(["HARYX_EXIT_WITH_PARENT": "1"]) { _, new in new }
        p.standardInput = FileHandle.nullDevice
        let out = Pipe(), err = Pipe()
        p.standardOutput = out
        p.standardError = err

        // Hop to main via GCD (FIFO) so chunks are consumed in order.
        out.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            if data.isEmpty { handle.readabilityHandler = nil } // EOF: stop the handler spinning
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.consume(data) } }
        }
        // Drain stderr continuously; a full pipe would block the child mid-poll.
        err.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            if data.isEmpty { handle.readabilityHandler = nil; return }
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.keepTail(data) } }
        }
        p.terminationHandler = { [weak self] proc in
            let status = proc.terminationStatus
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.exitStatus = status; self?.finishIfDone() } }
        }

        buffer.removeAll()
        errTail.removeAll()
        stdoutClosed = false
        exitStatus = nil
        stopping = false
        do {
            try p.run()
            process = p
            isRunning = true
            lastExit = nil
            append(AutoEvent(kind: "started", summary: dryRun ? "Auto-switch started (dry run)" : "Auto-switch started", severity: .notable))
        } catch {
            out.fileHandleForReading.readabilityHandler = nil
            err.fileHandleForReading.readabilityHandler = nil
            lastExit = "Could not start: \(error.localizedDescription)"
        }
    }

    func stop() {
        guard let process else { return }
        stopping = true
        process.terminate() // SIGTERM: cswap stops its loop cleanly
    }

    /// For callers that must not overlap with the child (e.g. purge).
    func stopAndWait(timeout: Duration = .seconds(5)) async {
        stop()
        let deadline = ContinuousClock.now + timeout
        while isRunning, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(100))
        }
    }

    private func consume(_ data: Data) {
        guard !data.isEmpty else {
            stdoutClosed = true
            finishIfDone()
            return
        }
        buffer.append(data)
        while let nl = buffer.firstIndex(of: 0x0A) {
            let line = String(decoding: buffer[buffer.startIndex..<nl], as: UTF8.self)
            buffer.removeSubrange(buffer.startIndex...nl)
            guard let event = AutoEvent.parse(line: line) else { continue }
            if event.kind == "poll" { lastPoll = event.date }
            append(event)
            onEvent?(event)
        }
    }

    private func keepTail(_ data: Data) {
        errTail.append(data)
        if errTail.count > Self.stderrTail { errTail = errTail.suffix(Self.stderrTail) }
    }

    private func append(_ event: AutoEvent) {
        events.insert(event, at: 0)
        if events.count > Self.maxEvents { events.removeLast(events.count - Self.maxEvents) }
    }

    private func finishIfDone() {
        guard stdoutClosed, let status = exitStatus, process != nil else { return }
        process = nil
        isRunning = false
        if stopping || status == 0 || status == SIGTERM {
            lastExit = nil
            append(AutoEvent(kind: "stopped", summary: "Auto-switch stopped", severity: .notable))
        } else {
            let stderr = String(decoding: errTail, as: UTF8.self)
            let message = stderr.split(separator: "\n").last.map(String.init) ?? "exit \(status)"
            lastExit = "Auto-switch exited (\(status)): \(message)"
            let event = AutoEvent(kind: "error", summary: lastExit ?? "", severity: .problem, notificationTitle: "Auto-switch stopped")
            append(event)
            onEvent?(event)
        }
    }
}

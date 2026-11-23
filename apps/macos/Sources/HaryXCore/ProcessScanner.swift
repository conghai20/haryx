import Darwin
import Foundation

public struct ClaudeProcess: Sendable, Hashable, Identifiable {
    public let pid: Int32
    public let configDir: String?
    public let cwd: String?
    public let startedAt: Date
    public var id: Int32 { pid }
}

/// Finds live Claude Code processes natively (no `ps` fork): libproc for the cheap path filter,
/// then KERN_PROCARGS2 for argv + environment of the few candidates. Same-user processes only.
public enum ProcessScanner {
    /// Helper modes that are not "Claude working on something".
    static let ignoredArgs: Set<String> = ["--chrome-native-host", "mcp", "--version", "-v", "doctor", "update"]

    /// nil when the process table can't be read, so callers don't mistake it for "all sessions ended".
    public static func scan() -> [ClaudeProcess]? {
        let count = proc_listallpids(nil, 0)
        guard count > 0 else { return nil }
        var pids = [Int32](repeating: 0, count: Int(count) + 32)
        let filled = pids.withUnsafeMutableBytes { proc_listallpids($0.baseAddress, Int32($0.count)) }
        guard filled > 0 else { return nil }

        let me = getpid()
        var found: [ClaudeProcess] = []
        var pathBuf = [UInt8](repeating: 0, count: Int(MAXPATHLEN) * 4)
        for pid in pids.prefix(Int(filled)) where pid > 0 && pid != me {
            guard proc_pidpath(pid, &pathBuf, UInt32(pathBuf.count)) > 0 else { continue }
            let path = String(decoding: pathBuf.prefix(while: { $0 != 0 }), as: UTF8.self)
            let isNode = path.hasSuffix("/node")
            guard isNode || isClaudeBinary(path) else { continue }
            guard let (args, env) = procArgs(pid) else { continue }
            if isNode && !args.dropFirst().prefix(2).contains(where: { $0.contains("claude-code") || $0.hasSuffix("/claude") }) { continue }
            // Only the subcommand position marks a helper mode; `claude -p "update"` is real work.
            if args.count > 1, ignoredArgs.contains(args[1]) { continue }
            found.append(ClaudeProcess(pid: pid, configDir: env["CLAUDE_CONFIG_DIR"], cwd: env["PWD"],
                                       startedAt: startTime(pid) ?? Date()))
        }
        return found
    }

    /// Native installs live at `~/.local/share/claude/versions/<ver>`; others are a `claude` binary.
    public static func isClaudeBinary(_ path: String) -> Bool {
        path.contains("/claude/versions/") || path.hasSuffix("/claude")
    }

    /// Which cswap slot a process is using: `cswap run N` sets CLAUDE_CONFIG_DIR to
    /// `<backup>/sessions/<N>-<slug>`; no override (or ~/.claude) means the active login.
    public static func slot(forConfigDir dir: String?, backupRoot: String, home: String, activeSlot: Int?) -> Int? {
        guard let dir, !dir.isEmpty else { return activeSlot }
        let normalized = (dir as NSString).standardizingPath
        if normalized == (home as NSString).appendingPathComponent(".claude") { return activeSlot }
        let sessions = (backupRoot as NSString).appendingPathComponent("sessions") + "/"
        guard normalized.hasPrefix(sessions) else { return nil }
        let leaf = normalized.dropFirst(sessions.count).split(separator: "/").first ?? ""
        return leaf.split(separator: "-", maxSplits: 1).first.flatMap { Int($0) }
    }

    /// KERN_PROCARGS2 layout: argc (Int32) · exec path · NUL padding · argv[argc] · env strings.
    public static func procArgs(_ pid: Int32) -> ([String], [String: String])? {
        var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
        var size = 0
        guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > MemoryLayout<Int32>.size else { return nil }
        var buf = [UInt8](repeating: 0, count: size)
        guard sysctl(&mib, 3, &buf, &size, nil, 0) == 0 else { return nil }
        return parseProcArgs(Array(buf.prefix(size)))
    }

    public static func parseProcArgs(_ buf: [UInt8]) -> ([String], [String: String])? {
        guard buf.count > 4 else { return nil }
        let argc = buf.withUnsafeBytes { $0.loadUnaligned(as: Int32.self) }
        var i = 4
        while i < buf.count && buf[i] != 0 { i += 1 } // exec path
        while i < buf.count && buf[i] == 0 { i += 1 } // padding
        func next() -> String? {
            guard i < buf.count else { return nil }
            let start = i
            while i < buf.count && buf[i] != 0 { i += 1 }
            defer { i += 1 }
            return String(decoding: buf[start..<i], as: UTF8.self)
        }
        // argv entries may legitimately be empty (`claude -p ""`), so read exactly argc of them.
        var args: [String] = []
        for _ in 0..<max(0, Int(argc)) {
            guard let s = next() else { break }
            args.append(s)
        }
        var env: [String: String] = [:]
        while let kv = next(), !kv.isEmpty { // an empty string ends the env block
            guard let eq = kv.firstIndex(of: "=") else { continue }
            env[String(kv[..<eq])] = String(kv[kv.index(after: eq)...])
        }
        return (args, env)
    }

    static func startTime(_ pid: Int32) -> Date? {
        var info = proc_bsdinfo()
        let size = Int32(MemoryLayout<proc_bsdinfo>.size)
        guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(info.pbi_start_tvsec) + TimeInterval(info.pbi_start_tvusec) / 1e6)
    }
}

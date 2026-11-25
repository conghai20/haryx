import Foundation
import Testing
@testable import HaryXCore

private let notReadyJSON = """
{
  "ready": false,
  "claude": null,
  "signedIn": false,
  "email": null,
  "configPath": "/Users/x/.claude.json",
  "installCommand": "curl -fsSL https://claude.ai/install.sh | bash",
  "problems": [
    {"code": "claude_missing", "message": "Claude Code is not installed, or haryx cannot find it.",
     "fix": "Install it with `curl -fsSL https://claude.ai/install.sh | bash`."},
    {"code": "not_signed_in", "message": "Claude Code has not signed in yet.",
     "fix": "Run `claude` in a terminal and sign in once."}
  ]
}
"""

@Test func doctorReportDecodesAndLeadsWithTheFirstProblem() throws {
    let setup = try CSwapJSON.decoder.decode(ClaudeSetup.self, from: Data(notReadyJSON.utf8))
    #expect(!setup.ready)
    #expect(setup.next?.kind == .claudeMissing)
    #expect(setup.installCommand.contains("install.sh"))
}

@Test func signedOutReportPointsAtSigningIn() throws {
    let json = """
    {"ready": false, "claude": "/x/claude", "signedIn": false, "email": null,
     "configPath": "/x/.claude.json", "installCommand": "curl -fsSL https://claude.ai/install.sh | bash",
     "problems": [{"code": "not_signed_in", "message": "m", "fix": "f"}]}
    """
    let setup = try CSwapJSON.decoder.decode(ClaudeSetup.self, from: Data(json.utf8))
    #expect(setup.next?.kind == .notSignedIn)
}

@Test func loginShellPathIgnoresWhatRcFilesPrint() {
    let noisy = "Welcome back!\n\u{1B}[32mnvm ok\u{1B}[0m\n__HARYX_PATH__/a/bin:/b/bin__HARYX_END__\nbye\n"
    #expect(CSwapCLI.extractShellPath(noisy) == "/a/bin:/b/bin")
    #expect(CSwapCLI.extractShellPath("no marker here") == nil)
    #expect(CSwapCLI.extractShellPath("__HARYX_PATH____HARYX_END__") == nil)
}

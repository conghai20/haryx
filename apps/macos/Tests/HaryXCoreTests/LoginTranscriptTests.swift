import Foundation
import Testing
@testable import HaryXCore

/// Captured from `claude auth login` on a pseudo-terminal (ESC/BEL written out).
private let claudeLoginOutput =
    "\u{04}\u{08}\u{08}Opening browser to sign in…\r\n"
    + "If the browser didn't open, visit:\r\n"
    + "\u{1B}]8;;https://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com\u{07}"
    + "\u{1B}[94mhttps://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com\u{1B}[39m\u{1B}]8;;\u{07}\r\n"
    + "Paste code here if prompted > "

@Test func cleansClaudeLoginOutputAndFindsLink() {
    var t = LoginTranscript()
    t.append(claudeLoginOutput)
    #expect(t.text == """
    Opening browser to sign in…
    If the browser didn't open, visit:
    https://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com
    Paste code here if prompted > 
    """)
    #expect(t.url?.absoluteString == "https://claude.ai/oauth/authorize?code=true&login_hint=a%40x.com")
    #expect(t.awaitingCode)
}

@Test func escapeSequenceSplitAcrossChunks() {
    var t = LoginTranscript()
    t.append("a\u{1B}[3")
    t.append("9mb\r\n")
    #expect(t.text == "ab\n")
}

@Test func ptyDoubleCarriageReturnKeepsTheLine() {
    #expect(LoginTranscript.clean("visit:\r\r\nhttps://x\r\r\n") == "visit:\nhttps://x\n")
}

@Test func bareCarriageReturnOverwritesTheLine() {
    #expect(LoginTranscript.clean("one\nwait 1\rwait 2\r\n") == "one\nwait 2\n")
}

@Test func reportsHaryxOutcome() {
    var ok = LoginTranscript()
    ok.append("Signed in Account 3: a@x.com [Team]\r\n")
    #expect(ok.successMessage == "Signed in Account 3: a@x.com [Team]")

    var failed = LoginTranscript()
    failed.append("Paste code here if prompted > \r\nError: You signed in as b@x.com, but Account-3 is a@x.com.\r\n")
    #expect(failed.failureMessage == "You signed in as b@x.com, but Account-3 is a@x.com.")
    #expect(!failed.awaitingCode)
}

@Test func detectsOverwriteQuestion() {
    var t = LoginTranscript()
    t.append("Slot 2 already occupied\r\nOverwrite slot 2? [y/N] ")
    #expect(t.awaitingAnswer)
}

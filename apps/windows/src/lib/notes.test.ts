import { describe, expect, it } from "vitest";
import { blocks, codeSpans } from "./notes";

describe("release notes", () => {
  it("reads headings, wrapped bullets and paragraphs", () => {
    const md = "### Added\n\n- HaryX for Windows: a tray app\n  over the CLI.\n- `haryx upgrade --check`\n\nSee the README\nfor more.";
    expect(blocks(md)).toEqual([
      { kind: "heading", text: "Added" },
      { kind: "bullet", text: "HaryX for Windows: a tray app over the CLI." },
      { kind: "bullet", text: "`haryx upgrade --check`" },
      { kind: "paragraph", text: "See the README for more." },
    ]);
  });

  it("is empty for empty notes", () => {
    expect(blocks("")).toEqual([]);
  });

  it("splits code spans", () => {
    expect(codeSpans("run `haryx upgrade` now")).toEqual(["run ", "haryx upgrade", " now"]);
  });
});

import { describe, expect, it } from "vitest";
import { describeError } from "./errors";

describe("describeError", () => {
  it("keeps the CLI's own first line as a sentence, with the raw text as detail", () => {
    const d = describeError("error: no account matches 9\nhint: run haryx list", "Switching failed");
    expect(d.text).toBe("Switching failed. No account matches 9.");
    expect(d.detail).toContain("hint: run haryx list");
  });

  it("names known failures in plain words", () => {
    expect(describeError(new Error("program not found")).text).toMatch(/couldn't find its haryx command-line tool/);
  });

  it("never shows an empty message", () => {
    expect(describeError("").text).toBe("Something went wrong.");
  });
});

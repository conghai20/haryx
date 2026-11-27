import { describe, expect, it } from "vitest";
import { otherSections, ruleEntries, settingText, type ConfigEntry } from "./settings";

const entry = (key: string, value: ConfigEntry["value"] = null): ConfigEntry => ({ key, value, isSet: false });

describe("settings", () => {
  it("names settings in plain words and keeps unknown keys as they are", () => {
    expect(settingText("autoswitch.hysteresisPct").label).toBe("Headroom margin");
    expect(settingText("autoswitch.hysteresisPct").unit).toBe("pts");
    expect(settingText("brand.new").label).toBe("brand.new");
  });

  it("orders the auto-switch rules when, how often, which account; unknown rules last", () => {
    const keys = ruleEntries([
      entry("autoswitch.model"),
      entry("ui.theme"),
      entry("autoswitch.future"),
      entry("autoswitch.threshold"),
      entry("autoswitch.strategy"),
    ]).map((e) => e.key);
    expect(keys).toEqual(["autoswitch.threshold", "autoswitch.strategy", "autoswitch.model", "autoswitch.future"]);
  });

  it("groups the other settings by section, alphabetically, without the rules", () => {
    const sections = otherSections([
      entry("usage.headerProbe"),
      entry("autoswitch.threshold"),
      entry("history.enabled"),
      entry("ui.theme"),
      entry("usage.fableProbe"),
    ]);
    expect(sections.map((s) => s.title)).toEqual(["Usage history", "Command-line display", "Usage readings"]);
    expect(sections[2]?.entries.map((e) => e.key)).toEqual(["usage.headerProbe", "usage.fableProbe"]);
  });
});

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

  it("groups the other settings by task, without the rules; newer groups last", () => {
    const sections = otherSections([
      entry("usage.fableProbe"),
      entry("updates.check"),
      entry("brand.new"),
      entry("autoswitch.threshold"),
      entry("history.enabled"),
      entry("ui.theme"),
      entry("usage.headerProbe"),
    ]);
    expect(sections.map((s) => s.title)).toEqual(["Usage & tray", "Usage history", "Command line", "Brand"]);
    // The switch a setting needs comes before it, whatever the key order.
    expect(sections[0]?.entries.map((e) => e.key)).toEqual(["usage.headerProbe", "usage.fableProbe"]);
    expect(sections[2]?.entries.map((e) => e.key)).toEqual(["ui.theme", "updates.check"]);
  });

  it("names choice values in words and records what a setting needs", () => {
    expect(settingText("history.import").choiceLabels?.none).toBe("Skip them");
    expect(settingText("usage.fableProbe").needs).toBe("usage.headerProbe");
  });
});

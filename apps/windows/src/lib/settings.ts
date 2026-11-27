// haryx's own settings (`haryx config`) in plain words. The dotted key is what the CLI and the
// settings file use; people see the label and caption, and the key only in a control's tooltip.
// The same words as the macOS app's ConfigRow.

export type SettingValue = string | number | boolean | null;

export interface ConfigEntry {
  key: string;
  value: SettingValue;
  isSet: boolean;
}

export interface SettingText {
  label: string;
  caption?: string;
  /** Shown after a number field. */
  unit?: string;
  /** A fixed set of values: shown as a picker. */
  choices?: string[];
  /** The word a picker shows for a value; the value itself otherwise. */
  choiceLabels?: Record<string, string>;
  /** A switch that does nothing while this other on/off setting is off: shown disabled then. */
  needs?: string;
}

export const SETTING_TEXT: Record<string, SettingText> = {
  "autoswitch.threshold": {
    label: "Switch away at",
    unit: "%",
    caption: "When the active account's 5-hour window reaches this. 50–99.9.",
  },
  "autoswitch.weeklyThreshold": {
    label: "Weekly switch line",
    unit: "%",
    caption: "For weekly and per-model limits. Blank uses the line above.",
  },
  "autoswitch.intervalSeconds": { label: "Check every", unit: "s", caption: "How often auto-switch reads usage. 15–3600." },
  "autoswitch.cooldownSeconds": { label: "Wait between switches", unit: "s", caption: "0–86400." },
  "autoswitch.hysteresisPct": {
    label: "Headroom margin",
    unit: "pts",
    caption: "Only switch to an account with this many more points of headroom. 0–50.",
  },
  "autoswitch.strategy": {
    label: "Pick the next account by",
    choices: ["best", "weekly-first", "consume-first", "priority-consume-first"],
  },
  "autoswitch.includeApiKeyAccounts": { label: "Include API-key accounts", caption: "They bill per token." },
  "autoswitch.unhealthyTicks": { label: "Failed checks before quarantine" },
  "autoswitch.model": {
    label: "Also watch model limits",
    caption: "Per-model weekly limits to count, e.g. Fable, Fable,Opus or all.",
  },
  "usage.headerProbe": { label: "Read setup-token usage with a 1-token request", caption: "Uses a little quota." },
  "usage.fableProbe": {
    label: "Also read Fable's weekly limit",
    caption: "Needs the setting above. Uses a little Fable quota.",
    needs: "usage.headerProbe",
  },
  "usage.shareBetweenMachines": {
    label: "Share usage readings with your other machines",
    caption: "Through HaryX sync, so each account spends one usage budget.",
  },
  "run.shareHistory": { label: "Share one chat history across accounts", caption: "For haryx ai run." },
  "updates.check": { label: "Check for updates daily" },
  "ui.theme": {
    label: "Terminal colour theme",
    caption: "Auto follows the terminal background.",
    choices: ["auto", "dark", "light"],
    choiceLabels: { auto: "Auto", dark: "Dark", light: "Light" },
  },
  "ui.orgTag": {
    label: "Show organisation tag",
    caption: "Auto shows it only where accounts share an email.",
    choices: ["auto", "on", "off"],
    choiceLabels: { auto: "Auto", on: "On", off: "Off" },
  },
  "history.enabled": {
    label: "Keep a usage history",
    caption: "From Claude Code's transcripts on this PC. No conversation text is stored.",
  },
  "history.import": {
    label: "Past transcripts on first read",
    choices: ["all", "none", "ask"],
    choiceLabels: { ask: "Ask", all: "Import all", none: "Skip them" },
  },
  "history.keepDays": { label: "Keep usage history for", unit: "days", caption: "0 keeps everything." },
};

/** The Auto-switch rules in the order they read best: when, how often, which account (as macOS). */
export const RULE_ORDER = [
  "autoswitch.threshold",
  "autoswitch.weeklyThreshold",
  "autoswitch.strategy",
  "autoswitch.hysteresisPct",
  "autoswitch.intervalSeconds",
  "autoswitch.cooldownSeconds",
  "autoswitch.unhealthyTicks",
  "autoswitch.includeApiKeyAccounts",
  "autoswitch.model",
];

/** The other settings in the order they read best: a switch before the one that needs it (as macOS). */
export const KEY_ORDER = [
  "usage.headerProbe",
  "usage.fableProbe",
  "usage.shareBetweenMachines",
  "history.enabled",
  "history.import",
  "history.keepDays",
];

/** Which section of AI settings a key's first word lands in: ui and updates share "Command line". */
const SECTION_OF: Record<string, string> = { usage: "usage", history: "history", run: "run", ui: "commandLine", updates: "commandLine" };

/** The known sections, in page order; usage's rows sit under the app's "Usage & tray" heading. */
export const SECTION_ORDER = ["usage", "history", "run", "commandLine"];

/** Section titles for the settings outside auto-switch. */
export const SECTION_TITLES: Record<string, string> = {
  usage: "Usage & tray",
  history: "Usage history",
  run: "Run",
  commandLine: "Command line",
};

/** The words for a key; an unknown key (a newer CLI) shows the key itself. */
export function settingText(key: string): SettingText {
  return SETTING_TEXT[key] ?? { label: key };
}

export const isAutoSwitchKey = (key: string) => key.startsWith("autoswitch.");

/** The auto-switch rules, in RULE_ORDER; keys a newer CLI adds go last. */
export function ruleEntries(entries: ConfigEntry[]): ConfigEntry[] {
  const rank = (k: string) => {
    const i = RULE_ORDER.indexOf(k);
    return i === -1 ? RULE_ORDER.length : i;
  };
  return entries.filter((e) => isAutoSwitchKey(e.key)).sort((a, b) => rank(a.key) - rank(b.key));
}

export interface SettingSection {
  section: string;
  title: string;
  entries: ConfigEntry[];
}

/** Known keys in KEY_ORDER, the rest after them in key order. */
function byKeyOrder(a: ConfigEntry, b: ConfigEntry): number {
  const rank = (k: string) => {
    const i = KEY_ORDER.indexOf(k);
    return i === -1 ? KEY_ORDER.length : i;
  };
  return rank(a.key) - rank(b.key) || a.key.localeCompare(b.key);
}

/**
 * Every other setting by section: the known ones in SECTION_ORDER, then a newer CLI's unknown groups
 * alphabetically, titled by their first word (as macOS).
 */
export function otherSections(entries: ConfigEntry[]): SettingSection[] {
  const bySection = new Map<string, ConfigEntry[]>();
  for (const e of entries) {
    if (isAutoSwitchKey(e.key)) continue;
    const word = e.key.split(".")[0] ?? "";
    const section = SECTION_OF[word] ?? word;
    bySection.set(section, [...(bySection.get(section) ?? []), e]);
  }
  const rank = (s: string) => {
    const i = SECTION_ORDER.indexOf(s);
    return i === -1 ? SECTION_ORDER.length : i;
  };
  return [...bySection.keys()]
    .sort((a, b) => rank(a) - rank(b) || a.localeCompare(b))
    .map((section) => ({
      section,
      title: SECTION_TITLES[section] ?? `${section.charAt(0).toUpperCase()}${section.slice(1)}`,
      entries: [...(bySection.get(section) ?? [])].sort(byKeyOrder),
    }));
}

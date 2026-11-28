// The signed-in sections, in tab order. The hash names the section (#usage); an unknown hash opens
// Accounts.
export const SECTIONS = [
  { id: "accounts", label: "Accounts", needsUnlock: true },
  { id: "usage", label: "Usage", needsUnlock: true },
  { id: "profiles", label: "Profiles", needsUnlock: true },
  { id: "machines", label: "Machines", needsUnlock: false },
  { id: "privacy", label: "Privacy", needsUnlock: false },
] as const;

export type Section = (typeof SECTIONS)[number];
export type SectionId = Section["id"];

export function sectionFromHash(hash: string): SectionId {
  return SECTIONS.find((s) => s.id === hash.replace(/^#/, ""))?.id ?? "accounts";
}

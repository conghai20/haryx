// Moving between dashboard panes from inside a pane ("Turn on history" opens AI settings, and so on).
// The dashboard listens for this event; panes never import the dashboard.

export const OPEN_PANE_EVENT = "haryx-open-pane";

/** Pane ids, as the dashboard's sidebar knows them. */
export type PaneId =
  | "accounts"
  | "usage"
  | "sessions"
  | "auto"
  | "wake"
  | "mappings"
  | "profiles"
  | "settings"
  | "tools"
  | "account"
  | "preferences"
  | "about";

export function openPane(id: PaneId): void {
  window.dispatchEvent(new CustomEvent(OPEN_PANE_EVENT, { detail: id }));
}

export const REVEAL_ACCOUNT_EVENT = "haryx-reveal-account";

/** Open Accounts with `slot`'s details (Usage's week rows). */
export function revealAccount(slot: number): void {
  window.dispatchEvent(new CustomEvent(REVEAL_ACCOUNT_EVENT, { detail: slot }));
}

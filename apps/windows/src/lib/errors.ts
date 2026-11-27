// Turns a thrown value into words for the error banner: a plain cause and, when one is known, what to
// do about it. The raw text stays available as the banner's detail (its tooltip), never as the message.

export interface Described {
  /** One plain sentence (or two: the cause, then the next step). */
  text: string;
  /** The raw error, for the tooltip and for support. */
  detail: string;
}

/** Known failures, matched on the raw text, with the words a person needs. */
const KNOWN: [RegExp, string][] = [
  [/program not found|cannot find the file|no such file.*haryx|haryx.*not found/i,
    "HaryX couldn't find its haryx command-line tool. Reinstall HaryX, or check the path under Backup & maintenance."],
  [/timed? ?out/i, "The haryx command took too long to answer. Try again in a moment."],
  [/permission denied|access is denied/i, "Windows refused access to a file HaryX needs. Check the file isn't open elsewhere, then try again."],
  [/network|connection|dns|unreachable|offline/i, "HaryX couldn't reach the network. Check your connection, then try again."],
  [/rate.?limit|http.?429/i, "The usage service is rate-limiting requests. HaryX will try again on its next check."],
];

/** The raw text of anything thrown: an Error's message, a string, or its JSON. */
export function rawText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

/** A cause and next step for `e`; `what` names the action that failed ("Couldn't save the setting"). */
export function describeError(e: unknown, what?: string): Described {
  const detail = rawText(e).trim();
  const known = KNOWN.find(([re]) => re.test(detail))?.[1];
  // The CLI's own errors are already sentences ("error: no account matches 9"): keep the first line.
  const firstLine = detail.split(/\r?\n/)[0]?.replace(/^(error|Error):\s*/, "").trim() ?? "";
  const cause = known ?? (firstLine ? `${firstLine.charAt(0).toUpperCase()}${firstLine.slice(1)}` : "Something went wrong.");
  const sentence = /[.!?]$/.test(cause) ? cause : `${cause}.`;
  return { text: what ? `${what}. ${sentence}` : sentence, detail };
}

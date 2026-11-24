// Release notes (Markdown rendered from CHANGELOG.md) as plain blocks the UI draws without an HTML
// renderer: `###` headings, `-` bullets with their wrapped continuation lines, and paragraphs.
// Inline code spans are split out so they can be styled; nothing is ever inserted as HTML.

export type Block =
  | { kind: "heading"; text: string }
  | { kind: "bullet"; text: string }
  | { kind: "paragraph"; text: string };

export function blocks(markdown: string): Block[] {
  const out: Block[] = [];
  for (const raw of markdown.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const last = out[out.length - 1];
    if (line.startsWith("#")) {
      out.push({ kind: "heading", text: line.replace(/^#+\s*/, "") });
    } else if (line.startsWith("- ") || line.startsWith("* ")) {
      out.push({ kind: "bullet", text: line.slice(2) });
    } else if (raw.startsWith(" ") && last?.kind === "bullet") {
      out[out.length - 1] = { ...last, text: `${last.text} ${line}` };
    } else if (last?.kind === "paragraph") {
      out[out.length - 1] = { ...last, text: `${last.text} ${line}` };
    } else {
      out.push({ kind: "paragraph", text: line });
    }
  }
  return out;
}

/** Split text on `code` spans: odd indexes are code. */
export function codeSpans(text: string): string[] {
  return text.split("`");
}

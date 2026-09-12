/**
 * Knowledge "link" content is agent-supplied and stored verbatim — the server
 * does no URL validation, so the component decides what renders as an anchor.
 * React 19 already blocks `javascript:` hrefs at render time; this allowlist
 * keeps every other scheme (`data:`, `vbscript:`, scheme-less text) from
 * rendering as a clickable link at all. The URL parser strips tabs and
 * newlines exactly like browsers do, so obfuscated `java\nscript:` input
 * still fails the allowlist.
 */
export function isHttpUrl(content: string): boolean {
  try {
    const url = new URL(content);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

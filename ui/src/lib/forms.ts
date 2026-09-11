import type { ZodError } from "zod";

/**
 * Flattens zod issues into a field → message map compatible with what
 * `fieldErrors()` produces for server 422s, so forms render both the same way.
 * Issues without a path land under `_form`.
 */
export function zodFieldErrors(error: ZodError): Record<string, string> {
  const map: Record<string, string> = {};
  for (const issue of error.issues) {
    const key = issue.path.join(".") || "_form";
    if (!(key in map)) map[key] = issue.message;
  }
  return map;
}

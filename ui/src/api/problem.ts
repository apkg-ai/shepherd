/**
 * RFC 9457 problem+json error domain (docs/03-api.md).
 *
 * Every server error carries a `ProblemDetail` body with a stable
 * `urn:shepherd:error:*` type slug. `shepherdFetch` (client.ts) normalizes
 * those into `ShepherdError` so screens can switch on `errorSlug()` and forms
 * can map 422 `errors[]` onto fields via `fieldErrors()`.
 */

/** A single field-level validation error (422 responses). */
export interface ValidationErrorDetail {
  /** JSON Pointer to the invalid field, e.g. `/title`. */
  field: string;
  message: string;
  code?: string;
}

/** RFC 9457 problem detail as served by shepherd. */
export interface ProblemDetail {
  /** Stable URN, e.g. `urn:shepherd:error:dependency-cycle`. */
  type: string;
  title: string;
  status: number;
  detail?: string;
  instance?: string;
  errors?: ValidationErrorDetail[];
}

export class ShepherdError extends Error {
  readonly type: string;
  readonly title: string;
  readonly status: number;
  readonly detail?: string;
  readonly errors?: ValidationErrorDetail[];

  constructor(problem: ProblemDetail) {
    super(problem.detail ?? problem.title);
    this.name = "ShepherdError";
    this.type = problem.type;
    this.title = problem.title;
    this.status = problem.status;
    this.detail = problem.detail;
    this.errors = problem.errors;
  }
}

export function isShepherdError(err: unknown): err is ShepherdError {
  return err instanceof ShepherdError;
}

/** Strips the URN prefix: `urn:shepherd:error:claim-conflict` → `claim-conflict`. */
export function errorSlug(err: ShepherdError): string {
  return err.type.replace(/^urn:shepherd:error:/, "");
}

/**
 * Maps 422 validation errors onto form fields.
 * JSON Pointer `/title` → `title`; nested pointers keep their tail
 * (`/settings/review_gate` → `settings.review_gate`).
 */
export function fieldErrors(err: ShepherdError): Record<string, string> {
  const map: Record<string, string> = {};
  for (const e of err.errors ?? []) {
    const key = e.field.replace(/^\//, "").replaceAll("/", ".");
    if (!(key in map)) map[key] = e.message;
  }
  return map;
}

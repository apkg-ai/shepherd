/**
 * RFC 9457 problem+json domain: server errors carry a ProblemDetail with a
 * stable `urn:shepherd:error:*` type slug.
 */

export interface ValidationErrorDetail {
  /** JSON Pointer to the invalid field, e.g. `/title`. */
  field: string;
  message: string;
  code?: string;
}

export interface ProblemDetail {
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

export function errorSlug(err: ShepherdError): string {
  return err.type.replace(/^urn:shepherd:error:/, "");
}

// JSON Pointer `/settings/review_gate` → `settings.review_gate`.
export function fieldErrors(err: ShepherdError): Record<string, string> {
  const map: Record<string, string> = {};
  for (const e of err.errors ?? []) {
    const key = e.field.replace(/^\//, "").replace(/\//g, ".");
    if (!(key in map)) map[key] = e.message;
  }
  return map;
}

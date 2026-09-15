import { setupServer } from "msw/node";
import { HttpResponse } from "msw";
import { getSystemMock } from "../api/generated/system/system.msw";
import type { ProblemDetail, ValidationErrorDetail } from "../api/problem";

/**
 * Spec-derived MSW server. Generated handlers answer any request with
 * spec-shaped faker data as background noise; tests seed the responses they
 * assert on with `server.use(get<Op>MockHandler(fixture))`.
 */
export const server = setupServer(...getSystemMock());

/** Builds a problem+json response for error-path tests. */
export function problemResponse(
  status: number,
  slug: string,
  options: { title?: string; detail?: string; errors?: ValidationErrorDetail[] } = {},
) {
  const problem: ProblemDetail = {
    type: `urn:shepherd:error:${slug}`,
    title: options.title ?? slug,
    status,
    ...(options.detail !== undefined && { detail: options.detail }),
    ...(options.errors !== undefined && { errors: options.errors }),
  };
  return HttpResponse.json(problem, {
    status,
    headers: { "Content-Type": "application/problem+json" },
  });
}

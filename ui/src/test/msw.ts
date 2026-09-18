import { setupServer } from "msw/node";
import { HttpResponse } from "msw";
import { getSystemMock } from "../api/generated/system/system.msw";
import type { ProblemDetail, ValidationErrorDetail } from "../api/problem";

export const server = setupServer(...getSystemMock());

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

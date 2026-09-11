import { setupServer } from "msw/node";
import { HttpResponse } from "msw";
import { getExportImportMock } from "../api/generated/export-import/export-import.msw";
import { getKnowledgeMock } from "../api/generated/knowledge/knowledge.msw";
import { getProjectsMock } from "../api/generated/projects/projects.msw";
import { getRelationsMock } from "../api/generated/relations/relations.msw";
import { getSessionsMock } from "../api/generated/sessions/sessions.msw";
import { getTasksMock } from "../api/generated/tasks/tasks.msw";
import type { ProblemDetail, ValidationErrorDetail } from "../api/problem";

/**
 * Spec-derived MSW server. Generated handlers answer any request with
 * spec-shaped faker data as background noise; tests seed the responses they
 * assert on with `server.use(get<Op>MockHandler(fixture))`.
 */
export const server = setupServer(
  ...getProjectsMock(),
  ...getExportImportMock(),
  ...getTasksMock(),
  ...getSessionsMock(),
  ...getRelationsMock(),
  ...getKnowledgeMock(),
);

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

import { setupServer } from "msw/node";
import { http, HttpResponse } from "msw";
import { getSystemMock } from "../api/generated/system/system.msw";
import type { BrowserSession } from "../api/generated/model/browserSession";
import type { ProblemDetail, ValidationErrorDetail } from "../api/problem";
import { makeBrowserSession, makePage } from "./fixtures";
import type { Project } from "../api/generated/model/project";
import type { Goal } from "../api/generated/model/goal";
import type { Epic } from "../api/generated/model/epic";

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

export function sessionHandler(overrides?: Partial<BrowserSession>) {
  return http.get("/api/v1/session", () =>
    HttpResponse.json(makeBrowserSession(overrides)),
  );
}

export function expiredSessionHandler() {
  return http.get("/api/v1/session", () => problemResponse(401, "unauthenticated"));
}

export function projectListHandler(projects: Project[]) {
  return http.get("/api/v1/projects", () =>
    HttpResponse.json(makePage(projects)),
  );
}

export function projectHandler(project: Project) {
  return http.get(`/api/v1/projects/${project.id}`, () =>
    HttpResponse.json(project),
  );
}

export function goalListHandler(goals: Goal[]) {
  return http.get("/api/v1/projects/:projectId/goals", () =>
    HttpResponse.json(makePage(goals)),
  );
}

export function goalHandler(goal: Goal) {
  return http.get(`/api/v1/projects/:projectId/goals/${goal.id}`, () =>
    HttpResponse.json(goal),
  );
}

export function epicListHandler(epics: Epic[]) {
  return http.get("/api/v1/projects/:projectId/epics", () =>
    HttpResponse.json(makePage(epics)),
  );
}

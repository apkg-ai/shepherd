/**
 * `shepherdFetch` — the orval custom mutator (see orval.config.ts).
 *
 * Every generated operation funnels through here: JSON in/out, problem+json
 * error bodies normalized into `ShepherdError` (docs/agent-guide.md), 204
 * responses resolved as `undefined`.
 */
import { ShepherdError, type ProblemDetail } from "./problem";

function isProblemDetail(body: unknown): body is ProblemDetail {
  return (
    typeof body === "object" &&
    body !== null &&
    typeof (body as ProblemDetail).type === "string" &&
    typeof (body as ProblemDetail).title === "string" &&
    typeof (body as ProblemDetail).status === "number"
  );
}

async function toShepherdError(response: Response): Promise<ShepherdError> {
  let body: unknown;
  try {
    body = await response.json();
  } catch {
    body = undefined;
  }
  if (isProblemDetail(body)) return new ShepherdError(body);
  // Non-problem+json error body (proxy failure, HTML error page, …).
  return new ShepherdError({
    type: "urn:shepherd:error:internal-error",
    title: response.statusText || "Request failed",
    status: response.status,
  });
}

export async function shepherdFetch<T>(url: string, init: RequestInit): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body !== undefined && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(url, { ...init, headers });

  if (!response.ok) throw await toShepherdError(response);
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export default shepherdFetch;

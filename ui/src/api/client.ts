/**
 * The orval custom mutator (orval.config.ts): every generated operation
 * funnels here — problem+json errors throw ShepherdError, 204 → undefined.
 * Adds credentials, CSRF injection, and transport retry.
 */
import { ShepherdError, errorSlug, type ProblemDetail } from "./problem";

let csrfToken: string | undefined;

export function setCsrfToken(token: string | undefined) {
  csrfToken = token;
}

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
  return new ShepherdError({
    type: "urn:shepherd:error:internal-error",
    title: response.statusText || "Request failed",
    status: response.status,
  });
}

const RETRY_DELAYS = [250, 500, 1000];

function isRetryable(error: unknown): boolean {
  if (error instanceof TypeError) return true;
  if (error instanceof ShepherdError) {
    return errorSlug(error) === "storage_busy";
  }
  return false;
}

function isMutationMethod(method: string): boolean {
  return method === "POST" || method === "PATCH" || method === "DELETE";
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function shepherdFetch<T>(url: string, init: RequestInit): Promise<T> {
  const method = (init.method ?? "GET").toUpperCase();
  const headers = new Headers(init.headers);

  if (init.body !== undefined && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  if (isMutationMethod(method) && csrfToken) {
    headers.set("X-CSRF-Token", csrfToken);
  }

  const requestInit: RequestInit = { ...init, headers, credentials: "same-origin" };

  const canRetry =
    !isMutationMethod(method) || headers.has("Idempotency-Key");

  let lastError: unknown;
  for (let attempt = 0; attempt <= RETRY_DELAYS.length; attempt++) {
    try {
      const response = await fetch(url, requestInit);

      if (!response.ok) throw await toShepherdError(response);
      if (response.status === 204) return undefined as T;
      return (await response.json()) as T;
    } catch (error) {
      lastError = error;
      if (!canRetry || attempt >= RETRY_DELAYS.length || !isRetryable(error)) {
        throw error;
      }
      await sleep(RETRY_DELAYS[attempt]);
    }
  }

  throw lastError;
}

export default shepherdFetch;

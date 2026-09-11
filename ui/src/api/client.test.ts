import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server, problemResponse } from "../test/msw";
import { shepherdFetch } from "./client";
import { ShepherdError } from "./problem";

const URL = "/api/v1/projects/test-client";

describe("shepherdFetch", () => {
  it("resolves JSON bodies", async () => {
    server.use(http.get(URL, () => HttpResponse.json({ ok: true })));
    await expect(shepherdFetch(URL, { method: "GET" })).resolves.toEqual({
      ok: true,
    });
  });

  it("resolves 204 responses as undefined", async () => {
    server.use(http.delete(URL, () => new HttpResponse(null, { status: 204 })));
    await expect(shepherdFetch(URL, { method: "DELETE" })).resolves.toBeUndefined();
  });

  it("throws ShepherdError from problem+json bodies", async () => {
    server.use(
      http.get(URL, () =>
        problemResponse(409, "claim-conflict", {
          title: "Claim conflict",
          detail: "Already claimed",
        }),
      ),
    );
    const err = (await shepherdFetch(URL, { method: "GET" }).catch(
      (e: unknown) => e,
    )) as ShepherdError;
    expect(err).toBeInstanceOf(ShepherdError);
    expect(err.type).toBe("urn:shepherd:error:claim-conflict");
    expect(err.status).toBe(409);
    expect(err.message).toBe("Already claimed");
  });

  it("synthesizes a ShepherdError from non-JSON error bodies", async () => {
    server.use(
      http.get(
        URL,
        () =>
          new HttpResponse("<html>Bad gateway</html>", {
            status: 502,
            statusText: "Bad Gateway",
          }),
      ),
    );
    const err = (await shepherdFetch(URL, { method: "GET" }).catch(
      (e: unknown) => e,
    )) as ShepherdError;
    expect(err).toBeInstanceOf(ShepherdError);
    expect(err.type).toBe("urn:shepherd:error:internal-error");
    expect(err.status).toBe(502);
  });

  it("sets Content-Type: application/json when a body is sent", async () => {
    let contentType: string | null = null;
    server.use(
      http.post(URL, ({ request }) => {
        contentType = request.headers.get("content-type");
        return HttpResponse.json({ ok: true });
      }),
    );
    await shepherdFetch(URL, { method: "POST", body: JSON.stringify({}) });
    expect(contentType).toBe("application/json");
  });

  it("keeps a caller-provided Content-Type", async () => {
    let contentType: string | null = null;
    server.use(
      http.post(URL, ({ request }) => {
        contentType = request.headers.get("content-type");
        return HttpResponse.json({ ok: true });
      }),
    );
    await shepherdFetch(URL, {
      method: "POST",
      body: "raw",
      headers: { "Content-Type": "text/plain" },
    });
    expect(contentType).toBe("text/plain");
  });
});

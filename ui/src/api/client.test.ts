import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { server, problemResponse } from "../test/msw";
import { shepherdFetch, setCsrfToken } from "./client";
import { ShepherdError } from "./problem";

const URL = "/api/v1/projects/test-client";

afterEach(() => {
  setCsrfToken(undefined);
});

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
        problemResponse(409, "edit-conflict", {
          title: "Edit conflict",
          detail: "The resource changed underneath the request",
        }),
      ),
    );
    const err = (await shepherdFetch(URL, { method: "GET" }).catch(
      (e: unknown) => e,
    )) as ShepherdError;
    expect(err).toBeInstanceOf(ShepherdError);
    expect(err.type).toBe("urn:shepherd:error:edit-conflict");
    expect(err.status).toBe(409);
    expect(err.message).toBe("The resource changed underneath the request");
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

  describe("credentials", () => {
    it("sends same-origin credentials on every request", async () => {
      let receivedCookie = false;
      server.use(
        http.get(URL, ({ request }) => {
          receivedCookie = request.credentials === "same-origin";
          return HttpResponse.json({ ok: true });
        }),
      );
      await shepherdFetch(URL, { method: "GET" });
      // MSW does not relay credentials, but we verify the init option is set
      // by checking the request reaches the handler without error.
      expect(receivedCookie).toBe(true);
    });
  });

  describe("CSRF", () => {
    it("injects X-CSRF-Token on mutations when set", async () => {
      let csrfHeader: string | null = null;
      server.use(
        http.post(URL, ({ request }) => {
          csrfHeader = request.headers.get("x-csrf-token");
          return HttpResponse.json({ ok: true });
        }),
      );
      setCsrfToken("test-csrf-token");
      await shepherdFetch(URL, { method: "POST", body: JSON.stringify({}) });
      expect(csrfHeader).toBe("test-csrf-token");
    });

    it("injects X-CSRF-Token on PATCH", async () => {
      let csrfHeader: string | null = null;
      server.use(
        http.patch(URL, ({ request }) => {
          csrfHeader = request.headers.get("x-csrf-token");
          return HttpResponse.json({ ok: true });
        }),
      );
      setCsrfToken("patch-csrf");
      await shepherdFetch(URL, { method: "PATCH", body: JSON.stringify({}) });
      expect(csrfHeader).toBe("patch-csrf");
    });

    it("injects X-CSRF-Token on DELETE", async () => {
      let csrfHeader: string | null = null;
      server.use(
        http.delete(URL, ({ request }) => {
          csrfHeader = request.headers.get("x-csrf-token");
          return new HttpResponse(null, { status: 204 });
        }),
      );
      setCsrfToken("delete-csrf");
      await shepherdFetch(URL, { method: "DELETE" });
      expect(csrfHeader).toBe("delete-csrf");
    });

    it("does not inject X-CSRF-Token on GET", async () => {
      let csrfHeader: string | null = null;
      server.use(
        http.get(URL, ({ request }) => {
          csrfHeader = request.headers.get("x-csrf-token");
          return HttpResponse.json({ ok: true });
        }),
      );
      setCsrfToken("should-not-appear");
      await shepherdFetch(URL, { method: "GET" });
      expect(csrfHeader).toBeNull();
    });

    it("omits X-CSRF-Token when not set", async () => {
      let csrfHeader: string | null = null;
      server.use(
        http.post(URL, ({ request }) => {
          csrfHeader = request.headers.get("x-csrf-token");
          return HttpResponse.json({ ok: true });
        }),
      );
      await shepherdFetch(URL, { method: "POST", body: JSON.stringify({}) });
      expect(csrfHeader).toBeNull();
    });
  });

  describe("transport retry", () => {
    it("retries GET on storage_busy and succeeds", async () => {
      let callCount = 0;
      server.use(
        http.get(URL, () => {
          callCount++;
          if (callCount < 3) return problemResponse(503, "storage_busy");
          return HttpResponse.json({ ok: true });
        }),
      );
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const result = await shepherdFetch(URL, { method: "GET" });
      vi.useRealTimers();
      expect(result).toEqual({ ok: true });
      expect(callCount).toBe(3);
    });

    it("does not retry 4xx errors", async () => {
      let callCount = 0;
      server.use(
        http.get(URL, () => {
          callCount++;
          return problemResponse(422, "validation_error");
        }),
      );
      await expect(shepherdFetch(URL, { method: "GET" })).rejects.toThrow();
      expect(callCount).toBe(1);
    });

    it("does not retry mutations without Idempotency-Key", async () => {
      let callCount = 0;
      server.use(
        http.post(URL, () => {
          callCount++;
          return problemResponse(503, "storage_busy");
        }),
      );
      await expect(
        shepherdFetch(URL, { method: "POST", body: JSON.stringify({}) }),
      ).rejects.toThrow();
      expect(callCount).toBe(1);
    });

    it("retries mutations with Idempotency-Key on storage_busy", async () => {
      let callCount = 0;
      server.use(
        http.post(URL, () => {
          callCount++;
          if (callCount < 2) return problemResponse(503, "storage_busy");
          return HttpResponse.json({ ok: true });
        }),
      );
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const result = await shepherdFetch(URL, {
        method: "POST",
        body: JSON.stringify({}),
        headers: { "Idempotency-Key": "test-key" },
      });
      vi.useRealTimers();
      expect(result).toEqual({ ok: true });
      expect(callCount).toBe(2);
    });

    it("gives up after max retries", async () => {
      server.use(http.get(URL, () => problemResponse(503, "storage_busy")));
      vi.useFakeTimers({ shouldAdvanceTime: true });
      await expect(shepherdFetch(URL, { method: "GET" })).rejects.toThrow(ShepherdError);
      vi.useRealTimers();
    });
  });
});

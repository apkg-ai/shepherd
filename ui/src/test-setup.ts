import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vitest";
import { server } from "./test/msw";

// Without vitest globals, RTL cannot self-register its auto-cleanup —
// register it here so the DOM never leaks between tests (#21).
afterEach(cleanup);

// Spec-derived MSW server: generated handlers as background, per-test
// overrides via server.use(...). Unhandled requests are bugs.
beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

// React Flow's documented jsdom shims (the graph screen, S8): jsdom has no
// ResizeObserver or DOMMatrixReadOnly and never lays anything out. The
// observer fires synchronously on observe so React Flow measures nodes
// (via the offset* mocks below) and renders edges. Guarded so real browsers
// are never touched.
if (typeof globalThis.ResizeObserver === "undefined") {
  class ResizeObserverStub {
    private readonly callback: ResizeObserverCallback;
    constructor(callback: ResizeObserverCallback) {
      this.callback = callback;
    }
    observe(target: Element) {
      // Async like the real thing: React Flow registers nodes from child
      // effects before the parent effect stores the DOM node it needs for
      // measuring — a synchronous callback would fire into a half-built
      // store and never be retried.
      setTimeout(() => {
        this.callback(
          [{ target, contentRect: target.getBoundingClientRect() } as ResizeObserverEntry],
          this as unknown as ResizeObserver,
        );
      }, 0);
    }
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

  // Nodes carry explicit inline dimensions (graphLayout sets node.width/
  // height); surface them through the offset* properties React Flow measures.
  for (const [prop, styleProp] of [
    ["offsetWidth", "width"],
    ["offsetHeight", "height"],
  ] as const) {
    const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, prop);
    Object.defineProperty(HTMLElement.prototype, prop, {
      get(this: HTMLElement) {
        return parseFloat(this.style[styleProp]) || (original?.get?.call(this) as number) || 0;
      },
    });
  }
}

if (typeof globalThis.DOMMatrixReadOnly === "undefined") {
  class DOMMatrixReadOnlyStub {
    m22 = 1;
    constructor(_transform?: string) {}
  }
  globalThis.DOMMatrixReadOnly = DOMMatrixReadOnlyStub as unknown as typeof DOMMatrixReadOnly;
}

if (typeof SVGElement !== "undefined" && !("getBBox" in SVGElement.prototype)) {
  Object.defineProperty(SVGElement.prototype, "getBBox", {
    value: () => ({ x: 0, y: 0, width: 0, height: 0 }),
  });
}

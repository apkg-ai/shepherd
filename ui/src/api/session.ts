import { createContext, useContext } from "react";
import type { BrowserSession } from "./generated/model/browserSession";

export interface SessionContextValue {
  session: BrowserSession;
  logout: () => void;
}

export const SessionContext = createContext<SessionContextValue | null>(null);

export function useSession(): SessionContextValue {
  const ctx = useContext(SessionContext);
  if (!ctx) throw new Error("useSession must be used within AuthBoundary");
  return ctx;
}

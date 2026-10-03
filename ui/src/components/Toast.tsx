import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import styles from "./Toast.module.css";

export type ToastKind = "success" | "error" | "info";

interface ToastItem {
  id: number;
  kind: ToastKind;
  message: string;
}

interface ToastContextValue {
  toast: (message: string, kind?: ToastKind) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

const AUTO_DISMISS_MS = 5000;

export function ToastProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(0);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());
  // Which interaction currently holds each toast open.
  const holds = useRef(new Map<number, { hover: boolean; focus: boolean }>());

  const dismiss = useCallback((id: number) => {
    const timer = timers.current.get(id);
    if (timer !== undefined) clearTimeout(timer);
    timers.current.delete(id);
    holds.current.delete(id);
    setItems((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const startTimer = useCallback(
    (id: number) => {
      const existing = timers.current.get(id);
      if (existing !== undefined) clearTimeout(existing);
      timers.current.set(
        id,
        setTimeout(() => dismiss(id), AUTO_DISMISS_MS),
      );
    },
    [dismiss],
  );

  // WCAG 2.2.1: hover and focus each pause auto-dismiss; the full window
  // restarts only once the last hold on the toast releases.
  const hold = useCallback((id: number, kind: "hover" | "focus") => {
    const state = holds.current.get(id) ?? { hover: false, focus: false };
    state[kind] = true;
    holds.current.set(id, state);
    const timer = timers.current.get(id);
    if (timer !== undefined) clearTimeout(timer);
    timers.current.delete(id);
  }, []);

  const release = useCallback(
    (id: number, kind: "hover" | "focus") => {
      const state = holds.current.get(id);
      if (state === undefined) return;
      state[kind] = false;
      if (!state.hover && !state.focus) startTimer(id);
    },
    [startTimer],
  );

  const toast = useCallback(
    (message: string, kind: ToastKind = "info") => {
      const id = nextId.current++;
      setItems((prev) => [...prev, { id, kind, message }]);
      startTimer(id);
    },
    [startTimer],
  );

  const value = useMemo(() => ({ toast }), [toast]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className={styles.stack}>
        {items.map((item) => (
          <div
            key={item.id}
            role={item.kind === "error" ? "alert" : "status"}
            className={`${styles.toast} ${styles[item.kind]}`}
            onMouseEnter={() => hold(item.id, "hover")}
            onMouseLeave={() => release(item.id, "hover")}
            onFocusCapture={() => hold(item.id, "focus")}
            onBlurCapture={() => release(item.id, "focus")}
          >
            <span>{item.message}</span>
            <button
              type="button"
              className={styles.dismiss}
              aria-label={t("common.action.dismiss")}
              onClick={() => dismiss(item.id)}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

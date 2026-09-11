import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createHashRouter, RouterProvider } from "react-router";
import { ToastProvider } from "./components/Toast";
import { AppQueryProvider } from "./lib/queryClient";
import { routes } from "./router";
import "./index.css";

const router = createHashRouter(routes);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ToastProvider>
      <AppQueryProvider>
        <RouterProvider router={router} />
      </AppQueryProvider>
    </ToastProvider>
  </StrictMode>,
);

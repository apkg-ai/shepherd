import type { RouteObject } from "react-router";
import { AppShell } from "./screens/AppShell";
import { HomeScreen } from "./screens/HomeScreen";
import { NotFound } from "./screens/NotFound";
import { RouteError } from "./screens/RouteError";

/**
 * The route table, shared by the app (createHashRouter in main.tsx — hash
 * URLs survive the server's bare ServeDir and a future Tauri shell) and by
 * tests (createMemoryRouter in test/test-utils.tsx).
 */
export const routes: RouteObject[] = [
  {
    path: "/",
    element: <AppShell />,
    errorElement: <RouteError />,
    children: [
      { index: true, element: <HomeScreen /> },
      { path: "*", element: <NotFound /> },
    ],
  },
];

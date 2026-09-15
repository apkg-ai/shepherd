import type { RouteObject } from "react-router";
import { AppShell } from "./screens/AppShell";
import { HomeScreen } from "./screens/HomeScreen";
import { NotFound } from "./screens/NotFound";
import { RouteError } from "./screens/RouteError";

/**
 * The route table, shared by the app (createHashRouter in main.tsx — hash
 * URLs survive the server's bare ServeDir and a future Tauri shell) and by
 * tests (createMemoryRouter in test/test-utils.tsx).
 *
 * v1 step 000: one accessible scaffold route. Later steps add the v1
 * application routes here.
 */
export const routes: RouteObject[] = [
  {
    path: "/",
    element: <AppShell />,
    // A render-time crash in any screen falls back to RouteError instead of
    // a blank page (the toast system lives inside the shell).
    errorElement: <RouteError />,
    children: [
      { index: true, element: <HomeScreen /> },
      { path: "*", element: <NotFound /> },
    ],
  },
];

import type { RouteObject } from "react-router";
import { AppLayout } from "./screens/AppLayout";
import { NotFound } from "./screens/NotFound";
import { ProjectRegistryScreen } from "./screens/projects/ProjectRegistryScreen";

/**
 * The route table, shared by the app (createHashRouter in main.tsx — hash
 * URLs survive the server's bare ServeDir and a future Tauri shell) and by
 * tests (createMemoryRouter in test/test-utils.tsx).
 */
export const routes: RouteObject[] = [
  {
    path: "/",
    element: <AppLayout />,
    children: [
      { index: true, element: <ProjectRegistryScreen /> },
      { path: "*", element: <NotFound /> },
    ],
  },
];

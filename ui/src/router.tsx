import type { RouteObject } from "react-router";
import { AppLayout } from "./screens/AppLayout";
import { NotFound } from "./screens/NotFound";
import { GraphScreen } from "./screens/graph/GraphScreen";
import { ProjectRegistryScreen } from "./screens/projects/ProjectRegistryScreen";
import { ProjectSettingsScreen } from "./screens/projects/ProjectSettingsScreen";
import { ReviewQueueScreen } from "./screens/review/ReviewQueueScreen";
import { ProjectTasksScreen } from "./screens/tasks/ProjectTasksScreen";
import { TaskFormScreen } from "./screens/tasks/TaskFormScreen";
import { TaskDetailScreen } from "./screens/tasks/detail/TaskDetailScreen";

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
      // The graph is the project's home screen (S8, docs/04-ui.md); the
      // tasks table lives one level down.
      { path: "projects/:projectId", element: <GraphScreen /> },
      { path: "projects/:projectId/tasks", element: <ProjectTasksScreen /> },
      { path: "projects/:projectId/tasks/new", element: <TaskFormScreen /> },
      {
        path: "projects/:projectId/tasks/:taskId",
        element: <TaskDetailScreen />,
      },
      {
        path: "projects/:projectId/tasks/:taskId/edit",
        element: <TaskFormScreen />,
      },
      { path: "projects/:projectId/review", element: <ReviewQueueScreen /> },
      {
        path: "projects/:projectId/settings",
        element: <ProjectSettingsScreen />,
      },
      { path: "*", element: <NotFound /> },
    ],
  },
];

import type { RouteObject } from "react-router";
import { AppShell } from "./screens/AppShell";
import { AppLayout } from "./screens/AppLayout";
import { AuthBoundary } from "./screens/identity/AuthBoundary";
import { LoginScreen } from "./screens/identity/LoginScreen";
import { ProjectListScreen } from "./screens/projects/ProjectListScreen";
import { ProjectOverviewScreen } from "./screens/projects/ProjectOverviewScreen";
import { GoalDetailScreen } from "./screens/goals/GoalDetailScreen";
import { GoalFormScreen } from "./screens/goals/GoalFormScreen";
import { EpicDetailScreen } from "./screens/epics/EpicDetailScreen";
import { EpicFormScreen } from "./screens/epics/EpicFormScreen";
import { TaskFormScreen } from "./screens/tasks/TaskFormScreen";
import { TaskDetailScreen } from "./screens/tasks/detail/TaskDetailScreen";
import { NotFound } from "./screens/NotFound";
import { RouteError } from "./screens/RouteError";

/**
 * The route table, shared by the app (createHashRouter in main.tsx — hash
 * URLs survive the server's bare ServeDir) and by tests.
 */
export const routes: RouteObject[] = [
  {
    path: "/",
    element: <AppShell />,
    errorElement: <RouteError />,
    children: [
      { path: "login", element: <LoginScreen /> },
      {
        element: <AuthBoundary />,
        children: [
          { index: true, element: <ProjectListScreen /> },
          {
            path: "projects/:projectId",
            element: <AppLayout />,
            children: [
              { index: true, element: <ProjectOverviewScreen /> },
              { path: "goals/new", element: <GoalFormScreen /> },
              { path: "goals/:goalId", element: <GoalDetailScreen /> },
              { path: "goals/:goalId/edit", element: <GoalFormScreen /> },
              { path: "goals/:goalId/epics/new", element: <EpicFormScreen /> },
              { path: "epics/:epicId", element: <EpicDetailScreen /> },
              { path: "epics/:epicId/edit", element: <EpicFormScreen /> },
              { path: "epics/:epicId/tasks/new", element: <TaskFormScreen /> },
              { path: "tasks/:taskId", element: <TaskDetailScreen /> },
              { path: "tasks/:taskId/edit", element: <TaskFormScreen /> },
            ],
          },
        ],
      },
      { path: "*", element: <NotFound /> },
    ],
  },
];

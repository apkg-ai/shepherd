import { Link } from "react-router";
import { EmptyState } from "../components/states";

export function NotFound() {
  return (
    <EmptyState title="Page not found">
      <Link to="/">Back to projects</Link>
    </EmptyState>
  );
}

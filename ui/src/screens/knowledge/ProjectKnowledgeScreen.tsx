import { Link, useParams, useSearchParams } from "react-router";
import type { ListKnowledgeScope, ListKnowledgeType } from "../../api/generated/model";
import { useListKnowledgeInfinite } from "../../api/generated/knowledge/knowledge";
import { cursorPaging, flattenPages, useAllPages } from "../../api/paging";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { formatDateTime } from "../../lib/format";
import { isHttpUrl } from "../../lib/links";
import styles from "./ProjectKnowledgeScreen.module.css";

const SCOPES: ListKnowledgeScope[] = ["project", "task", "session"];
const TYPES: ListKnowledgeType[] = ["note", "link", "decision", "transcript"];

/**
 * Project knowledge (docs/04-ui.md): everything the project knows —
 * project-scoped conventions plus task- and session-produced items — in one
 * searchable place. Scope/type narrow server-side; search filters the
 * fetched set client-side (server-side search is a tracked follow-up), which
 * is honest for a local, single-user daemon.
 */
export function ProjectKnowledgeScreen() {
  const { projectId } = useParams();
  if (!projectId) return <EmptyState title="No project selected" />;
  return <KnowledgeList projectId={projectId} />;
}

function KnowledgeList({ projectId }: { projectId: string }) {
  const [searchParams, setSearchParams] = useSearchParams();

  const rawScope = searchParams.get("scope");
  const rawType = searchParams.get("type");
  const scope = SCOPES.find((s) => s === rawScope);
  const type = TYPES.find((t) => t === rawType);
  const q = searchParams.get("q") ?? "";

  const query = useListKnowledgeInfinite(
    projectId,
    {
      limit: 100,
      ...(scope && { scope }),
      ...(type && { type }),
    },
    { query: cursorPaging() },
  );
  // Search needs the whole collection, not a page.
  useAllPages(query);
  const items = flattenPages(query.data?.pages);

  const needle = q.trim().toLowerCase();
  const matches =
    needle === ""
      ? items
      : items.filter(
          (item) =>
            item.title.toLowerCase().includes(needle) ||
            item.content.toLowerCase().includes(needle),
        );

  const setParam = (key: "scope" | "type" | "q", value: string) => {
    setSearchParams(
      (params) => {
        if (value === "") params.delete(key);
        else params.set(key, value);
        return params;
      },
      { replace: key === "q" },
    );
  };

  const body = () => {
    if (query.isPending) return <LoadingState label="Loading knowledge…" />;
    if (query.isError) {
      return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
    }
    if (items.length === 0) {
      return (
        <EmptyState title="No knowledge yet">
          <p>Notes, links, decisions, and transcripts recorded on tasks and sessions land here.</p>
        </EmptyState>
      );
    }
    return (
      <>
        <p role="status" className={styles.count}>
          {matches.length} of {items.length} item{items.length === 1 ? "" : "s"}
          {query.hasNextPage ? " (still loading…)" : ""}
        </p>
        <ul className={styles.list}>
          {matches.map((item) => (
            <li key={item.id} className={styles.item}>
              <div className={styles.itemHeader}>
                <span className={styles.type}>{item.type}</span>
                <strong>{item.title}</strong>
                <span className={styles.scope}>{item.scope}</span>
              </div>
              {item.type === "link" && isHttpUrl(item.content) ? (
                <a href={item.content} target="_blank" rel="noreferrer">
                  {item.content}
                </a>
              ) : (
                <p className={styles.content}>{item.content}</p>
              )}
              <div className={styles.meta}>
                <span>{formatDateTime(item.created_at)}</span>
                {item.task_id != null ? (
                  <Link to={`/projects/${projectId}/tasks/${item.task_id}`}>From task</Link>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      </>
    );
  };

  return (
    <section>
      <PageHeader
        title="Knowledge"
        description="Everything this project knows, searchable across tasks and sessions."
      >
        <div className={styles.filters}>
          <label className={styles.filter}>
            Search
            <input
              type="search"
              value={q}
              placeholder="Title or content…"
              onChange={(e) => setParam("q", e.target.value)}
            />
          </label>
          <label className={styles.filter}>
            Scope
            <select value={scope ?? ""} onChange={(e) => setParam("scope", e.target.value)}>
              <option value="">All</option>
              {SCOPES.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
          <label className={styles.filter}>
            Type
            <select value={type ?? ""} onChange={(e) => setParam("type", e.target.value)}>
              <option value="">All</option>
              {TYPES.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          </label>
        </div>
      </PageHeader>
      {body()}
    </section>
  );
}

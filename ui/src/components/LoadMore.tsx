import { Button } from "./Button";
import styles from "./LoadMore.module.css";

/**
 * Cursor-pagination footer for infinite queries: a button while more pages
 * exist, nothing once the cursor is exhausted.
 */
export function LoadMore({
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
}: {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
}) {
  if (!hasNextPage) return null;
  return (
    <div className={styles.wrap}>
      <Button busy={isFetchingNextPage} onClick={onLoadMore}>
        Load more
      </Button>
    </div>
  );
}

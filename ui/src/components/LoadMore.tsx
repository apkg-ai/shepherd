import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
  if (!hasNextPage) return null;
  return (
    <div className={styles.wrap}>
      <Button busy={isFetchingNextPage} onClick={onLoadMore}>
        {t("common.action.loadMore")}
      </Button>
    </div>
  );
}

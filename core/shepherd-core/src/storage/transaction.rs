use std::future::Future;
use std::pin::Pin;

use sqlx::{Sqlite, Transaction};

use super::{StorageError, Store};

// Boxed rather than AsyncFnOnce: spawned callers trip rustc's "implementation of
// AsyncFnOnce is not general enough" higher-ranked inference limit.
pub type TxFuture<'t, T> = Pin<Box<dyn Future<Output = Result<T, StorageError>> + Send + 't>>;

impl Store {
    // Write reservation before any reads (plan/05): the no-op UPDATE upgrades the
    // deferred BEGIN to SQLite's write lock so concurrent commands serialize.
    pub async fn command_transaction<T, F>(&self, command: F) -> Result<T, StorageError>
    where
        F: for<'t> FnOnce(&'t mut Transaction<'static, Sqlite>) -> TxFuture<'t, T>,
    {
        let mut tx = self.pool().begin().await?;
        let reserved = sqlx::query("UPDATE command_lock SET value=value WHERE id=1")
            .execute(&mut *tx)
            .await?;
        if reserved.rows_affected() != 1 {
            tx.rollback().await.ok();
            return Err(StorageError::Corrupt(
                "command_lock row missing; write serialization unavailable".into(),
            ));
        }
        match command(&mut tx).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(err) => {
                tx.rollback().await.ok();
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Actor, ActorId, ActorKind};
    use crate::storage::rows::{get_actor, insert_actor};
    use crate::storage::testing::{store_options, test_clock};
    use crate::storage::{StorageError, Store, open};

    async fn test_store(dir: &tempfile::TempDir) -> Store {
        open(store_options(dir.path(), "shepherd.db", test_clock()))
            .await
            .unwrap()
    }

    fn actor(store: &Store, label: &str) -> Actor {
        let now = store.clock().now();
        Actor {
            id: ActorId::generate(now),
            kind: ActorKind::Agent,
            label: label.to_string(),
            revoked: false,
            created_at: now,
        }
    }

    #[tokio::test]
    async fn committed_transaction_persists_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir).await;
        let actor = actor(&store, "worker");
        let inserted = actor.clone();
        store
            .command_transaction(|tx| {
                Box::pin(async move {
                    insert_actor(tx, &inserted).await?;
                    Ok(())
                })
            })
            .await
            .unwrap();
        assert_eq!(
            get_actor(store.pool(), &actor.id).await.unwrap(),
            Some(actor)
        );
    }

    #[tokio::test]
    async fn failing_transaction_rolls_back_all_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir).await;
        let actor = actor(&store, "worker");
        let inserted = actor.clone();
        let err = store
            .command_transaction(|tx| {
                Box::pin(async move {
                    insert_actor(tx, &inserted).await?;
                    Err::<(), _>(StorageError::Corrupt("injected failure".into()))
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Corrupt(_)));
        assert_eq!(get_actor(store.pool(), &actor.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn missing_command_lock_row_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir).await;
        sqlx::query("DELETE FROM command_lock")
            .execute(store.pool())
            .await
            .unwrap();
        let err = store
            .command_transaction(|_tx| Box::pin(async move { Ok(()) }))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Corrupt(_)));
    }
}

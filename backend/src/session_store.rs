//! SQLite session store for `tower-sessions`.
//!
//! Replaces `tower-sessions-sqlx-store`, which has no release for sqlx 0.9.
//! Table layout and record encoding (MessagePack) match that crate, so
//! sessions created before the switch remain valid.

use async_trait::async_trait;
use sqlx::{SqliteConnection, SqlitePool};
use time::OffsetDateTime;
use tower_sessions::{
    SessionStore,
    session::{Id, Record},
    session_store,
};

#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
}

fn backend_err(e: sqlx::Error) -> session_store::Error {
    session_store::Error::Backend(e.to_string())
}

fn encode(record: &Record) -> session_store::Result<Vec<u8>> {
    rmp_serde::to_vec(record).map_err(|e| session_store::Error::Encode(e.to_string()))
}

impl SqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> sqlx::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tower_sessions (
                id TEXT PRIMARY KEY NOT NULL,
                data BLOB NOT NULL,
                expiry_date INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Deletes the sessions that have expired (`load` no longer returns them); how many there were.
    /// A session is also written for visitors who are not signed in (the page wanted, an OIDC sign-in
    /// under way), so without this the table only grows.
    pub async fn delete_expired(&self) -> sqlx::Result<u64> {
        let done = sqlx::query("DELETE FROM tower_sessions WHERE expiry_date <= ?")
            .bind(OffsetDateTime::now_utc())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }

    /// Deletes the expired sessions now and then once an hour.
    pub fn spawn_cleanup(&self) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            loop {
                interval.tick().await;
                if let Err(e) = store.delete_expired().await {
                    eprintln!("[sessions] deleting expired sessions: {e}");
                }
            }
        });
    }

    /// Returns `false` if the ID is already taken.
    async fn try_create(
        conn: &mut SqliteConnection,
        record: &Record,
    ) -> session_store::Result<bool> {
        let res = sqlx::query(
            "INSERT OR ABORT INTO tower_sessions (id, data, expiry_date) VALUES (?, ?, ?)",
        )
        .bind(record.id.to_string())
        .bind(encode(record)?)
        .bind(record.expiry_date)
        .execute(conn)
        .await;

        match res {
            Ok(_) => Ok(true),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(false),
            Err(e) => Err(backend_err(e)),
        }
    }
}

#[async_trait]
impl SessionStore for SqliteStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        let mut tx = self.pool.begin().await.map_err(backend_err)?;
        while !Self::try_create(&mut tx, record).await? {
            record.id = Id::default();
        }
        tx.commit().await.map_err(backend_err)?;
        Ok(())
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        sqlx::query(
            "INSERT INTO tower_sessions (id, data, expiry_date) VALUES (?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               data = excluded.data,
               expiry_date = excluded.expiry_date",
        )
        .bind(record.id.to_string())
        .bind(encode(record)?)
        .bind(record.expiry_date)
        .execute(&self.pool)
        .await
        .map_err(backend_err)?;
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let data: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT data FROM tower_sessions WHERE id = ? AND expiry_date > ?")
                .bind(session_id.to_string())
                .bind(OffsetDateTime::now_utc())
                .fetch_optional(&self.pool)
                .await
                .map_err(backend_err)?;

        data.map(|d| {
            rmp_serde::from_slice(&d).map_err(|e| session_store::Error::Decode(e.to_string()))
        })
        .transpose()
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        sqlx::query("DELETE FROM tower_sessions WHERE id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use time::Duration;

    #[tokio::test]
    async fn expired_sessions_are_deleted() {
        // one connection: each connection to `sqlite::memory:` is a database of its own
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let store = SqliteStore::new(pool.clone());
        store.migrate().await.unwrap();
        let record = |expires_in: Duration| Record {
            id: Id::default(),
            data: Default::default(),
            expiry_date: OffsetDateTime::now_utc() + expires_in,
        };
        let (old, live) = (record(Duration::minutes(-1)), record(Duration::days(7)));
        store.save(&old).await.unwrap();
        store.save(&live).await.unwrap();

        assert_eq!(store.delete_expired().await.unwrap(), 1);
        assert!(store.load(&live.id).await.unwrap().is_some());
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tower_sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 1);
        assert_eq!(store.delete_expired().await.unwrap(), 0);
    }
}

//! `event_content`: large or sensitive event bodies — tool input and
//! output, prompts — stored by the xxh3-128 hex of their bytes, the same
//! hash the snapshot blob store uses (P3.2/P3.3, V102). An event payload
//! carries a [`ContentRef`]; retention deletes a body after its
//! namespace's window and the event stays. See `.context/data-model.md`
//! "event_content".

use oxplow_domain::events::schema::ContentRef;
use oxplow_domain::{DomainError, Timestamp};
use rusqlite::{params, Connection, OptionalExtension};

use crate::database::{map_sql_err, ts_to_string, Database};

/// The content hash of `bytes`: xxh3-128, lowercase hex.
pub fn hash(bytes: &[u8]) -> String {
    format!("{:032x}", xxhash_rust::xxh3::xxh3_128(bytes))
}

/// Store `bytes` under `namespace` in the caller's transaction and return
/// their ref. Identical bytes are stored once.
pub fn put_tx(conn: &Connection, namespace: &str, bytes: &[u8]) -> Result<ContentRef, DomainError> {
    let h = hash(bytes);
    conn.execute(
        "INSERT INTO event_content (hash, namespace, bytes, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (hash) DO NOTHING",
        params![
            h,
            namespace,
            bytes,
            bytes.len() as i64,
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(map_sql_err)?;
    Ok(ContentRef {
        hash: h,
        size: bytes.len() as u64,
    })
}

/// The bytes stored under `hash`, or `None` when there are none (never
/// stored, or retention removed them).
pub async fn read(db: &Database, hash: &str) -> Result<Option<Vec<u8>>, DomainError> {
    let hash = hash.to_string();
    db.call(move |conn| {
        conn.query_row(
            "SELECT bytes FROM event_content WHERE hash = ?1",
            [hash],
            |r| r.get(0),
        )
        .optional()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn content_is_stored_once_by_hash_and_read_back() {
        let db = Database::in_memory();
        let (a, b) = db
            .transaction(|tx| {
                Ok((
                    put_tx(tx, "agent", b"hello")?,
                    put_tx(tx, "agent", b"hello")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(a.size, 5);
        assert_eq!(a.hash, hash(b"hello"));
        assert_eq!(
            read(&db, &a.hash).await.unwrap().as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(read(&db, "missing").await.unwrap(), None);
    }
}

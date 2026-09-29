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
        truncated: false,
    })
}

/// The most of one body that is stored; the rest is dropped and the ref
/// says so (`truncated`). Tool outputs can run to megabytes.
pub const MAX_STORED_BYTES: usize = 256 * 1024;

/// Store a JSON body: serialized canonically (object keys sorted, at every
/// depth) so equal bodies hash equal whatever order their keys arrived
/// in, and cut to [`MAX_STORED_BYTES`] on a character boundary.
pub fn put_json_tx(
    conn: &Connection,
    namespace: &str,
    body: &serde_json::Value,
) -> Result<ContentRef, DomainError> {
    let bytes = serde_json::to_vec(&canonical(body))
        .map_err(|e| DomainError::Invalid(format!("event body: {e}")))?;
    let size = bytes.len();
    let kept = &bytes[..char_boundary_at_or_below(&bytes, MAX_STORED_BYTES)];
    let mut r = put_tx(conn, namespace, kept)?;
    r.size = size as u64;
    r.truncated = kept.len() < size;
    Ok(r)
}

/// `v` with every object's keys in sorted order.
fn canonical(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            serde_json::Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), canonical(&map[k])))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical).collect())
        }
        other => other.clone(),
    }
}

/// The largest length `<= max` that ends on a UTF-8 character boundary of
/// `bytes` (which are valid UTF-8).
pub fn char_boundary_at_or_below(bytes: &[u8], max: usize) -> usize {
    if bytes.len() <= max {
        return bytes.len();
    }
    let mut end = max;
    // A continuation byte is 0b10xx_xxxx.
    while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    end
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

    #[tokio::test]
    async fn json_bodies_hash_by_content_not_key_order_and_are_capped() {
        let db = Database::in_memory();
        let big = "é".repeat(MAX_STORED_BYTES); // two bytes each
        let (a, b, c) = db
            .transaction(move |tx| {
                Ok((
                    put_json_tx(
                        tx,
                        "agent",
                        &serde_json::json!({"a": 1, "b": {"y": 2, "x": 3}}),
                    )?,
                    put_json_tx(
                        tx,
                        "agent",
                        &serde_json::json!({"b": {"x": 3, "y": 2}, "a": 1}),
                    )?,
                    put_json_tx(tx, "agent", &serde_json::json!({ "out": big }))?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(a, b, "same body, different key order: one hash");
        assert!(!a.truncated);
        assert!(c.truncated);
        assert!(c.size > MAX_STORED_BYTES as u64, "size is the whole body's");
        let stored = read(&db, &c.hash).await.unwrap().unwrap();
        assert!(stored.len() <= MAX_STORED_BYTES);
        assert!(
            std::str::from_utf8(&stored).is_ok(),
            "cut on a character boundary"
        );
    }
}

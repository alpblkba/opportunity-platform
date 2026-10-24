use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::sources::SourceError;

/// Raw payloads are personal-adjacent operational data and can be large, so the bytes live under the ignored private tree and only the metadata and path go in the database.
pub fn cache_root() -> PathBuf {
    std::env::var("RAW_SNAPSHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("private/cache"))
}

/// Store a fetched payload and return its row id. Re-fetching an unchanged payload hits the content-hash uniqueness constraint and reuses the existing row instead of writing the bytes again.
pub async fn store(
    tx: &mut Transaction<'_, Postgres>,
    source_id: Uuid,
    source_url: &str,
    body: &str,
) -> Result<Option<Uuid>, SourceError> {
    let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    let existing: Option<Uuid> = sqlx::query_scalar(
        "select id from raw_documents where source_id = $1 and content_hash = $2",
    )
    .bind(source_id)
    .bind(&hash)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(id) = existing {
        return Ok(Some(id));
    }

    let relative = Path::new(&source_id.to_string()).join(format!("{hash}.json"));
    let absolute = cache_root().join(&relative);
    if let Some(parent) = absolute.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            tracing::warn!(%error, "could not create the raw snapshot directory; recording metadata without the body");
            return Ok(None);
        }
    }
    if let Err(error) = std::fs::write(&absolute, body) {
        tracing::warn!(%error, "could not write the raw snapshot; recording metadata without the body");
        return Ok(None);
    }

    let id: Uuid = sqlx::query_scalar(
        "insert into raw_documents (source_id, content_hash, source_url, byte_size, storage_path)
         values ($1, $2, $3, $4, $5) returning id",
    )
    .bind(source_id)
    .bind(&hash)
    .bind(source_url)
    .bind(body.len() as i32)
    .bind(absolute.to_string_lossy().as_ref())
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(id))
}

/// Read a stored payload back for offline reparse.
pub fn load(storage_path: &str) -> std::io::Result<String> {
    std::fs::read_to_string(storage_path)
}

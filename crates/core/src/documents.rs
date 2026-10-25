//! Private CV storage using the existing document registry and backed-up private tree.
use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{profiles, Db};

pub const MAX_CV_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("{0}")]
    Invalid(String),
    #[error("CV not found")]
    NotFound,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Owner(#[from] profiles::ProfileError),
}

#[derive(Debug, Serialize, FromRow)]
pub struct Document {
    pub id: Uuid,
    pub label: String,
    pub version: i32,
    pub byte_size: i32,
    pub media_type: String,
    pub created_at: DateTime<Utc>,
}

pub fn cv_media_type(name: &str, bytes: &[u8]) -> Result<&'static str, DocumentError> {
    if name.trim().is_empty()
        || name.len() > 180
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(DocumentError::Invalid(
            "Use a filename without directory names (at most 180 bytes).".into(),
        ));
    }
    if bytes.is_empty() || bytes.len() > MAX_CV_BYTES {
        return Err(DocumentError::Invalid(
            "Choose a nonempty CV up to 5 MiB.".into(),
        ));
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pdf") && bytes.starts_with(b"%PDF-") {
        Ok("application/pdf")
    } else if lower.ends_with(".txt") && !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok() {
        Ok("text/plain")
    } else {
        Err(DocumentError::Invalid(
            "Upload a PDF or UTF-8 text CV. Export Word documents as PDF first.".into(),
        ))
    }
}

pub async fn upload(pool: &Db, name: &str, bytes: &[u8]) -> Result<Document, DocumentError> {
    let name = name.trim();
    let media_type = cv_media_type(name, bytes)?;
    let owner = profiles::local_owner(pool).await?;
    let id = Uuid::new_v4();
    let path = PathBuf::from("private/documents").join(id.to_string());
    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(path.parent().unwrap())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let result = async {
        file.write_all(bytes)?;
        file.sync_all()?;
        let mut tx = pool.begin().await?;
        // Serialise versions of the same owner's CV without overwriting any earlier file.
        sqlx::query("select id from owners where id = $1 for update").bind(owner).execute(&mut *tx).await?;
        let doc = sqlx::query_as::<_, Document>(
            "insert into documents (id, owner_id, kind, label, version, file_path, content_hash, byte_size, media_type)
             values ($1,$2,'cv',$3,(select coalesce(max(version),0)+1 from documents where owner_id=$2 and kind='cv' and label=$3),$4,$5,$6,$7)
             returning id, label, version, byte_size, media_type, created_at")
            .bind(id).bind(owner).bind(name).bind(path.to_string_lossy().as_ref())
            .bind(format!("{:x}", Sha256::digest(bytes)))
            .bind(bytes.len() as i32).bind(media_type).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok::<_, DocumentError>(doc)
    }.await;
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

pub async fn list(pool: &Db) -> Result<Vec<Document>, DocumentError> {
    Ok(sqlx::query_as::<_, Document>("select id, label, version, byte_size, media_type, created_at from documents where kind='cv' order by created_at desc")
        .fetch_all(pool).await?)
}

pub async fn download(pool: &Db, id: Uuid) -> Result<(String, Vec<u8>), DocumentError> {
    let (path, label): (String, String) =
        sqlx::query_as("select file_path, label from documents where id=$1 and kind='cv'")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or(DocumentError::NotFound)?;
    Ok((label, std::fs::read(path)?))
}

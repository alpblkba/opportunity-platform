//! Private CV storage using the existing document registry and local private tree.
use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{candidate, profiles, Db};

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
    let root = std::env::var("DOCUMENT_ROOT").unwrap_or_else(|_| "private/documents".into());
    let path = PathBuf::from(root).join(format!(
        "{id}.{}",
        if media_type == "application/pdf" {
            "pdf"
        } else {
            "txt"
        }
    ));
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
        let text = if media_type == "text/plain" {
            String::from_utf8_lossy(bytes).to_string()
        } else {
            let mut command = tokio::process::Command::new("pdftotext");
            command.kill_on_drop(true).args(["-layout"]).arg(&path).arg("-");
            let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
                .await.map_err(|_| DocumentError::Invalid("CV text extraction timed out.".into()))?
                .map_err(|_| DocumentError::Invalid("PDF text extraction is unavailable. Install pdftotext or upload a .txt CV.".into()))?;
            if !output.status.success() { return Err(DocumentError::Invalid("This PDF could not be read. Upload an unlocked text PDF.".into())); }
            String::from_utf8_lossy(&output.stdout).to_string()
        };
        if text.len() > 500_000 { return Err(DocumentError::Invalid("CV text is too large.".into())); }
        let candidate = candidate::extract(&text).map_err(DocumentError::Invalid)?;
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
        sqlx::query("update documents set extracted_text=$2, candidate=$3, approved=true where id=$1")
            .bind(id).bind(&text).bind(serde_json::to_value(&candidate).unwrap()).execute(&mut *tx).await?;
        // Facts extracted from older CVs must not survive a replacement as current claims.
        sqlx::query("update applicant_facts set approved=false where owner_id=$1 and source_document_id is not null")
            .bind(owner).execute(&mut *tx).await?;
        for (key, value, category) in [
            ("full name", candidate.name.as_ref(), "identity"),
            ("email", candidate.email.as_ref(), "contact"),
            ("phone", candidate.phone.as_ref(), "contact"),
        ] {
            if let Some(value) = value {
                // Preserve an owner's explicitly maintained fact; only replace CV-derived values.
                sqlx::query("insert into applicant_facts (owner_id,key,value,category,approved,approved_at,provenance,source_document_id) values ($1,$2,$3,$4,true,now(),'cv',$5) on conflict (owner_id,key) where superseded_by is null do update set value=excluded.value, approved=true, approved_at=now(), source_document_id=excluded.source_document_id, version=applicant_facts.version+1 where applicant_facts.source_document_id is not null")
                    .bind(owner).bind(key).bind(value).bind(category).bind(id).execute(&mut *tx).await?;
            }
        }
        // An existing token may refer to an earlier CV. Require a fresh authorization for it.
        sqlx::query("update authorization_tokens set expires_at=now() where used_at is null").execute(&mut *tx).await?;
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

pub async fn current_path(pool: &Db) -> Result<Option<String>, DocumentError> {
    Ok(sqlx::query_scalar("select file_path from documents where kind='cv' and approved order by created_at desc limit 1").fetch_optional(pool).await?)
}

use opportunity_domain::{AcademicSelectors, SourceEndpoint};
use serde::Deserialize;

use crate::db::Db;
use crate::sources::{self, NewSource, SourceError};

/// On-disk shape of `config/sources.toml`. Each entry carries only the identifiers its provider documents, so importing a file can never produce a source built from a guessed employer name.
#[derive(Debug, Deserialize)]
pub struct SourcesFile {
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
}

#[derive(Debug, Deserialize)]
pub struct SourceEntry {
    pub key: String,
    pub kind: String,
    pub organization: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_refresh_hours")]
    pub refresh_hours: i32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub board_token: Option<String>,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default)]
    pub board_slug: Option<String>,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Academic sources only: the name of a built-in selector preset.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Academic sources only: explicit CSS selectors, which take precedence over a preset.
    #[serde(default)]
    pub selectors: Option<AcademicSelectors>,
}

fn default_refresh_hours() -> i32 {
    6
}

impl SourceEntry {
    pub fn endpoint(&self) -> Result<SourceEndpoint, String> {
        let missing = |field: &str| {
            format!(
                "source '{}' of kind '{}' is missing {field}",
                self.key, self.kind
            )
        };
        Ok(match self.kind.as_str() {
            "greenhouse" => SourceEndpoint::Greenhouse {
                board_token: self
                    .board_token
                    .clone()
                    .ok_or_else(|| missing("board_token"))?,
            },
            "lever" => SourceEndpoint::Lever {
                site_id: self.site_id.clone().ok_or_else(|| missing("site_id"))?,
            },
            "ashby" => SourceEndpoint::Ashby {
                board_slug: self
                    .board_slug
                    .clone()
                    .ok_or_else(|| missing("board_slug"))?,
            },
            "workday" => SourceEndpoint::Workday {
                tenant: self.tenant.clone().ok_or_else(|| missing("tenant"))?,
                instance: self.instance.clone().ok_or_else(|| missing("instance"))?,
                site: self.site.clone().ok_or_else(|| missing("site"))?,
            },
            "academic_html" => SourceEndpoint::AcademicHtml {
                url: self.url.clone().ok_or_else(|| missing("url"))?,
                pattern: self.pattern.clone(),
                selectors: self.selectors.clone().map(Box::new),
            },
            "manual" => SourceEndpoint::Manual {
                url: self.url.clone().ok_or_else(|| missing("url"))?,
            },
            other => return Err(format!("unknown source kind '{other}'")),
        })
    }
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub created: Vec<String>,
    pub skipped_existing: Vec<String>,
    /// Seeded by an earlier release and since deleted by the owner, so left deleted.
    pub skipped_previously_seeded: Vec<String>,
    pub rejected: Vec<(String, String)>,
}

/// Import a source file the owner chose to import. Entries that already exist are left alone so an import never overwrites an edit the owner made in the UI, and entries that fail validation are reported rather than silently dropped. Being the owner's explicit act, it may recreate a source they deleted.
pub async fn import_sources(pool: &Db, toml_text: &str) -> Result<ImportReport, SourceError> {
    import(pool, toml_text, false).await
}

/// The seed compiled into a release, imported at every start. It adds sources a new release
/// introduces and never recreates one the owner deleted: each key is created at most once.
pub async fn import_release_seed(pool: &Db, toml_text: &str) -> Result<ImportReport, SourceError> {
    import(pool, toml_text, true).await
}

async fn import(
    pool: &Db,
    toml_text: &str,
    release_seed: bool,
) -> Result<ImportReport, SourceError> {
    let parsed: SourcesFile =
        toml::from_str(toml_text).map_err(|e| SourceError::Invalid(e.to_string()))?;
    let mut report = ImportReport::default();
    for entry in parsed.sources {
        let endpoint = match entry.endpoint() {
            Ok(endpoint) => endpoint,
            Err(reason) => {
                report.rejected.push((entry.key.clone(), reason));
                continue;
            }
        };
        if let Err(reason) = endpoint.validate() {
            report.rejected.push((entry.key.clone(), reason));
            continue;
        }
        if sources::get_by_key(pool, &entry.key).await?.is_some() {
            report.skipped_existing.push(entry.key.clone());
            continue;
        }
        let seeded: bool =
            sqlx::query_scalar("select exists(select 1 from seed_ledger where key = $1)")
                .bind(&entry.key)
                .fetch_one(pool)
                .await
                .map_err(SourceError::from)?;
        if release_seed && seeded {
            report.skipped_previously_seeded.push(entry.key.clone());
            continue;
        }
        let display_name = match &entry.notes {
            Some(notes) if !notes.trim().is_empty() => {
                format!("{} ({})", entry.organization, notes.trim())
            }
            _ => entry.organization.clone(),
        };
        let new_source = NewSource {
            key: entry.key.clone(),
            display_name,
            organization: entry.organization.clone(),
            endpoint,
            enabled: entry.enabled,
            refresh_hours: entry.refresh_hours,
            tags: entry.tags.clone(),
        };
        match sources::create(pool, new_source).await {
            Ok(_) => {
                sqlx::query("insert into seed_ledger (key) values ($1) on conflict do nothing")
                    .bind(&entry.key)
                    .execute(pool)
                    .await
                    .map_err(SourceError::from)?;
                report.created.push(entry.key)
            }
            Err(SourceError::Invalid(reason)) => report.rejected.push((entry.key, reason)),
            Err(other) => return Err(other),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_template_file_parses_and_every_placeholder_entry_is_rejected() {
        let text = include_str!("../../../config/sources.example.toml");
        let parsed: SourcesFile = toml::from_str(text).unwrap();
        assert!(!parsed.sources.is_empty());
        for entry in &parsed.sources {
            let endpoint = entry
                .endpoint()
                .expect("every template entry declares its own identifier fields");
            if entry.key.starts_with("example-") {
                assert!(
                    endpoint.validate().is_err(),
                    "template entry {} must not validate",
                    entry.key
                );
            }
        }
    }

    #[test]
    fn the_verified_seed_file_parses_and_every_entry_validates() {
        let text = include_str!("../../../config/sources.seed.toml");
        let parsed: SourcesFile = toml::from_str(text).unwrap();
        assert!(!parsed.sources.is_empty());
        for entry in &parsed.sources {
            let endpoint = entry.endpoint().unwrap_or_else(|e| panic!("{e}"));
            endpoint
                .validate()
                .unwrap_or_else(|e| panic!("seed entry {} is invalid: {e}", entry.key));
        }
    }
}

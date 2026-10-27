//! Profile and exclusion-rule storage. All of it is user-owned private data.

use chrono::{DateTime, Utc};
use opportunity_domain::matching::{ExclusionKind, ExclusionRule};
use opportunity_domain::{normalize_organization, Profile, RemotePolicy, SeniorityPolicy, Weights};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Db;

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile is invalid: {0}")]
    Invalid(String),
    #[error("no profile with id {0}")]
    NotFound(Uuid),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

/// A stored profile, which is the domain profile plus its identity and timestamps.
#[derive(Debug, Clone, Serialize)]
pub struct StoredProfile {
    pub id: Uuid,
    #[serde(flatten)]
    pub profile: Profile,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct ProfileRow {
    id: Uuid,
    key: String,
    display_name: String,
    priority: i32,
    enabled: bool,
    job_kinds: Vec<String>,
    terms: Vec<String>,
    negative_terms: Vec<String>,
    preferred_technologies: Vec<String>,
    excluded_technologies: Vec<String>,
    preferred_countries: Vec<String>,
    preferred_cities: Vec<String>,
    current_country: Option<String>,
    relocation_willing: bool,
    remote_policy: String,
    include_unknown_location: bool,
    min_years_experience: Option<i32>,
    max_years_experience: Option<i32>,
    seniority_policy: String,
    education_level: Option<String>,
    work_authorization: Vec<String>,
    weights: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ProfileRow {
    fn into_stored(self) -> StoredProfile {
        StoredProfile {
            id: self.id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            profile: Profile {
                key: self.key,
                display_name: self.display_name,
                priority: self.priority,
                enabled: self.enabled,
                job_kinds: self.job_kinds,
                terms: self.terms,
                negative_terms: self.negative_terms,
                preferred_technologies: self.preferred_technologies,
                excluded_technologies: self.excluded_technologies,
                preferred_countries: self.preferred_countries,
                preferred_cities: self.preferred_cities,
                current_country: self.current_country,
                relocation_willing: self.relocation_willing,
                // A stored value that no longer parses falls back to the permissive default rather than failing the whole listing.
                remote_policy: self.remote_policy.parse().unwrap_or(RemotePolicy::Any),
                include_unknown_location: self.include_unknown_location,
                min_years_experience: self.min_years_experience,
                max_years_experience: self.max_years_experience,
                seniority_policy: self
                    .seniority_policy
                    .parse()
                    .unwrap_or(SeniorityPolicy::Preference),
                education_level: self.education_level,
                work_authorization: self.work_authorization,
                weights: serde_json::from_value(self.weights).unwrap_or_default(),
            },
        }
    }
}

const COLUMNS: &str = "id, key, display_name, priority, enabled, job_kinds, terms, negative_terms,
    preferred_technologies, excluded_technologies, preferred_countries, preferred_cities,
    current_country, relocation_willing, remote_policy, include_unknown_location,
    min_years_experience, max_years_experience, seniority_policy, education_level,
    work_authorization, weights, created_at, updated_at";

/// The single local owner. Multi-user evolution replaces this lookup without changing any table.
pub async fn local_owner(pool: &Db) -> Result<Uuid, ProfileError> {
    let id: Uuid = sqlx::query_scalar("select id from owners order by created_at limit 1")
        .fetch_one(pool)
        .await?;
    Ok(id)
}

pub async fn list(pool: &Db) -> Result<Vec<StoredProfile>, ProfileError> {
    let rows = sqlx::query_as::<_, ProfileRow>(&format!(
        "select {COLUMNS} from profiles order by priority, key"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ProfileRow::into_stored).collect())
}

pub async fn get(pool: &Db, id: Uuid) -> Result<StoredProfile, ProfileError> {
    let row =
        sqlx::query_as::<_, ProfileRow>(&format!("select {COLUMNS} from profiles where id = $1"))
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or(ProfileError::NotFound(id))?;
    Ok(row.into_stored())
}

pub async fn get_by_key(pool: &Db, key: &str) -> Result<Option<StoredProfile>, ProfileError> {
    let row =
        sqlx::query_as::<_, ProfileRow>(&format!("select {COLUMNS} from profiles where key = $1"))
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(ProfileRow::into_stored))
}

pub async fn create(pool: &Db, profile: Profile) -> Result<StoredProfile, ProfileError> {
    profile.validate().map_err(ProfileError::Invalid)?;
    let owner_id = local_owner(pool).await?;
    let row = sqlx::query_as::<_, ProfileRow>(&format!(
        "insert into profiles (owner_id, key, display_name, priority, enabled, job_kinds, terms,
            negative_terms, preferred_technologies, excluded_technologies, preferred_countries,
            preferred_cities, current_country, relocation_willing, remote_policy,
            include_unknown_location, min_years_experience, max_years_experience, seniority_policy,
            education_level, work_authorization, weights)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)
         returning {COLUMNS}"
    ))
    .bind(owner_id)
    .bind(profile.key.trim())
    .bind(profile.display_name.trim())
    .bind(profile.priority)
    .bind(profile.enabled)
    .bind(&profile.job_kinds)
    .bind(&profile.terms)
    .bind(&profile.negative_terms)
    .bind(&profile.preferred_technologies)
    .bind(&profile.excluded_technologies)
    .bind(&profile.preferred_countries)
    .bind(&profile.preferred_cities)
    .bind(profile.current_country.as_deref())
    .bind(profile.relocation_willing)
    .bind(profile.remote_policy.as_str())
    .bind(profile.include_unknown_location)
    .bind(profile.min_years_experience)
    .bind(profile.max_years_experience)
    .bind(profile.seniority_policy.as_str())
    .bind(profile.education_level.as_deref())
    .bind(&profile.work_authorization)
    .bind(serde_json::to_value(profile.weights)?)
    .fetch_one(pool)
    .await?;
    Ok(row.into_stored())
}

/// Replace a profile's editable content. The whole profile is sent, which keeps the editor simple and avoids a partial update silently dropping a field.
pub async fn update(pool: &Db, id: Uuid, profile: Profile) -> Result<StoredProfile, ProfileError> {
    profile.validate().map_err(ProfileError::Invalid)?;
    let row = sqlx::query_as::<_, ProfileRow>(&format!(
        "update profiles set
            key = $2, display_name = $3, priority = $4, enabled = $5, job_kinds = $6, terms = $7,
            negative_terms = $8, preferred_technologies = $9, excluded_technologies = $10,
            preferred_countries = $11, preferred_cities = $12, current_country = $13,
            relocation_willing = $14, remote_policy = $15, include_unknown_location = $16,
            min_years_experience = $17, max_years_experience = $18, seniority_policy = $19,
            education_level = $20, work_authorization = $21, weights = $22, updated_at = now()
         where id = $1 returning {COLUMNS}"
    ))
    .bind(id)
    .bind(profile.key.trim())
    .bind(profile.display_name.trim())
    .bind(profile.priority)
    .bind(profile.enabled)
    .bind(&profile.job_kinds)
    .bind(&profile.terms)
    .bind(&profile.negative_terms)
    .bind(&profile.preferred_technologies)
    .bind(&profile.excluded_technologies)
    .bind(&profile.preferred_countries)
    .bind(&profile.preferred_cities)
    .bind(profile.current_country.as_deref())
    .bind(profile.relocation_willing)
    .bind(profile.remote_policy.as_str())
    .bind(profile.include_unknown_location)
    .bind(profile.min_years_experience)
    .bind(profile.max_years_experience)
    .bind(profile.seniority_policy.as_str())
    .bind(profile.education_level.as_deref())
    .bind(&profile.work_authorization)
    .bind(serde_json::to_value(profile.weights)?)
    .fetch_optional(pool)
    .await?
    .ok_or(ProfileError::NotFound(id))?;
    Ok(row.into_stored())
}

pub async fn delete(pool: &Db, id: Uuid) -> Result<(), ProfileError> {
    let affected = sqlx::query("delete from profiles where id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(ProfileError::NotFound(id));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StoredRule {
    pub id: Uuid,
    pub profile_id: Option<Uuid>,
    pub kind: String,
    pub value: String,
    pub opportunity_id: Option<Uuid>,
    pub note: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewRule {
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub profile_id: Option<Uuid>,
    #[serde(default)]
    pub opportunity_id: Option<Uuid>,
    #[serde(default)]
    pub note: Option<String>,
}

fn parse_kind(kind: &str) -> Result<ExclusionKind, ProfileError> {
    Ok(match kind {
        "company" => ExclusionKind::Company,
        "title_term" => ExclusionKind::TitleTerm,
        "description_term" => ExclusionKind::DescriptionTerm,
        "hidden" => ExclusionKind::Hidden,
        other => {
            return Err(ProfileError::Invalid(format!(
                "unknown exclusion kind '{other}'"
            )))
        }
    })
}

pub async fn list_rules(pool: &Db) -> Result<Vec<StoredRule>, ProfileError> {
    let rows = sqlx::query_as::<_, StoredRule>(
        "select id, profile_id, kind, value, opportunity_id, note, enabled, created_at
         from exclusion_rules order by kind, value",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn create_rule(pool: &Db, rule: NewRule) -> Result<StoredRule, ProfileError> {
    let kind = parse_kind(&rule.kind)?;
    if kind == ExclusionKind::Hidden && rule.opportunity_id.is_none() {
        return Err(ProfileError::Invalid(
            "hiding an opportunity needs its id".into(),
        ));
    }
    if kind != ExclusionKind::Hidden && rule.value.trim().is_empty() {
        return Err(ProfileError::Invalid("rule value is empty".into()));
    }
    // A company rule is stored under its folded identity so matching is exact on the employer rather than on a substring of its name.
    let normalized = match kind {
        ExclusionKind::Company => normalize_organization(&rule.value),
        _ => opportunity_domain::normalize_location(Some(&rule.value)),
    };
    let owner_id = local_owner(pool).await?;
    let row = sqlx::query_as::<_, StoredRule>(
        "insert into exclusion_rules (owner_id, profile_id, kind, value, normalized_value, opportunity_id, note)
         values ($1,$2,$3,$4,$5,$6,$7)
         on conflict (owner_id, opportunity_id) where kind = 'hidden'
         do update set note = excluded.note, enabled = true
         returning id, profile_id, kind, value, opportunity_id, note, enabled, created_at",
    )
    .bind(owner_id)
    .bind(rule.profile_id)
    .bind(kind.as_str())
    .bind(rule.value.trim())
    .bind(&normalized)
    .bind(rule.opportunity_id)
    .bind(rule.note.as_deref())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_rule(pool: &Db, id: Uuid) -> Result<(), ProfileError> {
    let affected = sqlx::query("delete from exclusion_rules where id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(ProfileError::NotFound(id));
    }
    Ok(())
}

/// Rules that apply when scoring for one profile: the global ones plus that profile's own.
pub async fn rules_for(
    pool: &Db,
    profile_id: Option<Uuid>,
) -> Result<Vec<(ExclusionRule, Option<Uuid>)>, ProfileError> {
    let rows = sqlx::query_as::<_, StoredRule>(
        "select id, profile_id, kind, value, opportunity_id, note, enabled, created_at
         from exclusion_rules
         where enabled and (profile_id is null or profile_id = $1)",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = parse_kind(&row.kind)?;
        out.push((
            ExclusionRule {
                kind,
                value: row.value,
                note: row.note,
            },
            row.opportunity_id,
        ));
    }
    Ok(out)
}

/// Seed the four starting profiles described in the build contract. Each is a neutral technical track with no personal detail; the owner edits them in the interface. Existing profiles are never overwritten.
pub async fn seed_default_profiles(pool: &Db) -> Result<Vec<String>, ProfileError> {
    let mut created = Vec::new();
    for (key, name, priority, kinds, terms) in default_profiles() {
        if get_by_key(pool, key).await?.is_some() {
            continue;
        }
        let mut profile = Profile::blank(key, name);
        profile.priority = priority;
        profile.job_kinds = kinds.iter().map(|k| (*k).to_owned()).collect();
        profile.terms = terms.iter().map(|t| (*t).to_owned()).collect();
        profile.weights = Weights::default();
        create(pool, profile).await?;
        created.push(key.to_owned());
    }
    Ok(created)
}

type DefaultProfile = (
    &'static str,
    &'static str,
    i32,
    &'static [&'static str],
    &'static [&'static str],
);

/// The five career tracks the product offers. Deliberately generic: these are technical starting points, not anyone's personal preferences, and they carry no location, authorization or experience data.
fn default_profiles() -> Vec<DefaultProfile> {
    vec![
        (
            "firmware",
            "Firmware and platform systems",
            10,
            &[],
            &[
                "firmware",
                "embedded",
                "RTOS",
                "device driver",
                "bootloader",
                "SoC",
            ],
        ),
        (
            "memory",
            "Memory and operating systems",
            20,
            &[],
            &[
                "memory",
                "DRAM",
                "CXL",
                "kernel",
                "operating systems",
                "Linux",
                "cache",
            ],
        ),
        (
            "architecture",
            "CPU architecture and RTL",
            30,
            &[],
            &[
                "CPU",
                "RTL",
                "RISC-V",
                "microarchitecture",
                "SystemVerilog",
                "verification",
            ],
        ),
        (
            "ai-systems",
            "AI systems and hardware/software co-design",
            40,
            &[],
            &[
                "accelerator",
                "GPU",
                "compiler",
                "CUDA",
                "machine learning",
                "co-design",
            ],
        ),
        (
            "research",
            "Research, master's thesis and PhD opportunities",
            50,
            &["master_thesis", "research_assistant", "internship", "phd"],
            &["research", "thesis", "PhD", "Masterarbeit", "doctoral"],
        ),
    ]
}

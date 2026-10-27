//! The single-screen product journey, backed by the existing discovery and application stores.
use crate::{
    applications,
    candidate::{self, CandidateProfile},
    documents, matching, profiles, workflow, Db,
};
use applications::AppError;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

pub const TRACKS: &[(&str, &str)] = &[
    ("firmware", "Firmware and platform systems"),
    ("memory", "Memory and operating systems"),
    ("architecture", "CPU architecture and RTL"),
    ("ai-systems", "AI systems and hardware/software co-design"),
    (
        "research",
        "Research, master's thesis and PhD opportunities",
    ),
];

pub async fn initialize(db: &Db) -> anyhow::Result<()> {
    profiles::seed_default_profiles(db).await?;
    crate::config::import_sources(db, include_str!("../../../config/sources.seed.toml")).await?;
    Ok(())
}

pub async fn current_candidate(db: &Db) -> Result<Option<(Uuid, CandidateProfile)>, AppError> {
    let row: Option<(Uuid, Option<Value>)> = sqlx::query_as(
        "select id,candidate from documents where kind='cv' order by created_at desc limit 1",
    )
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|(id, v)| {
        v.and_then(|v| serde_json::from_value(v).ok())
            .map(|p| (id, p))
    }))
}

pub async fn selection(db: &Db) -> Result<Uuid, AppError> {
    let saved: Option<Uuid> = sqlx::query_scalar("select profile_id from product_settings where owner_id=(select id from owners order by created_at limit 1)").fetch_optional(db).await?.flatten();
    if let Some(id) = saved {
        return Ok(id);
    }
    Ok(profiles::get_by_key(db, "firmware")
        .await?
        .ok_or_else(|| AppError::Invalid("Search profiles are not ready.".into()))?
        .id)
}

pub async fn select(db: &Db, id: Uuid) -> Result<(), AppError> {
    let profile = profiles::get(db, id).await?;
    if !TRACKS.iter().any(|(k, _)| *k == profile.profile.key) {
        return Err(AppError::Invalid(
            "Choose one of the five career profiles.".into(),
        ));
    }
    let owner = profiles::local_owner(db).await?;
    sqlx::query("insert into product_settings(owner_id,profile_id) values($1,$2) on conflict(owner_id) do update set profile_id=$2,updated_at=now()")
        .bind(owner).bind(id).execute(db).await?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct RankedJob {
    #[serde(flatten)]
    pub job: matching::ScoredRow,
    pub score: f32,
    pub explanation: String,
    pub gaps: Vec<String>,
    pub auto_eligible: bool,
    pub source_key: String,
    pub provider: String,
}

fn direction(key: &str) -> &'static [&'static str] {
    match key {
        "firmware" => &[
            "firmware",
            "embedded",
            "RTOS",
            "bootloader",
            "device driver",
            "platform systems",
        ],
        "memory" => &[
            "memory",
            "DRAM",
            "CXL",
            "DDR",
            "kernel",
            "operating systems",
            "Linux",
            "storage",
        ],
        "architecture" => &[
            "CPU",
            "RTL",
            "microarchitecture",
            "RISC-V",
            "Verilog",
            "verification",
            "ASIC",
            "architecture",
        ],
        "ai-systems" => &[
            "AI",
            "GPU",
            "accelerator",
            "compiler",
            "CUDA",
            "machine learning",
            "co-design",
            "inference",
        ],
        _ => &[
            "research",
            "thesis",
            "doctoral",
            "PhD",
            "Masterarbeit",
            "internship",
        ],
    }
}

fn fit(
    cv: &CandidateProfile,
    key: &str,
    job: &matching::ScoredRow,
) -> (f32, String, Vec<String>, bool) {
    let description = job.description_text.as_deref().unwrap_or("");
    let text = format!("{} {description}", job.canonical_title);
    let normalized_title = opportunity_domain::normalize_location(Some(&job.canonical_title));
    let normalized_description = opportunity_domain::normalize_location(Some(description));
    let normalized_text = format!("{normalized_title} {normalized_description}");
    let contains = |hay: &str, term: &str| {
        opportunity_domain::normalize::contains_phrase(
            hay,
            &opportunity_domain::normalize_location(Some(term)),
        )
    };
    let title_matches = direction(key)
        .iter()
        .filter(|w| contains(&normalized_title, w))
        .count();
    let body_matches = direction(key)
        .iter()
        .filter(|w| contains(&normalized_description, w))
        .count();
    let skills: Vec<_> = cv
        .skills
        .iter()
        .filter(|s| {
            if s.as_str() == "C++" || s.as_str() == "C" {
                candidate::mentions(&text, s)
            } else {
                contains(&normalized_text, s)
            }
        })
        .collect();
    let mut gaps = Vec::new();
    let title = job.canonical_title.to_lowercase();
    let senior = [
        "senior",
        "staff",
        "principal",
        "director",
        "manager",
        "lead",
    ]
    .iter()
    .any(|w| candidate::mentions(&title, w));
    let years = required_years(description);
    if let Some(required) = years {
        if cv.experience_years.unwrap_or(0) < required {
            gaps.push(format!(
                "{required}+ years requested; not established by your CV"
            ));
        }
    } else if senior && cv.experience_years.unwrap_or(0) < 5 {
        gaps.push("Senior experience is not established by your CV".into());
    }
    let lower = description.to_lowercase();
    if ["phd required", "ph.d. required", "must have a phd"]
        .iter()
        .any(|s| lower.contains(s))
        && cv.degree.as_deref() != Some("PhD")
    {
        gaps.push("A completed PhD is required".into());
    }
    if [
        "citizen",
        "security clearance",
        "right to work",
        "work authorization",
        "no sponsorship",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        gaps.push("Work eligibility needs confirmation".into());
    }
    if job.title_only {
        gaps.push("Full requirements unavailable".into());
    }
    let evidence = skills
        .iter()
        .take(4)
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let relevance = title_matches > 0 || body_matches >= 2;
    let score = (if title_matches > 0 {
        40.0
    } else if body_matches >= 2 {
        20.0
    } else {
        0.0
    }) + (skills.len() as f32 * 7.0).min(35.0)
        + if !cv.projects.is_empty() && skills.len() >= 2 {
            8.0
        } else {
            0.0
        }
        + if !cv.employment.is_empty() && skills.len() >= 2 {
            8.0
        } else {
            0.0
        }
        + if (Utc::now() - job.last_seen_at).num_days() < 2 {
            5.0
        } else {
            0.0
        }
        - gaps.len() as f32 * 10.0;
    let explanation = if evidence.is_empty() {
        "Limited evidence in your current CV".into()
    } else {
        format!("Your CV: {evidence}")
    };
    let automatic = relevance && skills.len() >= 2 && gaps.is_empty() && score >= 55.0;
    (
        if relevance { score.max(0.0) } else { 0.0 },
        explanation,
        gaps,
        automatic,
    )
}

fn required_years(text: &str) -> Option<i32> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    words
        .windows(2)
        .filter_map(|w| {
            if !w[1].starts_with("year") {
                return None;
            }
            w[0].trim_matches(|c: char| !c.is_ascii_digit())
                .parse::<i32>()
                .ok()
                .filter(|n| (1..=30).contains(n))
        })
        .max()
}

pub async fn ranked(db: &Db, profile_id: Uuid) -> Result<Vec<RankedJob>, AppError> {
    let Some((_, cv)) = current_candidate(db).await? else {
        return Ok(vec![]);
    };
    let profile = profiles::get(db, profile_id).await?;
    let rows = matching::shortlist(db, &matching::MatchQuery::default()).await?;
    let sources: Vec<(Uuid,String,String)>=sqlx::query_as("select distinct on(p.opportunity_id) p.opportunity_id,s.key,s.kind from postings p join sources s on s.id=p.source_id where p.closed_at is null and p.opportunity_id is not null order by p.opportunity_id,p.url")
        .fetch_all(db).await?;
    let mut ranked = Vec::new();
    for row in rows {
        let (score, explanation, gaps, auto_eligible) = fit(&cv, &profile.profile.key, &row);
        if score < 20.0 {
            continue;
        }
        let Some((_, source_key, provider)) = sources.iter().find(|s| s.0 == row.id) else {
            continue;
        };
        ranked.push(RankedJob {
            job: row,
            score,
            explanation,
            gaps,
            auto_eligible,
            source_key: source_key.clone(),
            provider: provider.clone(),
        });
    }
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.job.last_seen_at.cmp(&a.job.last_seen_at))
            .then(a.job.id.cmp(&b.job.id))
    });
    Ok(ranked)
}

#[derive(Serialize, FromRow)]
pub struct ProductPolicy {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub source_keys: Vec<String>,
    pub profile_id: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub async fn dashboard(db: &Db) -> Result<Value, AppError> {
    let profile_id = selection(db).await?;
    let cv = documents::list(db)
        .await
        .map_err(|e| AppError::Invalid(e.to_string()))?
        .into_iter()
        .next();
    let all = profiles::list(db).await?;
    let profiles: Vec<_> = TRACKS
        .iter()
        .filter_map(|(key, label)| {
            all.iter()
                .find(|p| p.profile.key == *key)
                .map(|p| json!({"id":p.id,"name":label}))
        })
        .collect();
    let mut jobs = ranked(db, profile_id).await?;
    let matches = jobs.len();
    jobs.truncate(60);
    let applications:Vec<Value>=sqlx::query_scalar("select jsonb_build_object('id',a.id,'opportunity_id',a.opportunity_id,'status',a.status,'reason',a.route_reason,'title',o.canonical_title,'employer',org.display_name,'url',(select p.url from postings p where p.opportunity_id=o.id order by p.closed_at nulls first limit 1)) from applications a join opportunities o on o.id=a.opportunity_id join organizations org on org.id=o.organization_id order by a.updated_at desc limit 100").fetch_all(db).await?;
    let policies=sqlx::query_as::<_,ProductPolicy>("select id,name,enabled,source_keys,profile_id,expires_at from authorization_policies where profile_id is not null and revoked_at is null order by created_at desc").fetch_all(db).await?;
    let last_updated:Option<DateTime<Utc>>=sqlx::query_scalar("select max(finished_at) from source_runs where outcome in ('ok_complete','ok_verified_empty','partial','not_modified')").fetch_one(db).await?;
    let discovered: i64 =
        sqlx::query_scalar("select count(*) from opportunities where closed_at is null and (deadline_at is null or deadline_at>now())")
            .fetch_one(db)
            .await?;
    let questions = applications::list_review_tasks(db, None).await?;
    let mail:Vec<Value>=sqlx::query_scalar("select jsonb_build_object('application_id',application_id,'state',delivery_state,'error',delivery_error) from notifications where kind='submitted' order by created_at desc limit 100").fetch_all(db).await?;
    Ok(
        json!({"cv":cv,"profiles":profiles,"selected_profile":profile_id,"jobs":jobs,"matches":matches,"discovered":discovered,"applications":applications,"questions":questions,"policies":policies,"last_updated":last_updated,"mail":mail,"email_configured":crate::mail::configured(),"cv_ready":current_candidate(db).await?.is_some()}),
    )
}

pub async fn authorize_source(db: &Db, source_key: &str, profile_id: Uuid) -> Result<(), AppError> {
    select(db, profile_id).await?;
    let source = crate::sources::get_by_key(db, source_key)
        .await
        .map_err(|e| AppError::Invalid(e.to_string()))?
        .ok_or_else(|| AppError::Invalid("Unknown employer".into()))?;
    let previous:Vec<Uuid>=sqlx::query_scalar("select id from authorization_policies where profile_id=$1 and $2=any(source_keys) and revoked_at is null").bind(profile_id).bind(source_key).fetch_all(db).await?;
    for id in previous {
        applications::set_policy_state(db, id, false, true).await?;
    }
    let policy = applications::create_policy(
        db,
        applications::NewPolicy {
            name: format!("{} · 3/day · 30 days", source.display_name),
            source_keys: vec![source.key],
            max_per_day: Some(3),
            max_total: Some(30),
            expires_in_days: Some(30),
        },
    )
    .await?;
    sqlx::query("update authorization_policies set profile_id=$2,enabled=true,forbid_free_text=false where id=$1").bind(policy.id).bind(profile_id).execute(db).await?;
    Ok(())
}

pub async fn enqueue(db: &Db, opportunity_id: Uuid) -> Result<Uuid, AppError> {
    if current_candidate(db).await?.is_none() {
        return Err(AppError::Invalid("Upload a readable CV first.".into()));
    }
    let app =
        applications::create_application(db, opportunity_id, Some(selection(db).await?)).await?;
    if ["submitted", "submission_unknown", "submitting", "preparing"].contains(&app.status.as_str())
    {
        return Ok(app.id);
    }
    sqlx::query(
        "update applications set status='saved',auto_resume=true,updated_at=now() where id=$1",
    )
    .bind(app.id)
    .execute(db)
    .await?;
    Ok(app.id)
}

pub async fn process_application(db: &Db, id: Uuid) -> Result<(), AppError> {
    let app = applications::get_application(db, id).await?;
    if !["saved", "ready"].contains(&app.status.as_str()) {
        return Ok(());
    }
    let destination:Option<(String,String,String)>=sqlx::query_as("select p.url,s.key,s.kind from postings p join sources s on s.id=p.source_id join opportunities o on o.id=p.opportunity_id where p.opportunity_id=$1 and p.closed_at is null and o.closed_at is null and (o.deadline_at is null or o.deadline_at>now()) order by p.url limit 1")
        .bind(app.opportunity_id).fetch_optional(db).await?;
    let Some((url, key, provider)) = destination else {
        applications::set_status(db, id, "unsupported", json!({})).await?;
        sqlx::query("update applications set route_reason='This opportunity is no longer available.' where id=$1").bind(id).execute(db).await?;
        return Ok(());
    };
    let cv:Option<(Uuid,String)>=sqlx::query_as("select id,file_path from documents where kind='cv' and approved and candidate is not null order by created_at desc limit 1").fetch_optional(db).await?;
    let Some((cv_id, path)) = cv else {
        return Ok(());
    };
    sqlx::query("update applications set cv_document_id=$2 where id=$1")
        .bind(id)
        .bind(cv_id)
        .execute(db)
        .await?;
    // Real browser forms are supported only for these providers; no login or CAPTCHA bypass.
    if !["greenhouse", "lever", "ashby"].contains(&provider.as_str()) {
        applications::set_status(db, id, "unsupported", json!({})).await?;
        sqlx::query("update applications set route_reason='Complete this application on the employer website.' where id=$1").bind(id).execute(db).await?;
        return Ok(());
    }
    let prepared = match workflow::prepare(db, id, &url).await {
        Ok(p) => p,
        Err(error) => {
            if error.to_string().contains("source refresh") {
                return Ok(());
            }
            applications::set_status(db, id, "unsupported", json!({})).await?;
            sqlx::query("update applications set route_reason=$2 where id=$1")
                .bind(id)
                .bind({
                    tracing::warn!(%error, "form unavailable");
                    "This employer form requires manual completion.".to_owned()
                })
                .execute(db)
                .await?;
            return Ok(());
        }
    };
    if prepared.plan.has_captcha
        || prepared.plan.requires_login
        || prepared.plan.submit_selector.is_none()
        || prepared.plan.fields.is_empty()
        || prepared
            .plan
            .fields
            .iter()
            .any(|f| f.required && f.kind == opportunity_domain::FieldKind::Unsupported)
    {
        applications::set_status(db, id, "unsupported", json!({})).await?;
        sqlx::query("update applications set route_reason='The employer requires manual completion or a verification challenge.' where id=$1").bind(id).execute(db).await?;
        return Ok(());
    }
    if !prepared.mapping.open.is_empty() {
        return Ok(());
    }
    let permitted:bool=sqlx::query_scalar("select exists(select 1 from authorization_policies where enabled and revoked_at is null and expires_at>now() and profile_id=$1 and $2=any(source_keys))")
        .bind(app.profile_id).bind(&key).fetch_one(db).await?;
    if permitted {
        if let Err(error) = workflow::submit(db, id, &url, &key, Some(path)).await {
            // Quotas or revoked authorization leave it ready; no submission is claimed.
            sqlx::query("update applications set route_reason=$2 where id=$1")
                .bind(id)
                .bind(error.to_string())
                .execute(db)
                .await?;
        }
    }
    Ok(())
}

pub async fn tick(db: &Db) -> Result<(), AppError> {
    // A separate transaction lock prevents two scheduler processes creating the same queue work.
    let mut lock = db.begin().await?;
    let acquired: bool = sqlx::query_scalar("select pg_try_advisory_xact_lock(918273645)")
        .fetch_one(&mut *lock)
        .await?;
    if !acquired {
        return Ok(());
    }
    // Worker operations have a two-minute deadline. A stale claim after a crash is never resubmitted.
    sqlx::query("update application_attempts set state='unknown',diagnostic='Application interrupted; receipt must be checked with the employer.',finished_at=now() where state in ('started','filled') and started_at<now()-interval '10 minutes'").execute(db).await?;
    sqlx::query("update applications set status='submission_unknown',updated_at=now() where status='submitting' and updated_at<now()-interval '10 minutes'").execute(db).await?;
    sqlx::query("update applications set status='saved',updated_at=now() where status='preparing' and updated_at<now()-interval '10 minutes' and not exists(select 1 from application_attempts t where t.application_id=applications.id and t.state in ('started','submitted','unknown'))").execute(db).await?;
    let policy_profiles:Vec<Uuid>=sqlx::query_scalar("select distinct profile_id from authorization_policies where enabled and revoked_at is null and expires_at>now() and profile_id is not null").fetch_all(db).await?;
    for profile_id in policy_profiles {
        for job in ranked(db, profile_id)
            .await?
            .into_iter()
            .filter(|j| j.auto_eligible)
            .take(30)
        {
            let allowed:bool=sqlx::query_scalar("select exists(select 1 from authorization_policies where enabled and revoked_at is null and expires_at>now() and profile_id=$1 and $2=any(source_keys))").bind(profile_id).bind(&job.source_key).fetch_one(db).await?;
            if allowed {
                let app =
                    applications::create_application(db, job.job.id, Some(profile_id)).await?;
                sqlx::query(
                    "update applications set auto_resume=true where id=$1 and status='saved'",
                )
                .bind(app.id)
                .execute(db)
                .await?;
            }
        }
    }
    let queued:Vec<Uuid>=sqlx::query_scalar("select id from applications where auto_resume and (status='saved' or (status='ready' and updated_at<now()-interval '10 minutes')) order by updated_at limit 2").fetch_all(db).await?;
    for id in queued {
        process_application(db, id).await?;
    }
    lock.commit().await?;
    Ok(())
}

pub fn spawn(db: Db) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = tick(&db).await {
                tracing::warn!(%error,"application queue");
            }
            if let Err(error) = crate::mail::dispatch(&db).await {
                tracing::warn!(%error,"email delivery");
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requirements_are_not_cv_qualifications() {
        assert_eq!(required_years("Requires 7+ years of experience"), Some(7));
        assert_eq!(required_years("Our company was founded in 2001"), None);
    }
}

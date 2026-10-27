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

/// Names the ranking rules in API output, so a score can be traced to the policy that produced it.
pub const RANKING_POLICY: &str = "career-priority-v2";

#[derive(Debug, Serialize)]
pub struct RankedJob {
    #[serde(flatten)]
    pub job: matching::ScoredRow,
    /// Priority for this owner, blending the three components below. Not a hiring probability.
    pub score: f32,
    pub technical: f32,
    pub profile_match: f32,
    pub career_stage: &'static str,
    pub career_priority: f32,
    /// "eligible", or "needs_review" when a gap below cannot be settled from the CV and posting.
    pub eligibility: &'static str,
    pub explanation: String,
    pub gaps: Vec<String>,
    pub auto_eligible: bool,
    pub source_key: String,
    pub provider: String,
}

/// Classified from the posting kind and title only: descriptions often mention supervising
/// interns or a past internship without being internships themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CareerStage {
    Thesis,
    Internship,
    WorkingStudent,
    ResearchAssistant,
    Phd,
    NewGrad,
    Junior,
    Unspecified,
    Mid,
    Senior,
    Staff,
}

impl CareerStage {
    fn label(self) -> &'static str {
        match self {
            Self::Thesis => "Master's thesis",
            Self::Internship => "Internship",
            Self::WorkingStudent => "Working student",
            Self::ResearchAssistant => "Research assistant",
            Self::Phd => "PhD position",
            Self::NewGrad => "New graduate",
            Self::Junior => "Junior",
            Self::Unspecified => "Level not stated",
            Self::Mid => "Mid-level",
            Self::Senior => "Senior",
            Self::Staff => "Staff / principal",
        }
    }
}

/// Initial, uncalibrated policy: (technical, saved profile, career stage) weights.
fn weights(profile_key: &str) -> (f32, f32, f32) {
    if profile_key == "research" {
        (0.45, 0.20, 0.35)
    } else {
        (0.55, 0.15, 0.30)
    }
}

/// 0-100 preference for a stage. Research has its own table so a funded PhD vacancy is not
/// outranked by an internship's stage bonus, while an unconfirmed PhD is never treated as funded.
fn stage_priority(stage: CareerStage, profile_key: &str, advertised_state: &str) -> f32 {
    use CareerStage::*;
    let research = profile_key == "research";
    match stage {
        Thesis => 100.0,
        Internship if research => 85.0,
        Internship => 100.0,
        WorkingStudent if research => 80.0,
        WorkingStudent | ResearchAssistant => 92.0,
        Phd if research && advertised_state == "advertised" => 95.0,
        Phd if research => 60.0,
        Phd => 65.0,
        NewGrad if research => 72.0,
        NewGrad => 78.0,
        Junior if research => 68.0,
        Junior => 74.0,
        Unspecified => 52.0,
        Mid => 36.0,
        Senior => 8.0,
        Staff => 2.0,
    }
}

/// Recover an explicit title category only for generic feed classifications.
fn effective_kind(job: &matching::ScoredRow) -> &str {
    use opportunity_domain::OpportunityKind as K;
    if matches!(job.kind.as_str(), "employment" | "other" | "part_time") {
        match opportunity_domain::classify_kind(&job.canonical_title, None) {
            K::MasterThesis => return "master_thesis",
            K::Internship => return "internship",
            K::WorkingStudent => return "working_student",
            K::ResearchAssistant => return "research_assistant",
            K::Phd => return "phd",
            _ => {}
        }
    }
    &job.kind
}

fn career_stage(job: &matching::ScoredRow) -> CareerStage {
    use CareerStage::*;
    match effective_kind(job) {
        "master_thesis" => return Thesis,
        "internship" => return Internship,
        "working_student" => return WorkingStudent,
        "research_assistant" => return ResearchAssistant,
        "phd" => return Phd,
        _ => {}
    }
    let title = opportunity_domain::normalize_location(Some(&job.canonical_title));
    let has = |words: &[&str]| {
        words
            .iter()
            .any(|w| opportunity_domain::normalize::contains_phrase(&title, w))
    };
    // German academic employment ("wissenschaftliche/r Mitarbeiter/in") is in practice a doctoral post.
    if title.contains("wissenschaftlich") && title.contains("mitarbeiter") {
        return Phd;
    }
    if has(&[
        "staff",
        "principal",
        "distinguished",
        "director",
        "head of",
        "vp",
    ]) {
        return Staff;
    }
    if has(&["senior", "sr", "lead", "manager", "iii", "l5", "l6"]) {
        return Senior;
    }
    if has(&[
        "mid",
        "mid level",
        "midlevel",
        "mid career",
        "intermediate",
        "ii",
        "l4",
    ]) {
        return Mid;
    }
    if has(&["student"]) {
        return WorkingStudent;
    }
    if has(&[
        "new grad",
        "new graduate",
        "new college graduate",
        "graduate",
        "early career",
        "early talent",
        "entry level",
        "entrylevel",
        "absolvent",
        "berufseinsteiger",
        "trainee",
    ]) {
        return NewGrad;
    }
    if has(&["junior", "jr", "associate engineer", "engineer i", "l3"]) {
        return Junior;
    }
    // A plain "Software Engineer" is not assumed junior; a stated requirement only downranks.
    match required_years(job.description_text.as_deref().unwrap_or("")) {
        Some(years) if years >= 8 => Senior,
        Some(years) if years >= 5 => Mid,
        _ => Unspecified,
    }
}

fn priority_score(technical: f32, profile: f32, stage: f32, profile_key: &str) -> f32 {
    let (t, p, s) = weights(profile_key);
    (t * technical.clamp(0.0, 100.0) + p * profile.clamp(0.0, 100.0) + s * stage).clamp(0.0, 100.0)
}

/// Neither stage priority nor a profile score grants permission to submit; the per-employer
/// authorization is still checked separately before any submission.
fn auto_application_eligible(
    cv_auto_eligible: bool,
    profile_fit: &opportunity_domain::MatchResult,
    advertised_state: &str,
) -> bool {
    cv_auto_eligible
        && profile_fit.eligible
        && profile_fit.score >= 20.0
        && advertised_state == "advertised"
}

fn direction(key: &str) -> Vec<&'static str> {
    match key {
        "firmware" => vec![
            "firmware",
            "embedded",
            "RTOS",
            "bootloader",
            "device driver",
            "platform systems",
        ],
        "memory" => vec![
            "memory",
            "DRAM",
            "CXL",
            "DDR",
            "kernel",
            "operating systems",
            "Linux",
            "storage",
        ],
        "architecture" => vec![
            "CPU",
            "RTL",
            "microarchitecture",
            "RISC-V",
            "Verilog",
            "verification",
            "ASIC",
            "architecture",
        ],
        "ai-systems" => vec![
            "AI",
            "GPU",
            "accelerator",
            "compiler",
            "CUDA",
            "machine learning",
            "co-design",
            "inference",
        ],
        // Research positions in the same four technical areas. Stage words such as "internship"
        // gave any internship technical credit; thesis/PhD kinds come from the profile's job kinds.
        _ => ["firmware", "memory", "architecture", "ai-systems"]
            .into_iter()
            .flat_map(direction)
            .collect(),
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
    let terms = direction(key);
    let title_matches = terms
        .iter()
        .filter(|w| contains(&normalized_title, w))
        .count();
    let body_matches = terms
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
    let key = profile.profile.key.as_str();
    let rules = profiles::rules_for(db, Some(profile_id)).await?;
    // Load only rows that contain a direction term, which fit() requires anyway, so the shortlist
    // cap can no longer drop an older relevant posting behind newer irrelevant ones.
    let terms = direction(key)
        .iter()
        .map(|t| {
            if t.contains(' ') {
                format!("\"{t}\"")
            } else {
                (*t).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" or ");
    let query = matching::MatchQuery {
        q: Some(terms),
        ..Default::default()
    };
    let rows = matching::shortlist(db, &query).await?;
    let sources: Vec<(Uuid,String,String)>=sqlx::query_as("select distinct on(p.opportunity_id) p.opportunity_id,s.key,s.kind from postings p join sources s on s.id=p.source_id where p.closed_at is null and p.opportunity_id is not null order by p.opportunity_id,p.url")
        .fetch_all(db).await?;
    let mut ranked = Vec::new();
    for row in rows {
        let (technical, evidence, mut gaps, cv_auto_eligible) = fit(&cv, key, &row);
        // No stage bonus can rescue an unrelated technical match.
        if technical < 20.0 {
            continue;
        }
        let applicable: Vec<_> = rules
            .iter()
            .filter(|(_, target)| target.is_none() || *target == Some(row.id))
            .map(|(rule, _)| rule.clone())
            .collect();
        let mut candidate = matching::to_candidate(&row);
        candidate.kind = effective_kind(&row).to_owned();
        let profile_fit =
            opportunity_domain::evaluate(&profile.profile, &candidate, &applicable, None);
        // Saved kinds, location preferences and blacklist rules are hard exclusions.
        if !profile_fit.eligible {
            continue;
        }
        let Some((_, source_key, provider)) = sources.iter().find(|s| s.0 == row.id) else {
            continue;
        };
        let stage = career_stage(&row);
        if stage == CareerStage::Phd && row.advertised_state != "advertised" {
            gaps.push("No funded vacancy confirmed".into());
        }
        let career_priority = stage_priority(stage, key, &row.advertised_state);
        let auto_eligible =
            auto_application_eligible(cv_auto_eligible, &profile_fit, &row.advertised_state);
        ranked.push(RankedJob {
            score: priority_score(technical, profile_fit.score, career_priority, key),
            technical,
            profile_match: profile_fit.score,
            career_stage: stage.label(),
            career_priority,
            eligibility: if gaps.is_empty() {
                "eligible"
            } else {
                "needs_review"
            },
            explanation: format!("{} · {evidence}", stage.label()),
            gaps,
            auto_eligible,
            job: row,
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
    let (paused, cap) = workflow::brakes(db).await?;
    let mail:Vec<Value>=sqlx::query_scalar("select jsonb_build_object('application_id',application_id,'state',delivery_state,'error',delivery_error) from notifications where kind='submitted' order by created_at desc limit 100").fetch_all(db).await?;
    Ok(
        json!({"cv":cv,"profiles":profiles,"selected_profile":profile_id,"jobs":jobs,"matches":matches,"discovered":discovered,"applications":applications,"questions":questions,"policies":policies,"last_updated":last_updated,"mail":mail,"email_configured":crate::mail::configured(),"cv_ready":current_candidate(db).await?.is_some(),"ranking_policy":RANKING_POLICY,"submissions_paused":paused,"max_automatic_per_day":cap}),
    )
}

/// The owner's emergency stop: refuses every submission and halts automatic preparation.
pub async fn pause_submissions(db: &Db, paused: bool) -> Result<(), AppError> {
    let owner = profiles::local_owner(db).await?;
    sqlx::query("insert into product_settings(owner_id,submissions_paused) values($1,$2) on conflict(owner_id) do update set submissions_paused=$2,updated_at=now()")
        .bind(owner).bind(paused).execute(db).await?;
    Ok(())
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

/// Hand an application to the owner: no automation will touch it again, so they are told once.
async fn manual(db: &Db, id: Uuid, reason: &str) -> Result<(), AppError> {
    applications::set_status(db, id, "unsupported", json!({})).await?;
    sqlx::query("update applications set route_reason=$2 where id=$1")
        .bind(id)
        .bind(reason)
        .execute(db)
        .await?;
    applications::notify(
        db,
        Some(id),
        "manual_required",
        &format!("manual:{id}"),
        json!({"reason": reason}),
    )
    .await?;
    Ok(())
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
        return manual(db, id, "Complete this application on the employer website.").await;
    }
    let prepared = match workflow::prepare(db, id, &url).await {
        Ok(p) => p,
        Err(error) => {
            if error.to_string().contains("source refresh") {
                return Ok(());
            }
            tracing::warn!(%error, "form unavailable");
            return manual(db, id, "This employer form requires manual completion.").await;
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
        return manual(
            db,
            id,
            "The employer requires manual completion or a verification challenge.",
        )
        .await;
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
    if workflow::brakes(db).await?.0 {
        lock.commit().await?;
        return Ok(());
    }
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

    use opportunity_domain::{ExclusionKind, ExclusionRule, Profile};

    fn row(title: &str, kind: &str, description: &str) -> matching::ScoredRow {
        matching::ScoredRow {
            id: Uuid::new_v4(),
            canonical_title: title.to_owned(),
            organization: "Example Silicon".into(),
            kind: kind.into(),
            advertised_state: "advertised".into(),
            country_code: Some("DE".into()),
            city: Some("Munich".into()),
            remote_raw: None,
            deadline_at: None,
            supervisor: None,
            first_seen_at: Utc::now(),
            last_seen_at: Utc::now(),
            title_only: description.is_empty(),
            apply_url: None,
            description_text: Some(description.to_owned()),
            department: None,
            source_names: vec!["Example Silicon".into()],
        }
    }

    fn assess(profile: &Profile, job: &matching::ScoredRow, rules: &[ExclusionRule]) -> bool {
        let mut candidate = matching::to_candidate(job);
        candidate.kind = effective_kind(job).to_owned();
        opportunity_domain::evaluate(profile, &candidate, rules, None).eligible
    }

    #[test]
    fn titles_classify_into_career_stages() {
        use CareerStage::*;
        for (title, kind, expected) in [
            ("Masterarbeit: RISC-V", "master_thesis", Thesis),
            ("Masterarbeit: DRAM", "employment", Thesis),
            ("Firmware Internship", "employment", Internship),
            ("Firmware Werkstudent (m/w/d)", "employment", WorkingStudent),
            (
                "Student Researcher, Memory Systems",
                "employment",
                WorkingStudent,
            ),
            ("PhD Candidate in Memory Systems", "employment", Phd),
            (
                "Doctoral Researcher, Computer Architecture",
                "employment",
                Phd,
            ),
            (
                "Wissenschaftliche/r Mitarbeiter/in Rechnerarchitektur",
                "employment",
                Phd,
            ),
            (
                "New College Graduate, GPU Architecture",
                "employment",
                NewGrad,
            ),
            ("Early Talent Firmware Engineer", "employment", NewGrad),
            ("Junior Firmware Engineer", "employment", Junior),
            ("Firmware Engineer I", "employment", Junior),
            ("Associate Engineer, RTL", "employment", Junior),
            ("Firmware Engineer", "employment", Unspecified),
            ("Research Engineer, Accelerators", "employment", Unspecified),
            ("Firmware Engineer II", "employment", Mid),
            ("Mid-Level Firmware Engineer", "employment", Mid),
            ("Senior Firmware Engineer", "employment", Senior),
            ("Sr. RTL Design Engineer", "employment", Senior),
            ("Principal CPU Architect", "employment", Staff),
            ("Staff Engineer, Memory", "employment", Staff),
        ] {
            assert_eq!(career_stage(&row(title, kind, "")), expected, "{title}");
        }
    }

    #[test]
    fn body_text_never_changes_the_stage_except_to_downrank() {
        use CareerStage::*;
        let stage = |title, body| career_stage(&row(title, "employment", body));
        assert_eq!(
            stage("Software Engineer", "You will mentor interns."),
            Unspecified
        );
        assert_eq!(stage("International Firmware Engineer", ""), Unspecified);
        assert_eq!(
            stage("Firmware Engineer", "Requires 6+ years of experience"),
            Mid
        );
        assert_eq!(
            stage("Firmware Engineer", "Requires 9+ years of experience"),
            Senior
        );
    }

    #[test]
    fn equal_relevance_orders_thesis_and_internship_then_student_junior_mid_senior() {
        use CareerStage::*;
        let score = |stage| {
            priority_score(
                75.0,
                60.0,
                stage_priority(stage, "memory", "advertised"),
                "memory",
            )
        };
        let order = [
            Thesis,
            WorkingStudent,
            NewGrad,
            Junior,
            Unspecified,
            Mid,
            Senior,
            Staff,
        ];
        assert_eq!(score(Thesis), score(Internship));
        for pair in order.windows(2) {
            assert!(
                score(pair[0]) > score(pair[1]),
                "{:?} > {:?}",
                pair[0],
                pair[1]
            );
        }
        // Strong technical fit beats a weak internship; the score stays bounded.
        assert!(
            priority_score(
                95.0,
                75.0,
                stage_priority(Junior, "firmware", "advertised"),
                "firmware"
            ) > priority_score(
                35.0,
                40.0,
                stage_priority(Internship, "firmware", "advertised"),
                "firmware"
            )
        );
        assert!(priority_score(150.0, 150.0, 100.0, "firmware") <= 100.0);
    }

    #[test]
    fn research_keeps_a_funded_phd_ahead_of_an_internship_but_not_an_unfunded_one() {
        use CareerStage::*;
        let research = |stage, state| {
            priority_score(
                70.0,
                60.0,
                stage_priority(stage, "research", state),
                "research",
            )
        };
        assert!(research(Phd, "advertised") > research(Internship, "advertised"));
        assert!(research(Phd, "open_topic") < research(Internship, "advertised"));
        assert!(research(Thesis, "open_topic") >= research(Phd, "advertised"));
        // Outside research the engineering table applies.
        assert!(
            stage_priority(Internship, "firmware", "advertised")
                > stage_priority(Phd, "firmware", "advertised")
        );
    }

    #[test]
    fn an_unrelated_internship_gets_no_technical_credit_in_any_profile() {
        let cv = CandidateProfile {
            skills: vec!["Linux".into(), "DRAM".into()],
            ..Default::default()
        };
        let marketing = row(
            "Marketing Internship",
            "internship",
            "Social media and brand strategy",
        );
        for (key, _) in TRACKS {
            assert_eq!(fit(&cv, key, &marketing).0, 0.0, "{key}");
        }
        let thesis = row(
            "Masterarbeit: DRAM reliability",
            "master_thesis",
            "DRAM memory Linux",
        );
        assert!(fit(&cv, "research", &thesis).0 >= 20.0);
    }

    #[test]
    fn saved_location_kind_and_blacklist_exclude_before_ranking() {
        let mut profile = Profile::blank("memory", "Memory systems");
        profile.terms = vec!["DRAM".into()];
        profile.preferred_countries = vec!["DE".into()];
        profile.job_kinds = vec!["internship".into(), "master_thesis".into()];
        let mut job = row("DRAM Internship", "internship", "DRAM memory systems");
        assert!(assess(&profile, &job, &[]));
        job.country_code = Some("FR".into());
        assert!(!assess(&profile, &job, &[]));
        job.country_code = Some("DE".into());
        job.kind = "employment".into();
        job.canonical_title = "DRAM Engineer".into();
        assert!(!assess(&profile, &job, &[]));
        job.kind = "internship".into();
        let rule = ExclusionRule {
            kind: ExclusionKind::Company,
            value: "Example Silicon".into(),
            note: None,
        };
        assert!(!assess(&profile, &job, &[rule]));
    }

    #[test]
    fn stage_priority_never_authorizes_an_automatic_application() {
        let mut profile = Profile::blank("memory", "Memory systems");
        profile.terms = vec!["DRAM".into()];
        let job = row("DRAM Internship", "internship", "DRAM memory systems");
        let mut candidate = matching::to_candidate(&job);
        candidate.kind = effective_kind(&job).to_owned();
        let fit = opportunity_domain::evaluate(&profile, &candidate, &[], None);
        assert!(fit.eligible);
        assert!(!auto_application_eligible(false, &fit, "advertised"));
        assert!(!auto_application_eligible(true, &fit, "open_topic"));
        assert!(!auto_application_eligible(
            true,
            &fit,
            "potential_supervisor"
        ));
    }
}

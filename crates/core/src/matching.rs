//! Scoring the stored corpus against a profile.
//!
//! The work is staged so the cheap, always-available part happens first. A bounded lexical shortlist is pulled from PostgreSQL, every candidate in it is scored deterministically, and only then is the top of that list reranked by an embedding model, and only when one actually answers. Nothing is recomputed on ingest and no match is stored, because the calculation is deterministic and cheap enough to run per request, and a stored copy would be stale between scans.

use chrono::Utc;
use opportunity_domain::matching::{Candidate, ExclusionRule, MatchResult};
use opportunity_domain::{content_hash, evaluate, Profile};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Db;
use crate::embeddings::{self, EmbeddingProvider, SemanticStatus};
use crate::profiles::{self, ProfileError};

/// How many opportunities are pulled from the database before scoring. Bounded because the whole shortlist is held in memory with its descriptions, which on a 1 GiB host is the largest single allocation this process makes. When the corpus outgrows it the ranking says so rather than quietly scoring a subset.
const SHORTLIST: i64 = 10000;

fn shortlist_limit() -> i64 {
    std::env::var("MATCH_SHORTLIST")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(SHORTLIST)
}
/// How many of the highest lexical scorers are sent to the embedding model. This is the bound that keeps the semantic stage from costing a model call per listing per refresh.
const RERANK_TOP_N: usize = 50;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ScoredRow {
    pub id: Uuid,
    pub canonical_title: String,
    pub organization: String,
    pub kind: String,
    pub advertised_state: String,
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub remote_raw: Option<String>,
    pub deadline_at: Option<chrono::DateTime<chrono::Utc>>,
    pub supervisor: Option<String>,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub title_only: bool,
    pub apply_url: Option<String>,
    pub description_text: Option<String>,
    pub department: Option<String>,
    pub source_names: Vec<String>,
}

/// One opportunity with its verdict attached.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredOpportunity {
    #[serde(flatten)]
    pub opportunity: ScoredRow,
    #[serde(rename = "match")]
    pub result: MatchResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchPage {
    pub profile_key: String,
    pub profile_id: Uuid,
    /// Says in one line exactly how these were ranked, including whether a model was involved.
    pub ranking: String,
    pub semantic: SemanticStatus,
    pub total_considered: usize,
    pub eligible: usize,
    pub excluded: usize,
    pub items: Vec<ScoredOpportunity>,
}

#[derive(Debug, Clone, Default)]
pub struct MatchQuery {
    pub q: Option<String>,
    pub limit: usize,
    /// Include opportunities a rule excluded, so the owner can see and undo the exclusion.
    pub include_excluded: bool,
}

/// Load the corpus a profile should be scored against. Deliberately not filtered by location or kind: those are the matcher's decisions and must come with a stated reason, not a silent omission.
pub(crate) async fn shortlist(
    pool: &Db,
    query: &MatchQuery,
) -> Result<Vec<ScoredRow>, sqlx::Error> {
    sqlx::query_as::<_, ScoredRow>(
        "select o.id, o.canonical_title, org.display_name as organization, o.kind, o.advertised_state,
                o.country_code, o.city, o.remote_raw, o.deadline_at, o.supervisor,
                o.first_seen_at, o.last_seen_at,
                coalesce(bool_and(p.title_only), true) as title_only,
                min(p.url) as apply_url,
                max(p.description_text) as description_text,
                max(p.department) as department,
                coalesce(array_agg(distinct s.display_name) filter (where s.display_name is not null), '{}') as source_names
         from opportunities o
         join organizations org on org.id = o.organization_id
         left join postings p on p.opportunity_id = o.id and p.closed_at is null
         left join sources s on s.id = p.source_id
         where o.closed_at is null and (o.deadline_at is null or o.deadline_at > now())
           and ($1::text is null or o.search_vector @@ websearch_to_tsquery('simple', $1))
         group by o.id, org.display_name
         -- With a query, the cap keeps the strongest candidates rather than the newest: a query term in
         -- the title first (which ranking scores highest), then full-text rank, then recency.
         order by ($1::text is not null and to_tsvector('simple', o.canonical_title) @@ websearch_to_tsquery('simple', $1)) desc,
                  case when $1::text is null then 0 else ts_rank(o.search_vector, websearch_to_tsquery('simple', $1)) end desc,
                  o.last_seen_at desc, o.id
         limit $2",
    )
    .bind(query.q.as_deref().filter(|q| !q.trim().is_empty()))
    .bind(shortlist_limit())
    .fetch_all(pool)
    .await
}

pub(crate) fn to_candidate(row: &ScoredRow) -> Candidate {
    Candidate {
        title: row.canonical_title.clone(),
        organization: row.organization.clone(),
        description: row.description_text.clone(),
        department: row.department.clone(),
        kind: row.kind.clone(),
        advertised_state: row.advertised_state.clone(),
        country_code: row.country_code.clone(),
        city: row.city.clone(),
        remote_raw: row.remote_raw.clone(),
        age_days: Some((Utc::now() - row.first_seen_at).num_days()),
        title_only: row.title_only,
    }
}

/// The text an embedding model sees for one opportunity. Kept short so a rerank stays cheap, and hashed so a cached vector is reused until the content itself changes.
fn embedding_text(row: &ScoredRow) -> String {
    let description = row.description_text.as_deref().unwrap_or("");
    let truncated: String = description.chars().take(1200).collect();
    format!(
        "{} at {}. {}",
        row.canonical_title, row.organization, truncated
    )
}

/// The text that describes a profile to an embedding model.
fn profile_text(profile: &Profile) -> String {
    format!(
        "{}. Interested in {}. Technologies: {}.",
        profile.display_name,
        profile.terms.join(", "),
        profile.preferred_technologies.join(", ")
    )
}

/// Score every opportunity in the shortlist against one profile and return the best.
pub async fn matches_for_profile(
    pool: &Db,
    profile_id: Uuid,
    query: &MatchQuery,
    provider: Option<&EmbeddingProvider>,
) -> Result<MatchPage, ProfileError> {
    let stored = profiles::get(pool, profile_id).await?;
    let rules_with_targets = profiles::rules_for(pool, Some(profile_id)).await?;
    let rows = shortlist(pool, query).await?;
    // A full shortlist means the corpus may be larger than what was scored. Nothing here may pretend otherwise.
    let truncated_at = (rows.len() as i64 == shortlist_limit()).then_some(rows.len());

    // Hidden rules name one opportunity each, so they are applied per row rather than to every candidate.
    let global_rules: Vec<ExclusionRule> = rules_with_targets
        .iter()
        .filter(|(_, target)| target.is_none())
        .map(|(rule, _)| rule.clone())
        .collect();

    let mut scored: Vec<(ScoredRow, MatchResult)> = Vec::with_capacity(rows.len());
    for row in rows {
        let mut rules = global_rules.clone();
        if let Some((hidden, _)) = rules_with_targets
            .iter()
            .find(|(_, target)| target.as_ref() == Some(&row.id))
        {
            rules.push(hidden.clone());
        }
        let result = evaluate(&stored.profile, &to_candidate(&row), &rules, None);
        scored.push((row, result));
    }

    let total_considered = scored.len();
    let excluded = scored.iter().filter(|(_, r)| !r.eligible).count();
    let eligible = total_considered - excluded;

    scored.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // The semantic stage reruns scoring for the top of the lexical list only, so a model call never happens per listing per refresh.
    let semantic = match provider {
        None => SemanticStatus::Disabled,
        Some(provider) => {
            let status = provider.status().await;
            if status.is_available() {
                rerank(pool, provider, &stored.profile, &mut scored, &global_rules).await;
                sort_ranked(&mut scored);
            }
            status
        }
    };

    let limit = query.limit.clamp(1, 200);
    // Asking to see what the rules excluded means seeing those, not hunting for them beneath everything that was kept: an excluded result scores zero and would otherwise sink below every eligible one. They are ordered by the score they would have had, so the most relevant exclusion comes first.
    if query.include_excluded {
        scored.retain(|(_, r)| !r.eligible);
        scored.sort_by(|a, b| {
            would_be_score(&b.1)
                .partial_cmp(&would_be_score(&a.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        scored.retain(|(_, r)| r.eligible);
    }
    let items: Vec<ScoredOpportunity> = scored
        .into_iter()
        .take(limit)
        .map(|(opportunity, result)| ScoredOpportunity {
            opportunity,
            result,
        })
        .collect();

    let ranking = match truncated_at {
        None => semantic.label(),
        Some(scored_count) => format!(
            "{} Only the {} most recently seen opportunities were scored: the catalogue is larger than the shortlist. Raise MATCH_SHORTLIST to score all of them.",
            semantic.label(),
            scored_count
        ),
    };

    Ok(MatchPage {
        profile_key: stored.profile.key.clone(),
        profile_id,
        ranking,
        semantic,
        total_considered,
        eligible,
        excluded,
        items,
    })
}

/// Rank by score, then break ties deterministically by recency and finally by id. Without the tie-break, two equally scored opportunities would swap places between requests and a saved position would mean nothing.
fn sort_ranked(scored: &mut [(ScoredRow, MatchResult)]) {
    scored.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.0.first_seen_at.cmp(&a.0.first_seen_at))
            .then(a.0.id.cmp(&b.0.id))
    });
}

/// What an excluded result would have scored had the rule not applied. Used only to order the exclusion list usefully; it is never shown as the result's score.
fn would_be_score(result: &MatchResult) -> f32 {
    result.factors.iter().map(|f| f.contribution).sum()
}

async fn rerank(
    pool: &Db,
    provider: &EmbeddingProvider,
    profile: &Profile,
    scored: &mut [(ScoredRow, MatchResult)],
    rules: &[ExclusionRule],
) {
    let Ok(profile_vector) = provider.embed(&profile_text(profile)).await else {
        return;
    };
    for (row, result) in scored.iter_mut().take(RERANK_TOP_N) {
        if !result.eligible {
            continue;
        }
        let text = embedding_text(row);
        let hash = content_hash(&text, None, None, "");
        let vector = match embeddings::cached(pool, row.id, &provider.model, &hash).await {
            Some(cached) => cached,
            None => match provider.embed(&text).await {
                Ok(fresh) => {
                    let _ = embeddings::store(pool, row.id, &provider.model, &hash, &fresh).await;
                    fresh
                }
                // One failed call drops the semantic stage for that row rather than failing the search.
                Err(_) => continue,
            },
        };
        if let Some(similarity) = embeddings::cosine(&profile_vector, &vector) {
            *result = evaluate(profile, &to_candidate(row), rules, Some(similarity));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(title: &str, description: Option<&str>) -> ScoredRow {
        ScoredRow {
            id: Uuid::nil(),
            canonical_title: title.into(),
            organization: "Example Silicon".into(),
            kind: "employment".into(),
            advertised_state: "advertised".into(),
            country_code: Some("DE".into()),
            city: Some("Munich".into()),
            remote_raw: None,
            deadline_at: None,
            supervisor: None,
            first_seen_at: Utc::now(),
            last_seen_at: Utc::now(),
            title_only: description.is_none(),
            apply_url: Some("https://example.org/1".into()),
            description_text: description.map(str::to_owned),
            department: None,
            source_names: vec!["Example Silicon".into()],
        }
    }

    #[test]
    fn the_embedding_text_is_bounded_so_a_long_posting_cannot_make_a_rerank_expensive() {
        let long = "x".repeat(50_000);
        let text = embedding_text(&row("Memory Engineer", Some(&long)));
        assert!(
            text.chars().count() < 1400,
            "got {} characters",
            text.chars().count()
        );
        assert!(text.starts_with("Memory Engineer at Example Silicon."));
    }

    #[test]
    fn the_embedding_text_changes_only_when_the_content_does_so_a_cached_vector_stays_valid() {
        let a = embedding_text(&row("Memory Engineer", Some("DRAM work")));
        let b = embedding_text(&row("Memory Engineer", Some("DRAM work")));
        let c = embedding_text(&row("Memory Engineer", Some("CXL work")));
        assert_eq!(
            content_hash(&a, None, None, ""),
            content_hash(&b, None, None, "")
        );
        assert_ne!(
            content_hash(&a, None, None, ""),
            content_hash(&c, None, None, "")
        );
    }

    #[test]
    fn a_profile_is_described_to_a_model_by_its_interests_and_never_by_personal_data() {
        let mut profile = Profile::blank("architecture", "Computer architecture");
        profile.terms = vec!["DRAM".into()];
        profile.current_country = Some("DE".into());
        profile.work_authorization = vec!["DE".into()];
        profile.education_level = Some("MSc".into());
        let text = profile_text(&profile);
        assert!(text.contains("DRAM"));
        // Location, authorization and education are private and never leave the machine in a model prompt.
        assert!(!text.contains("DE"));
        assert!(!text.contains("MSc"));
    }
}

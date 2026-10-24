//! Daily digest and export.
//!
//! A digest is a snapshot of what a profile ranked highest, with the employer's own link and the dates the source published. Deadlines that are close are called out separately, because a deadline is the one thing in this tool that expires. Nothing is summarised by a model: the text is assembled from stored fields, so every line can be traced to a record.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Db;
use crate::embeddings::EmbeddingProvider;
use crate::matching::{self, MatchQuery, ScoredOpportunity};
use crate::profiles::{self, ProfileError};

/// Deadlines within this many days are listed in their own section.
const DEADLINE_HORIZON_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestFormat {
    Markdown,
    Csv,
}

impl DigestFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Csv => "csv",
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StoredDigest {
    pub id: Uuid,
    pub profile_id: Option<Uuid>,
    pub generated_at: DateTime<Utc>,
    pub format: String,
    pub item_count: i32,
    pub deadline_count: i32,
    pub body: String,
}

/// Build a digest for one profile and store it so the owner can look back at what was surfaced.
pub async fn generate(
    pool: &Db,
    profile_id: Uuid,
    format: DigestFormat,
    limit: usize,
    provider: Option<&EmbeddingProvider>,
) -> Result<StoredDigest, ProfileError> {
    let stored = profiles::get(pool, profile_id).await?;
    let page = matching::matches_for_profile(
        pool,
        profile_id,
        &MatchQuery {
            q: None,
            limit,
            include_excluded: false,
        },
        provider,
    )
    .await?;

    let horizon = Utc::now() + Duration::days(DEADLINE_HORIZON_DAYS);
    let due_soon: Vec<&ScoredOpportunity> = page
        .items
        .iter()
        .filter(|item| {
            item.opportunity
                .deadline_at
                .is_some_and(|d| d <= horizon && d >= Utc::now())
        })
        .collect();

    let body = match format {
        DigestFormat::Markdown => render_markdown(
            &stored.profile.display_name,
            &page.ranking,
            &page.items,
            &due_soon,
        ),
        DigestFormat::Csv => render_csv(&page.items),
    };

    let row = sqlx::query_as::<_, StoredDigest>(
        "insert into digests (owner_id, profile_id, format, item_count, deadline_count, body)
         values ($1,$2,$3,$4,$5,$6)
         returning id, profile_id, generated_at, format, item_count, deadline_count, body",
    )
    .bind(profiles::local_owner(pool).await?)
    .bind(profile_id)
    .bind(format.as_str())
    .bind(page.items.len() as i32)
    .bind(due_soon.len() as i32)
    .bind(&body)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &Db, limit: i64) -> Result<Vec<StoredDigest>, ProfileError> {
    let rows = sqlx::query_as::<_, StoredDigest>(
        "select id, profile_id, generated_at, format, item_count, deadline_count, body
         from digests order by generated_at desc limit $1",
    )
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn date(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn location(item: &ScoredOpportunity) -> String {
    let parts: Vec<String> = [
        item.opportunity.city.clone(),
        item.opportunity.country_code.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        "location unknown".into()
    } else {
        parts.join(", ")
    }
}

pub fn render_markdown(
    profile_name: &str,
    ranking: &str,
    items: &[ScoredOpportunity],
    due_soon: &[&ScoredOpportunity],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Digest for {profile_name}\n\n"));
    out.push_str(&format!(
        "Generated {}. {}\n\n",
        Utc::now().format("%Y-%m-%d %H:%M UTC"),
        ranking
    ));

    if items.is_empty() {
        out.push_str("No opportunity matched this profile. That is not the same as there being no openings: check the Coverage screen to see which sources were actually read.\n");
        return out;
    }

    if !due_soon.is_empty() {
        out.push_str(&format!(
            "## Deadlines within {DEADLINE_HORIZON_DAYS} days\n\n"
        ));
        for item in due_soon {
            out.push_str(&format!(
                "- **{}** at {}, closes {} ({})\n",
                item.opportunity.canonical_title,
                item.opportunity.organization,
                date(item.opportunity.deadline_at),
                item.opportunity
                    .apply_url
                    .as_deref()
                    .unwrap_or("no application link published")
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!("## Top {} matches\n\n", items.len()));
    for item in items {
        out.push_str(&format!(
            "### {} at {}\n\n",
            item.opportunity.canonical_title, item.opportunity.organization
        ));
        out.push_str(&format!(
            "- Score {:.0} using {}\n- {} | {} | first seen {} | last verified {}\n",
            item.result.score,
            item.result.model,
            location(item),
            item.opportunity.kind.replace('_', " "),
            date(Some(item.opportunity.first_seen_at)),
            date(Some(item.opportunity.last_seen_at)),
        ));
        match item.opportunity.apply_url.as_deref() {
            Some(url) => out.push_str(&format!("- Employer posting: {url}\n")),
            None => out.push_str("- The source published no application link\n"),
        }
        if item.opportunity.advertised_state != "advertised" {
            out.push_str(&format!(
                "- Recorded as {}, so it may not be a funded vacancy\n",
                item.opportunity.advertised_state.replace('_', " ")
            ));
        }
        if let Some(supervisor) = &item.opportunity.supervisor {
            out.push_str(&format!("- Supervisor: {supervisor}\n"));
        }
        for factor in item
            .result
            .factors
            .iter()
            .filter(|f| f.contribution > 0.0)
            .take(3)
        {
            out.push_str(&format!("- Why: {}\n", factor.detail));
        }
        for uncertainty in &item.result.uncertainties {
            out.push_str(&format!("- Unknown: {}\n", uncertainty.detail));
        }
        out.push('\n');
    }
    out
}

/// Comma-separated export. Values are quoted and embedded quotes doubled, so a title containing a comma cannot shift every later column.
pub fn render_csv(items: &[ScoredOpportunity]) -> String {
    let mut out = String::from(
        "score,model,title,organization,kind,advertised_state,city,country,deadline,first_seen,last_verified,url\n",
    );
    for item in items {
        let fields = [
            format!("{:.0}", item.result.score),
            item.result.model.clone(),
            item.opportunity.canonical_title.clone(),
            item.opportunity.organization.clone(),
            item.opportunity.kind.clone(),
            item.opportunity.advertised_state.clone(),
            item.opportunity.city.clone().unwrap_or_default(),
            item.opportunity.country_code.clone().unwrap_or_default(),
            date(item.opportunity.deadline_at),
            date(Some(item.opportunity.first_seen_at)),
            date(Some(item.opportunity.last_seen_at)),
            item.opportunity.apply_url.clone().unwrap_or_default(),
        ];
        let row: Vec<String> = fields
            .iter()
            .map(|f| format!("\"{}\"", f.replace('"', "\"\"")))
            .collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewFeedback {
    pub opportunity_id: Uuid,
    pub profile_id: Option<Uuid>,
    pub verdict: String,
    #[serde(default)]
    pub sampled_from: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StoredFeedback {
    pub id: Uuid,
    pub profile_id: Option<Uuid>,
    pub opportunity_id: Uuid,
    pub verdict: String,
    pub sampled_from: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn record_feedback(
    pool: &Db,
    feedback: NewFeedback,
) -> Result<StoredFeedback, ProfileError> {
    if !matches!(feedback.verdict.as_str(), "interested" | "not_interested") {
        return Err(ProfileError::Invalid(format!(
            "unknown verdict '{}'",
            feedback.verdict
        )));
    }
    let sampled = feedback.sampled_from.as_deref().unwrap_or("surfaced");
    if !matches!(sampled, "surfaced" | "not_surfaced") {
        return Err(ProfileError::Invalid(format!(
            "unknown sample origin '{sampled}'"
        )));
    }
    let row = sqlx::query_as::<_, StoredFeedback>(
        "insert into opportunity_feedback (owner_id, profile_id, opportunity_id, verdict, sampled_from, note)
         values ($1,$2,$3,$4,$5,$6)
         on conflict (owner_id, profile_id, opportunity_id)
         do update set verdict = excluded.verdict, sampled_from = excluded.sampled_from, note = excluded.note, created_at = now()
         returning id, profile_id, opportunity_id, verdict, sampled_from, note, created_at",
    )
    .bind(profiles::local_owner(pool).await?)
    .bind(feedback.profile_id)
    .bind(feedback.opportunity_id)
    .bind(&feedback.verdict)
    .bind(sampled)
    .bind(feedback.note.as_deref())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Opportunities the ranking did not surface, drawn at random for honest recall checking. Judging only what was already shown would measure the ranking against itself.
pub async fn sample_not_surfaced(
    pool: &Db,
    surfaced: &[Uuid],
    limit: i64,
) -> Result<Vec<(Uuid, String, String)>, ProfileError> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "select o.id, o.canonical_title, org.display_name
         from opportunities o
         join organizations org on org.id = o.organization_id
         where o.closed_at is null and not (o.id = any($1))
         order by random() limit $2",
    )
    .bind(surfaced)
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::ScoredRow;
    use opportunity_domain::matching::{Factor, MatchResult, Uncertainty};

    fn item(title: &str, deadline: Option<DateTime<Utc>>, url: Option<&str>) -> ScoredOpportunity {
        ScoredOpportunity {
            opportunity: ScoredRow {
                id: Uuid::nil(),
                canonical_title: title.into(),
                organization: "Example Silicon".into(),
                kind: "master_thesis".into(),
                advertised_state: "open_topic".into(),
                country_code: Some("DE".into()),
                city: Some("Karlsruhe".into()),
                remote_raw: None,
                deadline_at: deadline,
                supervisor: Some("Prof. Dr. Anna Beispiel".into()),
                first_seen_at: Utc::now(),
                last_seen_at: Utc::now(),
                title_only: false,
                apply_url: url.map(str::to_owned),
                description_text: Some("Work on page migration.".into()),
                department: None,
                source_names: vec!["Example Institut".into()],
            },
            result: MatchResult {
                score: 72.0,
                eligible: true,
                exclusions: Vec::new(),
                factors: vec![Factor {
                    name: "related wording".into(),
                    contribution: 2.0,
                    detail: "page migration matched your term DRAM".into(),
                    evidence: vec!["page migration".into()],
                }],
                uncertainties: vec![Uncertainty {
                    field: "offer".into(),
                    detail: "this is recorded as open topic, so it may not be a funded vacancy"
                        .into(),
                }],
                matched_concepts: vec!["memory systems".into()],
                model: "lexical-concepts-v1".into(),
            },
        }
    }

    #[test]
    fn a_markdown_digest_carries_the_real_link_the_dates_and_the_reason() {
        let soon = Utc::now() + Duration::days(10);
        let items = vec![item(
            "Masterarbeit: Seitenmigration",
            Some(soon),
            Some("https://example.org/arbeit"),
        )];
        let due: Vec<&ScoredOpportunity> = items.iter().collect();
        let out = render_markdown(
            "Research",
            "lexical ranking only: no embedding provider is configured",
            &items,
            &due,
        );

        assert!(
            out.contains("https://example.org/arbeit"),
            "the employer link must be present verbatim"
        );
        assert!(out.contains("Deadlines within 30 days"));
        assert!(
            out.contains("page migration matched your term DRAM"),
            "the reason must be shown"
        );
        assert!(
            out.contains("may not be a funded vacancy"),
            "an open topic must stay flagged"
        );
        assert!(out.contains("Prof. Dr. Anna Beispiel"));
        assert!(
            out.contains("lexical ranking only"),
            "the ranking method must be stated"
        );
        assert!(!out.to_lowercase().contains("ai-powered"));
    }

    #[test]
    fn a_digest_with_no_matches_says_so_without_implying_there_are_no_jobs() {
        let out = render_markdown("Research", "lexical ranking only", &[], &[]);
        assert!(out.contains("not the same as there being no openings"));
    }

    #[test]
    fn an_opportunity_without_a_published_link_says_so_rather_than_inventing_one() {
        let items = vec![item("Masterarbeit", None, None)];
        let out = render_markdown("Research", "lexical", &items, &[]);
        assert!(out.contains("published no application link"));
        assert!(!out.contains("http"));
    }

    #[test]
    fn csv_quoting_survives_a_title_containing_a_comma_and_a_quote() {
        let items = vec![item(
            "Thesis: \"memory, tiering\"",
            None,
            Some("https://example.org/x"),
        )];
        let csv = render_csv(&items);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[1].contains("\"Thesis: \"\"memory, tiering\"\"\""),
            "got {}",
            lines[1]
        );
        // The header and the row must have the same number of fields once quoting is respected.
        assert_eq!(lines[0].split(',').count(), 12);
    }

    #[test]
    fn csv_leaves_an_absent_value_empty_rather_than_writing_a_placeholder() {
        let mut only = item("Thesis", None, None);
        only.opportunity.city = None;
        only.opportunity.country_code = None;
        let csv = render_csv(&[only]);
        assert!(
            csv.contains("\"\",\"\",\"unknown\""),
            "missing city and country stay empty, deadline reads unknown: {csv}"
        );
    }
}

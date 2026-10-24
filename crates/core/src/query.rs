use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Db;

#[derive(Debug, Clone, Deserialize)]
pub struct OpportunityQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub source: Option<Uuid>,
    /// Filter by what the source is actually offering, so a search for real vacancies never returns thesis topics and vice versa.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub include_closed: bool,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
    #[serde(default)]
    pub sort: Option<String>,
}

impl OpportunityQuery {
    fn offset_limit(&self) -> (i64, i64) {
        let per_page = self.per_page.unwrap_or(25).clamp(1, 100);
        let page = self.page.unwrap_or(1).max(1);
        ((page - 1) * per_page, per_page)
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct OpportunityRow {
    pub id: Uuid,
    pub canonical_title: String,
    pub organization: String,
    pub kind: String,
    pub advertised_state: String,
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub remote_raw: Option<String>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub supervisor: Option<String>,
    pub contact: Option<String>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    /// True when no source gave a usable description for any posting behind this opportunity.
    pub title_only: bool,
    pub posting_count: i64,
    pub apply_url: Option<String>,
    pub source_names: Vec<String>,
    pub posted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpportunityPage {
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub items: Vec<OpportunityRow>,
}

/// Discover listing. All filters are bound parameters and the sort is chosen from a fixed set, so no caller input reaches the SQL text.
pub async fn list_opportunities(
    pool: &Db,
    query: &OpportunityQuery,
) -> Result<OpportunityPage, sqlx::Error> {
    let (offset, limit) = query.offset_limit();
    let order = match query.sort.as_deref() {
        Some("oldest") => "o.first_seen_at asc",
        Some("deadline") => "o.deadline_at asc nulls last",
        Some("title") => "o.canonical_title asc",
        Some("relevance") if query.q.is_some() => "rank desc, o.last_seen_at desc",
        _ => "o.last_seen_at desc",
    };
    let sql = format!(
        "with filtered as (
            select o.*, org.display_name as organization,
                   case when $1::text is null then 0::float4
                        else ts_rank(o.search_vector, websearch_to_tsquery('simple', $1)) end as rank
            from opportunities o
            join organizations org on org.id = o.organization_id
            where ($1::text is null or o.search_vector @@ websearch_to_tsquery('simple', $1))
              and ($2::text is null or o.country_code = $2)
              and ($3::text is null or o.city ilike $3)
              and ($4::text is null or o.kind = $4)
              and ($5::uuid is null or exists (select 1 from postings p where p.opportunity_id = o.id and p.source_id = $5))
              and ($9::text is null or o.advertised_state = $9)
              and ($6::bool or o.closed_at is null)
        )
        select o.id, o.canonical_title, o.organization, o.kind, o.advertised_state, o.country_code, o.city,
               o.remote_raw, o.deadline_at, o.supervisor, o.contact, o.first_seen_at, o.last_seen_at, o.closed_at,
               coalesce(bool_and(p.title_only), true) as title_only,
               count(p.id) as posting_count,
               min(p.url) as apply_url,
               coalesce(array_agg(distinct s.display_name) filter (where s.display_name is not null), '{{}}') as source_names,
               max(p.posted_at) as posted_at
        from filtered o
        left join postings p on p.opportunity_id = o.id and (($6::bool) or p.closed_at is null)
        left join sources s on s.id = p.source_id
        group by o.id, o.canonical_title, o.organization, o.kind, o.advertised_state, o.country_code, o.city,
                 o.remote_raw, o.deadline_at, o.supervisor, o.contact, o.first_seen_at, o.last_seen_at, o.closed_at, o.rank
        order by {order}
        limit $7 offset $8"
    );
    let items = sqlx::query_as::<_, OpportunityRow>(&sql)
        .bind(query.q.as_deref().filter(|q| !q.trim().is_empty()))
        .bind(query.country.as_deref().filter(|c| !c.trim().is_empty()))
        .bind(query.city.as_deref().filter(|c| !c.trim().is_empty()))
        .bind(query.kind.as_deref().filter(|k| !k.trim().is_empty()))
        .bind(query.source)
        .bind(query.include_closed)
        .bind(limit)
        .bind(offset)
        .bind(query.state.as_deref().filter(|s| !s.trim().is_empty()))
        .fetch_all(pool)
        .await?;

    let total: i64 = sqlx::query_scalar(
        "select count(*) from opportunities o
         where ($1::text is null or o.search_vector @@ websearch_to_tsquery('simple', $1))
           and ($2::text is null or o.country_code = $2)
           and ($3::text is null or o.city ilike $3)
           and ($4::text is null or o.kind = $4)
           and ($5::uuid is null or exists (select 1 from postings p where p.opportunity_id = o.id and p.source_id = $5))
           and ($7::text is null or o.advertised_state = $7)
           and ($6::bool or o.closed_at is null)",
    )
    .bind(query.q.as_deref().filter(|q| !q.trim().is_empty()))
    .bind(query.country.as_deref().filter(|c| !c.trim().is_empty()))
    .bind(query.city.as_deref().filter(|c| !c.trim().is_empty()))
    .bind(query.kind.as_deref().filter(|k| !k.trim().is_empty()))
    .bind(query.source)
    .bind(query.include_closed)
    .bind(query.state.as_deref().filter(|s| !s.trim().is_empty()))
    .fetch_one(pool)
    .await?;

    let (offset, limit) = query.offset_limit();
    Ok(OpportunityPage {
        total,
        page: offset / limit + 1,
        per_page: limit,
        items,
    })
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PostingRow {
    pub id: Uuid,
    pub source_name: String,
    pub source_kind: String,
    pub native_id: String,
    pub url: String,
    pub title: String,
    pub description_html: Option<String>,
    pub description_text: Option<String>,
    pub title_only: bool,
    pub location_raw: Option<String>,
    pub department: Option<String>,
    pub employment_type_raw: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub supervisor: Option<String>,
    pub contact: Option<String>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub advertised_state: String,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpportunityDetail {
    pub opportunity: OpportunityRow,
    pub postings: Vec<PostingRow>,
}

pub async fn get_opportunity(
    pool: &Db,
    id: Uuid,
) -> Result<Option<OpportunityDetail>, sqlx::Error> {
    let query = OpportunityQuery {
        q: None,
        country: None,
        city: None,
        kind: None,
        source: None,
        state: None,
        include_closed: true,
        page: Some(1),
        per_page: Some(1),
        sort: None,
    };
    let mut page = list_opportunities_by_id(pool, &query, id).await?;
    let Some(opportunity) = page.pop() else {
        return Ok(None);
    };
    let postings = sqlx::query_as::<_, PostingRow>(
        "select p.id, s.display_name as source_name, s.kind as source_kind, p.native_id, p.url, p.title,
                p.description_html, p.description_text, p.title_only, p.location_raw, p.department,
                p.employment_type_raw, p.posted_at, p.supervisor, p.contact, p.deadline_at, p.advertised_state,
                p.first_seen_at, p.last_seen_at, p.closed_at
         from postings p join sources s on s.id = p.source_id
         where p.opportunity_id = $1 order by p.first_seen_at",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(Some(OpportunityDetail {
        opportunity,
        postings,
    }))
}

async fn list_opportunities_by_id(
    pool: &Db,
    _query: &OpportunityQuery,
    id: Uuid,
) -> Result<Vec<OpportunityRow>, sqlx::Error> {
    sqlx::query_as::<_, OpportunityRow>(
        "select o.id, o.canonical_title, org.display_name as organization, o.kind, o.advertised_state,
                o.country_code, o.city, o.remote_raw, o.deadline_at, o.supervisor, o.contact, o.first_seen_at, o.last_seen_at, o.closed_at,
                coalesce(bool_and(p.title_only), true) as title_only,
                count(p.id) as posting_count,
                min(p.url) as apply_url,
                coalesce(array_agg(distinct s.display_name) filter (where s.display_name is not null), '{}') as source_names,
                max(p.posted_at) as posted_at
         from opportunities o
         join organizations org on org.id = o.organization_id
         left join postings p on p.opportunity_id = o.id
         left join sources s on s.id = p.source_id
         where o.id = $1
         group by o.id, org.display_name",
    )
    .bind(id)
    .fetch_all(pool)
    .await
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct CoverageRow {
    pub source_id: Uuid,
    pub key: String,
    pub display_name: String,
    pub kind: String,
    pub enabled: bool,
    pub health: String,
    /// The honest label: what has been proven about this source, not what the adapter could in principle do.
    pub support: String,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_outcome: Option<String>,
    pub last_diagnostic: Option<String>,
    pub open_postings: i64,
    pub last_complete_sweep_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub consecutive_failures: i32,
    pub backoff_until: Option<DateTime<Utc>>,
    pub refresh_hours: i32,
    /// Freshness is a separate axis from support: `support` says what has been proven about the adapter, `freshness` says whether what it found is still recent. A source can be `verified_live` and `stale` at the same time, and hiding that would present old data as current.
    pub freshness: String,
    pub open_topics: i64,
}

/// Coverage dashboard data. `open_postings` counts only what this tool actually holds; a source that has never been read reports zero with a `not_configured` or `fixture_only` label rather than an implied empty board.
pub async fn coverage(pool: &Db) -> Result<Vec<CoverageRow>, sqlx::Error> {
    sqlx::query_as::<_, CoverageRow>(
        "select s.id as source_id, s.key, s.display_name, s.kind, s.enabled, s.health, s.support,
                s.last_attempt_at, s.last_success_at, s.last_outcome, s.last_diagnostic,
                (select count(*) from postings p where p.source_id = s.id and p.closed_at is null) as open_postings,
                (select max(r.started_at) from source_runs r where r.source_id = s.id and r.complete) as last_complete_sweep_at,
                s.next_run_at, s.consecutive_failures, s.backoff_until, s.refresh_hours,
                -- Stale once a source has missed three of its own refresh cycles, so the threshold follows each source's cadence instead of one global guess.
                case
                    when s.last_success_at is null then 'never'
                    when s.last_success_at < now() - make_interval(hours => s.refresh_hours * 3) then 'stale'
                    else 'fresh'
                end as freshness,
                (select count(*) from postings p where p.source_id = s.id and p.closed_at is null and p.advertised_state = 'open_topic') as open_topics
         from sources s order by s.key",
    )
    .fetch_all(pool)
    .await
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SourceRunRow {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_key: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: String,
    pub complete: bool,
    pub pages_fetched: i32,
    pub records_seen: i32,
    pub total_reported: Option<i32>,
    pub diagnostic: Option<String>,
}

pub async fn source_runs(
    pool: &Db,
    source_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<SourceRunRow>, sqlx::Error> {
    sqlx::query_as::<_, SourceRunRow>(
        "select r.id, r.source_id, s.key as source_key, r.started_at, r.finished_at, r.outcome, r.complete,
                r.pages_fetched, r.records_seen, r.total_reported, r.diagnostic
         from source_runs r join sources s on s.id = r.source_id
         where ($1::uuid is null or r.source_id = $1)
         order by r.started_at desc limit $2",
    )
    .bind(source_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub sources_total: i64,
    pub sources_enabled: i64,
    pub sources_verified_live: i64,
    pub opportunities_open: i64,
    pub opportunities_closed: i64,
    pub postings_open: i64,
    pub organizations: i64,
    pub last_complete_sweep_at: Option<DateTime<Utc>>,
    pub sources_stale: i64,
    pub sources_in_backoff: i64,
    pub open_topics: i64,
    pub next_scheduled_run_at: Option<DateTime<Utc>>,
}

pub async fn stats(pool: &Db) -> Result<Stats, sqlx::Error> {
    // One row of scalar counts; naming it keeps the tuple readable and satisfies the complexity lint.
    type StatsRow = (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<DateTime<Utc>>,
        i64,
        i64,
        i64,
        Option<DateTime<Utc>>,
    );
    let row: StatsRow = sqlx::query_as(
        "select
            (select count(*) from sources),
            (select count(*) from sources where enabled),
            (select count(*) from sources where support = 'verified_live'),
            (select count(*) from opportunities where closed_at is null),
            (select count(*) from opportunities where closed_at is not null),
            (select count(*) from postings where closed_at is null),
            (select count(*) from organizations),
            (select max(started_at) from source_runs where complete),
            (select count(*) from sources where enabled and last_success_at is not null
                and last_success_at < now() - make_interval(hours => refresh_hours * 3)),
            (select count(*) from sources where backoff_until is not null and backoff_until > now()),
            (select count(*) from opportunities where closed_at is null and advertised_state = 'open_topic'),
            (select min(next_run_at) from sources where enabled and next_run_at is not null)",
    )
    .fetch_one(pool)
    .await?;
    Ok(Stats {
        sources_total: row.0,
        sources_enabled: row.1,
        sources_verified_live: row.2,
        opportunities_open: row.3,
        opportunities_closed: row.4,
        postings_open: row.5,
        organizations: row.6,
        last_complete_sweep_at: row.7,
        sources_stale: row.8,
        sources_in_backoff: row.9,
        open_topics: row.10,
        next_scheduled_run_at: row.11,
    })
}

use chrono::{DateTime, Utc};
use opportunity_domain::{
    classify_kind, content_hash, dedupe_key, detect_advertised_state, strip_html, FetchedPosting,
    RunOutcome, ScanResult, MISSING_SWEEPS_BEFORE_CLOSE,
};
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Db;
use crate::snapshot;
use crate::sources::{self, Source, SourceError};

#[derive(Debug, Clone, Serialize)]
pub struct IngestSummary {
    pub run_id: Uuid,
    pub source_key: String,
    pub outcome: String,
    pub complete: bool,
    pub records_seen: i32,
    pub postings_created: i32,
    pub postings_updated: i32,
    pub opportunities_created: i32,
    pub revisions_recorded: i32,
    pub postings_marked_missing: i32,
    pub postings_closed: i32,
    pub diagnostic: Option<String>,
}

/// Persist one sweep. The whole function is one transaction: either the run, its postings and its disappearance decisions all land, or none of them do, so a crash mid-ingest cannot leave postings marked missing by a run that was never recorded.
pub async fn ingest(
    pool: &Db,
    source: &Source,
    scan: ScanResult,
) -> Result<IngestSummary, SourceError> {
    let started_at = Utc::now();
    let mut tx = pool.begin().await?;

    let raw_document_id = match (&scan.raw_body, &scan.raw_url) {
        (Some(body), Some(url)) => snapshot::store(&mut tx, source.id, url, body).await?,
        _ => None,
    };

    let organization_id = match source.organization_id {
        Some(id) => id,
        None => sources::upsert_organization(pool, &source.display_name).await?,
    };

    let mut summary = IngestSummary {
        run_id: Uuid::nil(),
        source_key: source.key.clone(),
        outcome: scan.outcome.as_str().to_owned(),
        complete: scan.outcome.is_complete_sweep(),
        records_seen: scan.stats.records_seen,
        postings_created: 0,
        postings_updated: 0,
        opportunities_created: 0,
        revisions_recorded: 0,
        postings_marked_missing: 0,
        postings_closed: 0,
        diagnostic: scan.diagnostic.clone(),
    };

    let mut observed: Vec<String> = Vec::with_capacity(scan.postings.len());
    for posting in &scan.postings {
        // A source that repeats a native identifier across pages describes one posting twice; persisting it twice would create a phantom duplicate.
        if observed.contains(&posting.native_id) {
            continue;
        }
        observed.push(posting.native_id.clone());
        let outcome =
            upsert_posting(&mut tx, source, organization_id, posting, scan.outcome).await?;
        match outcome {
            UpsertOutcome::Created => summary.postings_created += 1,
            UpsertOutcome::Updated { revised } => {
                summary.postings_updated += 1;
                if revised {
                    summary.revisions_recorded += 1;
                }
            }
        }
    }
    summary.opportunities_created =
        count_new_opportunities(&mut tx, organization_id, started_at).await?;

    // The single place disappearance is decided. Anything other than a sweep that saw the whole inventory leaves every existing posting exactly as it was.
    if scan.outcome.is_complete_sweep() {
        let (missing, closed) = apply_disappearance(&mut tx, source.id, &observed).await?;
        summary.postings_marked_missing = missing;
        summary.postings_closed = closed;
    }

    let run_id: Uuid = sqlx::query_scalar(
        "insert into source_runs (source_id, started_at, finished_at, outcome, complete, pages_fetched, records_seen, total_reported, observed_native_ids, raw_document_id, diagnostic)
         values ($1, $2, now(), $3, $4, $5, $6, $7, $8, $9, $10) returning id",
    )
    .bind(source.id)
    .bind(started_at)
    .bind(scan.outcome.as_str())
    .bind(scan.outcome.is_complete_sweep())
    .bind(scan.stats.pages_fetched)
    .bind(scan.stats.records_seen)
    .bind(scan.stats.total_reported)
    .bind(&observed)
    .bind(raw_document_id)
    .bind(scan.diagnostic.as_deref())
    .fetch_one(&mut *tx)
    .await?;
    summary.run_id = run_id;

    tx.commit().await?;
    sources::record_attempt(pool, source.id, scan.outcome, scan.diagnostic.as_deref()).await?;
    Ok(summary)
}

/// A board that advertises vacancies, as opposed to an academic page that may be offering a topic or only naming a supervisor.
fn from_ats(source: &Source) -> bool {
    matches!(
        source.kind.as_str(),
        "greenhouse" | "lever" | "ashby" | "workday"
    )
}

enum UpsertOutcome {
    Created,
    Updated { revised: bool },
}

async fn upsert_posting(
    tx: &mut Transaction<'_, Postgres>,
    source: &Source,
    organization_id: Uuid,
    posting: &FetchedPosting,
    outcome: RunOutcome,
) -> Result<UpsertOutcome, SourceError> {
    let description_text = posting.description_html.as_deref().map(strip_html);
    let hash = content_hash(
        &posting.title,
        description_text.as_deref(),
        posting.location_raw.as_deref(),
        &posting.url,
    );
    let title_only = posting.is_title_only();
    // An ATS board advertises vacancies; an academic page says what it is offering, and that statement is carried through rather than overwritten.
    let advertised = posting.advertised_state.unwrap_or_else(|| {
        detect_advertised_state(
            classify_kind(&posting.title, posting.employment_type_raw.as_deref()),
            from_ats(source),
        )
    });
    let opportunity_id =
        link_opportunity(tx, organization_id, source, posting, &description_text).await?;
    let complete_seen: Option<DateTime<Utc>> = outcome.is_complete_sweep().then(Utc::now);

    let row: Option<(Uuid, String)> = sqlx::query_as(
        "select id, content_hash from postings where source_id = $1 and native_id = $2",
    )
    .bind(source.id)
    .bind(&posting.native_id)
    .fetch_optional(&mut **tx)
    .await?;

    match row {
        None => {
            sqlx::query(
                "insert into postings (source_id, native_id, opportunity_id, url, title, description_html, description_text, title_only, location_raw, country_code, city, remote_raw, department, employment_type_raw, posted_at, content_hash, last_complete_seen_at, supervisor, contact, deadline_at, advertised_state)
                 values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)",
            )
            .bind(source.id)
            .bind(&posting.native_id)
            .bind(opportunity_id)
            .bind(&posting.url)
            .bind(&posting.title)
            .bind(posting.description_html.as_deref())
            .bind(description_text.as_deref())
            .bind(title_only)
            .bind(posting.location_raw.as_deref())
            .bind(posting.country_code.as_deref())
            .bind(posting.city.as_deref())
            .bind(posting.remote_raw.as_deref())
            .bind(posting.department.as_deref())
            .bind(posting.employment_type_raw.as_deref())
            .bind(posting.posted_at)
            .bind(&hash)
            .bind(complete_seen)
            .bind(posting.supervisor.as_deref())
            .bind(posting.contact.as_deref())
            .bind(posting.deadline_at)
            .bind(advertised.as_str())
            .execute(&mut **tx)
            .await?;
            Ok(UpsertOutcome::Created)
        }
        Some((posting_id, previous_hash)) => {
            let revised = previous_hash != hash;
            sqlx::query(
                "update postings set
                    opportunity_id = $2, url = $3, title = $4, description_html = $5, description_text = $6,
                    title_only = $7, location_raw = $8, country_code = $9, city = $10, remote_raw = $11,
                    department = $12, employment_type_raw = $13, posted_at = $14, content_hash = $15,
                    supervisor = $17, contact = $18, deadline_at = $19, advertised_state = $20,
                    last_seen_at = now(),
                    last_complete_seen_at = coalesce($16, last_complete_seen_at),
                    -- A posting that is present again is present: the streak resets and a previous closure is undone.
                    missing_streak = 0,
                    closed_at = null
                 where id = $1",
            )
            .bind(posting_id)
            .bind(opportunity_id)
            .bind(&posting.url)
            .bind(&posting.title)
            .bind(posting.description_html.as_deref())
            .bind(description_text.as_deref())
            .bind(title_only)
            .bind(posting.location_raw.as_deref())
            .bind(posting.country_code.as_deref())
            .bind(posting.city.as_deref())
            .bind(posting.remote_raw.as_deref())
            .bind(posting.department.as_deref())
            .bind(posting.employment_type_raw.as_deref())
            .bind(posting.posted_at)
            .bind(&hash)
            .bind(complete_seen)
            .bind(posting.supervisor.as_deref())
            .bind(posting.contact.as_deref())
            .bind(posting.deadline_at)
            .bind(advertised.as_str())
            .execute(&mut **tx)
            .await?;
            if revised {
                if let Some(opportunity_id) = opportunity_id {
                    sqlx::query(
                        "insert into opportunity_revisions (opportunity_id, posting_id, content_hash, change_reason, diff)
                         values ($1, $2, $3, 'source content changed', jsonb_build_object('previous_hash', $4::text))",
                    )
                    .bind(opportunity_id)
                    .bind(posting_id)
                    .bind(&hash)
                    .bind(&previous_hash)
                    .execute(&mut **tx)
                    .await?;
                }
            }
            Ok(UpsertOutcome::Updated { revised })
        }
    }
}

/// Attach the posting to a canonical opportunity, creating one when this is the first posting that describes it. Two sources describing the same role converge here through the conservative dedupe key; anything the key does not consider identical stays a separate opportunity.
async fn link_opportunity(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Uuid,
    source: &Source,
    posting: &FetchedPosting,
    description_text: &Option<String>,
) -> Result<Option<Uuid>, SourceError> {
    let key = dedupe_key(
        &source.display_name,
        &posting.title,
        posting.location_raw.as_deref(),
    );
    let kind = classify_kind(&posting.title, posting.employment_type_raw.as_deref());
    // An academic adapter states what the page is offering; only when a source says nothing does the kind-based default apply.
    let advertised = posting
        .advertised_state
        .unwrap_or_else(|| detect_advertised_state(kind, from_ats(source)));
    let search_text = [
        Some(posting.title.as_str()),
        description_text.as_deref(),
        posting.department.as_deref(),
        posting.location_raw.as_deref(),
        posting.supervisor.as_deref(),
        Some(source.display_name.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");

    let id: Uuid = sqlx::query_scalar(
        "insert into opportunities (organization_id, dedupe_key, canonical_title, kind, advertised_state, country_code, city, remote_raw, search_text, supervisor, contact, deadline_at)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
         on conflict (organization_id, dedupe_key) do update set
            last_seen_at = now(),
            closed_at = null,
            canonical_title = excluded.canonical_title,
            kind = excluded.kind,
            advertised_state = excluded.advertised_state,
            country_code = coalesce(excluded.country_code, opportunities.country_code),
            city = coalesce(excluded.city, opportunities.city),
            remote_raw = coalesce(excluded.remote_raw, opportunities.remote_raw),
            supervisor = coalesce(excluded.supervisor, opportunities.supervisor),
            contact = coalesce(excluded.contact, opportunities.contact),
            deadline_at = coalesce(excluded.deadline_at, opportunities.deadline_at),
            -- The longer text wins so a title-only posting from a second source cannot erase a full description from the first.
            search_text = case when length(excluded.search_text) > length(opportunities.search_text) then excluded.search_text else opportunities.search_text end,
            updated_at = now()
         returning id",
    )
    .bind(organization_id)
    .bind(&key)
    .bind(&posting.title)
    .bind(kind.as_str())
    .bind(advertised.as_str())
    .bind(posting.country_code.as_deref())
    .bind(posting.city.as_deref())
    .bind(posting.remote_raw.as_deref())
    .bind(&search_text)
    .bind(posting.supervisor.as_deref())
    .bind(posting.contact.as_deref())
    .bind(posting.deadline_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(id))
}

async fn count_new_opportunities(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Uuid,
    since: DateTime<Utc>,
) -> Result<i32, SourceError> {
    let count: i64 = sqlx::query_scalar(
        "select count(*) from opportunities where organization_id = $1 and created_at >= $2",
    )
    .bind(organization_id)
    .bind(since)
    .fetch_one(&mut **tx)
    .await?;
    Ok(count as i32)
}

/// Increment the missing streak for postings the sweep did not observe, and close the ones that have now been missed by enough consecutive complete sweeps. Called only for a complete sweep.
async fn apply_disappearance(
    tx: &mut Transaction<'_, Postgres>,
    source_id: Uuid,
    observed: &[String],
) -> Result<(i32, i32), SourceError> {
    let missing = sqlx::query(
        "update postings set missing_streak = missing_streak + 1
         where source_id = $1 and closed_at is null and not (native_id = any($2))",
    )
    .bind(source_id)
    .bind(observed)
    .execute(&mut **tx)
    .await?
    .rows_affected();

    let closed = sqlx::query(
        "update postings set closed_at = now()
         where source_id = $1 and closed_at is null and missing_streak >= $2",
    )
    .bind(source_id)
    .bind(MISSING_SWEEPS_BEFORE_CLOSE)
    .execute(&mut **tx)
    .await?
    .rows_affected();

    // An opportunity closes only when every posting that described it has closed, so one source dropping a cross-posted role does not hide it.
    sqlx::query(
        "update opportunities o set closed_at = now(), updated_at = now()
         where o.closed_at is null
           and exists (select 1 from postings p where p.opportunity_id = o.id)
           and not exists (select 1 from postings p where p.opportunity_id = o.id and p.closed_at is null)",
    )
    .execute(&mut **tx)
    .await?;

    Ok((missing as i32, closed as i32))
}

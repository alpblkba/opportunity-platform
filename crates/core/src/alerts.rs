//! Emails the owner asked for about opportunities: at most one digest a day of new matches, and
//! reminders before deadlines a posting actually stated. Both go through the notification outbox with
//! deterministic keys, so repeated ticks and restarts never queue one twice. Nothing is sent to an
//! employer; deadlines are never inferred from a posting date.

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::applications::{self, AppError};
use crate::{preferences, product, profiles, Db};

const DIGEST_ITEMS: usize = 10;

/// Queue today's digest once the owner's digest hour has passed in their time zone. Returns how
/// many matches it listed, or `None` when nothing was due. An evaluation with no new matches still
/// counts as today's, so the ranking is not recomputed every tick.
pub async fn queue_new_match_digest(db: &Db) -> Result<Option<usize>, AppError> {
    let prefs = preferences::notification_preferences(db).await?;
    if !prefs.new_match_digest {
        return Ok(None);
    }
    let owner = profiles::local_owner(db).await?;
    let (due, today, last): (bool, String, Option<DateTime<Utc>>) = sqlx::query_as(
        "select extract(hour from now() at time zone time_zone)::int >= digest_hour
                and (last_digest_at is null or (last_digest_at at time zone time_zone)::date < (now() at time zone time_zone)::date),
                ((now() at time zone time_zone)::date)::text, last_digest_at
         from notification_preferences where owner_id = $1",
    )
    .bind(owner)
    .fetch_one(db)
    .await?;
    if !due {
        return Ok(None);
    }
    let since = last.unwrap_or_else(|| Utc::now() - Duration::days(1));
    let profile_id = product::selection(db).await?;
    let profile = profiles::get(db, profile_id).await?;
    let profile_name = product::TRACKS
        .iter()
        .find(|(key, _)| *key == profile.profile.key)
        .map_or(profile.profile.display_name.as_str(), |(_, label)| *label)
        .to_owned();
    let items = digest_items(
        product::ranked(db, profile_id).await?,
        since,
        prefs.min_priority as f32,
    );
    let mut tx = db.begin().await?;
    if !items.is_empty() {
        sqlx::query(
            "insert into notifications (owner_id, kind, dedupe_key, payload) values ($1, 'new_matches', $2, $3)
             on conflict (owner_id, dedupe_key) do nothing",
        )
        .bind(owner)
        .bind(format!("digest:{today}"))
        .bind(json!({ "profile": profile_name, "since": since, "items": items }))
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("update notification_preferences set last_digest_at = now() where owner_id = $1")
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(items.len()))
}

/// The digest's entries: matches first seen after `since`, at or above the threshold, best first.
fn digest_items(
    ranked: Vec<product::RankedJob>,
    since: DateTime<Utc>,
    min_priority: f32,
) -> Vec<Value> {
    ranked
        .into_iter()
        .filter(|j| j.job.first_seen_at > since && j.score >= min_priority)
        .take(DIGEST_ITEMS)
        .map(|j| {
            let location = [j.job.city.clone(), j.job.country_code.clone()].into_iter().flatten().collect::<Vec<_>>().join(", ");
            json!({
                "opportunity_id": j.job.id,
                "title": j.job.canonical_title,
                "employer": j.job.organization,
                "location": if location.is_empty() { "Location not stated".into() } else { location },
                "stage": j.career_stage,
                "priority": j.score.round() as i64,
                "explanation": j.explanation,
                "gap": j.gaps.first(),
                "deadline": j.job.deadline_at.map(|d| d.date_naive().to_string()),
                "url": j.job.apply_url,
            })
        })
        .collect()
}

/// One reminder per stated deadline date and reminder threshold, for opportunities the owner saved,
/// flagged or is applying to. Nothing for dismissed ones, closed ones, or once the owner's application
/// went out. A deadline that moves to another date is a new deadline and gets its own reminders.
pub async fn queue_deadline_reminders(db: &Db) -> Result<usize, AppError> {
    let prefs = preferences::notification_preferences(db).await?;
    let Some(furthest) = prefs.reminder_days.iter().max().copied() else {
        return Ok(0);
    };
    if !prefs.deadline_reminders {
        return Ok(0);
    }
    let owner = profiles::local_owner(db).await?;
    let due: Vec<(Uuid, String, String, String, f64, Option<String>)> = sqlx::query_as(
        "select o.id, o.canonical_title, org.display_name, to_char(o.deadline_at at time zone $3, 'YYYY-MM-DD'),
                (extract(epoch from o.deadline_at - now()) / 86400.0)::float8,
                (select p.url from postings p where p.opportunity_id = o.id and p.closed_at is null order by p.url limit 1)
         from opportunities o join organizations org on org.id = o.organization_id
         where o.closed_at is null and o.deadline_at > now() and o.deadline_at <= now() + make_interval(days => $1)
           and (exists (select 1 from opportunity_marks m where m.owner_id = $2 and m.opportunity_id = o.id)
                or exists (select 1 from applications a where a.owner_id = $2 and a.opportunity_id = o.id
                           and a.status in ('saved', 'preparing', 'needs_review', 'ready', 'failed', 'unsupported')))
           and not exists (select 1 from exclusion_rules r where r.owner_id = $2 and r.enabled and r.kind = 'hidden' and r.opportunity_id = o.id)
           and not exists (select 1 from applications a where a.owner_id = $2 and a.opportunity_id = o.id
                           and a.status in ('submitting', 'submitted', 'submission_unknown', 'interview', 'rejected', 'withdrawn'))",
    )
    .bind(furthest)
    .bind(owner)
    .bind(&prefs.time_zone)
    .fetch_all(db)
    .await?;
    let mut queued = 0;
    for (id, title, employer, deadline, days, url) in due {
        let Some(threshold) = reminder_threshold(days, &prefs.reminder_days) else {
            continue;
        };
        let key = format!("deadline:{id}:{deadline}:{threshold}");
        let payload = json!({ "opportunity_id": id, "title": title, "employer": employer, "deadline": deadline,
                              "days_left": days.ceil() as i64, "url": url.unwrap_or_default() });
        if applications::notify(db, None, "deadline_reminder", &key, payload).await? {
            queued += 1;
        }
    }
    Ok(queued)
}

/// The smallest configured threshold that `days_left` has crossed: a deadline 2.5 days out with
/// thresholds 7, 3, 1 is the 3-day reminder, so saving one late never sends all three at once.
fn reminder_threshold(days_left: f64, thresholds: &[i32]) -> Option<i32> {
    thresholds
        .iter()
        .copied()
        .filter(|d| days_left <= f64::from(*d))
        .min()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_late_save_gets_one_reminder_per_threshold_crossed_not_all_of_them() {
        let thresholds = [7, 3, 1];
        assert_eq!(super::reminder_threshold(10.0, &thresholds), None);
        assert_eq!(super::reminder_threshold(6.5, &thresholds), Some(7));
        assert_eq!(super::reminder_threshold(2.5, &thresholds), Some(3));
        assert_eq!(super::reminder_threshold(0.4, &thresholds), Some(1));
        assert_eq!(super::reminder_threshold(2.5, &[]), None);
    }
}

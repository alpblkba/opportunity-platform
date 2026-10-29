//! The owner's standing choices about opportunities and alerts: marks (saved, review), dismissed
//! opportunities and blocked employers (exclusion rules the ranking already applies), notification
//! preferences, and the last visit that "new" is measured against.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::applications::AppError;
use crate::profiles::{self, NewRule, StoredRule};
use crate::Db;

pub const MARKS: [&str; 2] = ["saved", "review"];

/// Save an opportunity, or flag it for review, or take either back. Ranking is unaffected.
pub async fn set_mark(db: &Db, opportunity_id: Uuid, mark: &str, on: bool) -> Result<(), AppError> {
    if !MARKS.contains(&mark) {
        return Err(AppError::Invalid(
            "A mark is either saved or review.".into(),
        ));
    }
    let owner = profiles::local_owner(db).await?;
    if on {
        let exists: bool =
            sqlx::query_scalar("select exists(select 1 from opportunities where id = $1)")
                .bind(opportunity_id)
                .fetch_one(db)
                .await?;
        if !exists {
            return Err(AppError::NotFound(opportunity_id));
        }
        sqlx::query("insert into opportunity_marks (owner_id, opportunity_id, mark) values ($1, $2, $3) on conflict do nothing")
            .bind(owner)
            .bind(opportunity_id)
            .bind(mark)
            .execute(db)
            .await?;
    } else {
        sqlx::query("delete from opportunity_marks where owner_id = $1 and opportunity_id = $2 and mark = $3")
            .bind(owner)
            .bind(opportunity_id)
            .bind(mark)
            .execute(db)
            .await?;
    }
    Ok(())
}

pub async fn marks(db: &Db) -> Result<Vec<(Uuid, String)>, AppError> {
    let owner = profiles::local_owner(db).await?;
    Ok(sqlx::query_as("select opportunity_id, mark from opportunity_marks where owner_id = $1 order by created_at")
        .bind(owner)
        .fetch_all(db)
        .await?)
}

/// Hide one opportunity from every profile. Undone by deleting the rule, which keeps its history.
pub async fn dismiss(db: &Db, opportunity_id: Uuid) -> Result<StoredRule, AppError> {
    profiles::create_rule(
        db,
        NewRule {
            kind: "hidden".into(),
            value: String::new(),
            profile_id: None,
            opportunity_id: Some(opportunity_id),
            note: Some("Dismissed in the workspace".into()),
        },
    )
    .await
}

/// Stop ranking an employer for every profile. Its sources, postings and applications are kept.
pub async fn block_employer(db: &Db, employer: &str) -> Result<StoredRule, AppError> {
    let employer = employer.trim();
    if employer.is_empty() || employer.chars().count() > 200 {
        return Err(AppError::Invalid(
            "Name the employer, in at most 200 characters.".into(),
        ));
    }
    profiles::create_rule(
        db,
        NewRule {
            kind: "company".into(),
            value: employer.into(),
            profile_id: None,
            opportunity_id: None,
            note: Some("Blocked in the workspace".into()),
        },
    )
    .await
}

/// Every enabled rule, with the title of a hidden opportunity, for the workspace's undo list.
pub async fn exclusions(db: &Db) -> Result<Vec<Value>, AppError> {
    let owner = profiles::local_owner(db).await?;
    Ok(sqlx::query_scalar(
        "select jsonb_build_object('id', r.id, 'kind', r.kind, 'value', r.value, 'profile_id', r.profile_id,
                                   'opportunity_id', r.opportunity_id, 'title', o.canonical_title, 'created_at', r.created_at)
         from exclusion_rules r left join opportunities o on o.id = r.opportunity_id
         where r.owner_id = $1 and r.enabled order by r.created_at desc limit 300",
    )
    .bind(owner)
    .fetch_all(db)
    .await?)
}

/// Record a visit and return the one before it, which is what "new" is measured against.
pub async fn record_visit(db: &Db) -> Result<Option<DateTime<Utc>>, AppError> {
    let owner = profiles::local_owner(db).await?;
    let previous: Option<DateTime<Utc>> =
        sqlx::query_scalar("select last_visit_at from product_settings where owner_id = $1")
            .bind(owner)
            .fetch_optional(db)
            .await?
            .flatten();
    sqlx::query("insert into product_settings (owner_id, last_visit_at) values ($1, now()) on conflict (owner_id) do update set last_visit_at = now(), updated_at = now()")
        .bind(owner)
        .execute(db)
        .await?;
    Ok(previous)
}

pub async fn last_visit(db: &Db) -> Result<Option<DateTime<Utc>>, AppError> {
    let owner = profiles::local_owner(db).await?;
    Ok(
        sqlx::query_scalar("select last_visit_at from product_settings where owner_id = $1")
            .bind(owner)
            .fetch_optional(db)
            .await?
            .flatten(),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct NotificationPreferences {
    pub new_match_digest: bool,
    pub min_priority: i32,
    pub digest_hour: i32,
    pub time_zone: String,
    pub deadline_reminders: bool,
    pub reminder_days: Vec<i32>,
}

/// The owner's preferences, created with the schema's defaults on first read.
pub async fn notification_preferences(db: &Db) -> Result<NotificationPreferences, AppError> {
    let owner = profiles::local_owner(db).await?;
    sqlx::query(
        "insert into notification_preferences (owner_id) values ($1) on conflict do nothing",
    )
    .bind(owner)
    .execute(db)
    .await?;
    Ok(sqlx::query_as(
        "select new_match_digest, min_priority, digest_hour, time_zone, deadline_reminders, reminder_days
         from notification_preferences where owner_id = $1",
    )
    .bind(owner)
    .fetch_one(db)
    .await?)
}

pub async fn set_notification_preferences(
    db: &Db,
    mut preferences: NotificationPreferences,
) -> Result<NotificationPreferences, AppError> {
    if !(0..=100).contains(&preferences.min_priority) {
        return Err(AppError::Invalid(
            "Minimum priority is between 0 and 100.".into(),
        ));
    }
    if !(0..=23).contains(&preferences.digest_hour) {
        return Err(AppError::Invalid(
            "The digest hour is between 0 and 23.".into(),
        ));
    }
    preferences.reminder_days.sort_unstable_by(|a, b| b.cmp(a));
    preferences.reminder_days.dedup();
    if preferences.reminder_days.len() > 5
        || preferences
            .reminder_days
            .iter()
            .any(|d| !(1..=60).contains(d))
    {
        return Err(AppError::Invalid(
            "Up to five reminders, each 1 to 60 days ahead.".into(),
        ));
    }
    // PostgreSQL knows the IANA zones; an unknown one is an error there, reported as such here.
    let zone = preferences.time_zone.trim().to_owned();
    if zone.is_empty()
        || zone.len() > 64
        || sqlx::query("select now() at time zone $1")
            .bind(&zone)
            .execute(db)
            .await
            .is_err()
    {
        return Err(AppError::Invalid(
            "Use a time zone name such as Europe/Berlin.".into(),
        ));
    }
    preferences.time_zone = zone;
    let owner = profiles::local_owner(db).await?;
    sqlx::query(
        "insert into notification_preferences (owner_id, new_match_digest, min_priority, digest_hour, time_zone, deadline_reminders, reminder_days)
         values ($1, $2, $3, $4, $5, $6, $7)
         on conflict (owner_id) do update set new_match_digest = $2, min_priority = $3, digest_hour = $4, time_zone = $5,
             deadline_reminders = $6, reminder_days = $7, updated_at = now()",
    )
    .bind(owner)
    .bind(preferences.new_match_digest)
    .bind(preferences.min_priority)
    .bind(preferences.digest_hour)
    .bind(&preferences.time_zone)
    .bind(preferences.deadline_reminders)
    .bind(&preferences.reminder_days)
    .execute(db)
    .await?;
    notification_preferences(db).await
}

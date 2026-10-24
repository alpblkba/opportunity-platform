import type { Freshness, Opportunity, RunOutcome, Support } from "./types";

const KIND_LABELS: Record<string, string> = {
  employment: "Employment",
  part_time: "Part time",
  internship: "Internship",
  working_student: "Working student",
  master_thesis: "Master thesis",
  research_assistant: "Research assistant",
  phd: "PhD",
  fellowship: "Fellowship",
  other: "Other",
};

const SUPPORT_LABELS: Record<Support, string> = {
  verified_live: "Verified live",
  fixture_only: "Fixture only",
  blocked: "Blocked",
  manual_link: "Manual link",
  not_configured: "Never checked",
};

const OUTCOME_LABELS: Record<RunOutcome, string> = {
  ok_complete: "Complete sweep",
  ok_verified_empty: "Complete sweep, no openings",
  partial: "Partial sweep",
  not_modified: "Not modified",
  access_denied: "Access denied",
  schema_error: "Schema changed",
  http_error: "HTTP error",
  rate_limited: "Rate limited",
};

const STATE_LABELS: Record<string, string> = {
  advertised: "Advertised vacancy",
  open_topic: "Open topic, not a funded post",
  potential_supervisor: "Possible supervisor, nothing advertised",
  unknown: "Offer unclear",
};

const FRESHNESS_LABELS: Record<Freshness, string> = {
  fresh: "Up to date",
  stale: "Stale, overdue for a refresh",
  never: "Never read",
};

export const kindLabel = (kind: string): string => KIND_LABELS[kind] ?? kind;
export const stateLabel = (state: string): string => STATE_LABELS[state] ?? state;
export const freshnessLabel = (freshness: Freshness): string => FRESHNESS_LABELS[freshness] ?? freshness;

/// A thesis topic or a supervisor contact must never be read as a funded vacancy, so anything other than an advertised post is called out.
export const isAdvertisedVacancy = (state: string): boolean => state === "advertised";

/// Whole days until a deadline, or null when the source did not state one. Negative means it has passed.
///
/// Compared as calendar days rather than elapsed time: a deadline at 23:59 tonight is today, not tomorrow, and showing it as tomorrow could cost a real application.
export function daysUntil(iso: string | null, now: Date = new Date()): number | null {
  if (!iso) return null;
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return null;
  const startOfDay = (d: Date) => Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate());
  return Math.round((startOfDay(then) - startOfDay(now)) / 86_400_000);
}

export function deadlineLabel(iso: string | null, now: Date = new Date()): string | null {
  const days = daysUntil(iso, now);
  if (days === null) return null;
  if (days < 0) return `Deadline passed ${formatDate(iso)}`;
  if (days === 0) return `Deadline today, ${formatDate(iso)}`;
  if (days === 1) return `Deadline tomorrow, ${formatDate(iso)}`;
  return `Deadline in ${days} days, ${formatDate(iso)}`;
}
export const supportLabel = (support: Support): string => SUPPORT_LABELS[support] ?? support;
export const outcomeLabel = (outcome: RunOutcome | null): string =>
  outcome === null ? "Never attempted" : (OUTCOME_LABELS[outcome] ?? outcome);

/// An absent value is shown as "unknown" rather than as a default, because the source not saying something is itself information.
export const orUnknown = (value: string | null | undefined): string =>
  value === null || value === undefined || value.trim() === "" ? "unknown" : value;

export function formatDate(iso: string | null): string {
  if (!iso) return "unknown";
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? "unknown" : date.toISOString().slice(0, 10);
}

export function relativeAge(iso: string | null, now: Date = new Date()): string {
  if (!iso) return "unknown";
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "unknown";
  const hours = Math.floor((now.getTime() - then.getTime()) / 3_600_000);
  if (hours < 1) return "under an hour ago";
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

/// Where the opportunity is, as the sources stated it. Physical location and remote wording are shown separately because they answer different questions.
export function locationLabel(opportunity: Pick<Opportunity, "city" | "country_code" | "remote_raw">): string {
  const parts = [opportunity.city, opportunity.country_code].filter((part): part is string => Boolean(part));
  const place = parts.length > 0 ? parts.join(", ") : "location unknown";
  return opportunity.remote_raw ? `${place} (source says: ${opportunity.remote_raw})` : place;
}

export const isStale = (iso: string | null, now: Date = new Date(), hours = 48): boolean => {
  if (!iso) return true;
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return true;
  return now.getTime() - then.getTime() > hours * 3_600_000;
};

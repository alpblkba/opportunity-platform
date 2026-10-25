export type RunOutcome =
  | "ok_complete"
  | "ok_verified_empty"
  | "partial"
  | "not_modified"
  | "access_denied"
  | "schema_error"
  | "http_error"
  | "rate_limited";

export type Support = "verified_live" | "fixture_only" | "blocked" | "manual_link" | "not_configured";

export interface Opportunity {
  id: string;
  canonical_title: string;
  organization: string;
  kind: string;
  advertised_state: string;
  country_code: string | null;
  city: string | null;
  remote_raw: string | null;
  deadline_at: string | null;
  supervisor: string | null;
  contact: string | null;
  first_seen_at: string;
  last_seen_at: string;
  closed_at: string | null;
  title_only: boolean;
  posting_count: number;
  apply_url: string | null;
  source_names: string[];
  posted_at: string | null;
}

export interface OpportunityPage {
  total: number;
  page: number;
  per_page: number;
  items: Opportunity[];
}

export interface Posting {
  id: string;
  source_name: string;
  source_kind: string;
  native_id: string;
  url: string;
  title: string;
  description_html: string | null;
  description_text: string | null;
  title_only: boolean;
  location_raw: string | null;
  department: string | null;
  employment_type_raw: string | null;
  posted_at: string | null;
  supervisor: string | null;
  contact: string | null;
  deadline_at: string | null;
  advertised_state: string;
  first_seen_at: string;
  last_seen_at: string;
  closed_at: string | null;
}

export interface OpportunityDetail {
  opportunity: Opportunity;
  postings: Posting[];
}

export interface Source {
  id: string;
  key: string;
  kind: string;
  display_name: string;
  endpoint: Record<string, string>;
  enabled: boolean;
  refresh_hours: number;
  tags: string[];
  health: string;
  support: Support;
  last_attempt_at: string | null;
  last_success_at: string | null;
  last_outcome: RunOutcome | null;
  last_diagnostic: string | null;
  next_run_at: string | null;
  consecutive_failures: number;
  backoff_until: string | null;
}

export interface CoverageRow {
  source_id: string;
  key: string;
  display_name: string;
  kind: string;
  enabled: boolean;
  health: string;
  support: Support;
  last_attempt_at: string | null;
  last_success_at: string | null;
  last_outcome: RunOutcome | null;
  last_diagnostic: string | null;
  open_postings: number;
  last_complete_sweep_at: string | null;
  next_run_at: string | null;
  consecutive_failures: number;
  backoff_until: string | null;
  refresh_hours: number;
  freshness: Freshness;
  open_topics: number;
}

export type Freshness = "fresh" | "stale" | "never";

export interface SourceRun {
  id: string;
  source_id: string;
  source_key: string;
  started_at: string;
  finished_at: string | null;
  outcome: RunOutcome;
  complete: boolean;
  pages_fetched: number;
  records_seen: number;
  total_reported: number | null;
  diagnostic: string | null;
}

export interface Stats {
  sources_total: number;
  sources_enabled: number;
  sources_verified_live: number;
  opportunities_open: number;
  opportunities_closed: number;
  postings_open: number;
  organizations: number;
  last_complete_sweep_at: string | null;
  sources_stale: number;
  sources_in_backoff: number;
  open_topics: number;
  next_scheduled_run_at: string | null;
}

export interface ValidationReport {
  source_key: string;
  reachable: boolean;
  outcome: RunOutcome;
  records_seen: number;
  complete: boolean;
  diagnostic: string | null;
}

export interface IngestSummary {
  run_id: string;
  source_key: string;
  outcome: RunOutcome;
  complete: boolean;
  records_seen: number;
  postings_created: number;
  postings_updated: number;
  opportunities_created: number;
  revisions_recorded: number;
  postings_marked_missing: number;
  postings_closed: number;
  diagnostic: string | null;
}

export interface Weights {
  terms: number;
  concepts: number;
  title: number;
  location: number;
  kind: number;
  technology: number;
  freshness: number;
  semantic: number;
}

export type RemotePolicy = "any" | "remote_from_current_country_only" | "on_site_or_relocate";
export type SeniorityPolicy = "preference" | "hard" | "ignore";

export interface Profile {
  id: string;
  key: string;
  display_name: string;
  priority: number;
  enabled: boolean;
  job_kinds: string[];
  terms: string[];
  negative_terms: string[];
  preferred_technologies: string[];
  excluded_technologies: string[];
  preferred_countries: string[];
  preferred_cities: string[];
  current_country: string | null;
  relocation_willing: boolean;
  remote_policy: RemotePolicy;
  include_unknown_location: boolean;
  min_years_experience: number | null;
  max_years_experience: number | null;
  seniority_policy: SeniorityPolicy;
  education_level: string | null;
  work_authorization: string[];
  weights: Weights;
  created_at: string;
  updated_at: string;
}

export interface Exclusion {
  reason: string;
  evidence: string | null;
  reversible: boolean;
}

export interface Uncertainty {
  field: string;
  detail: string;
}

export interface Factor {
  name: string;
  contribution: number;
  detail: string;
  evidence: string[];
}

export interface MatchResult {
  score: number;
  eligible: boolean;
  exclusions: Exclusion[];
  factors: Factor[];
  uncertainties: Uncertainty[];
  matched_concepts: string[];
  model: string;
}

export interface ScoredOpportunity extends Opportunity {
  description_text: string | null;
  department: string | null;
  match: MatchResult;
}

export type SemanticStatus =
  | { state: "available"; provider: string; model: string }
  | { state: "unavailable"; provider: string; reason: string }
  | { state: "disabled" };

export interface MatchPage {
  profile_key: string;
  profile_id: string;
  ranking: string;
  semantic: SemanticStatus;
  total_considered: number;
  eligible: number;
  excluded: number;
  items: ScoredOpportunity[];
}

export interface Rule {
  id: string;
  profile_id: string | null;
  kind: "company" | "title_term" | "description_term" | "hidden";
  value: string;
  opportunity_id: string | null;
  note: string | null;
  enabled: boolean;
  created_at: string;
}

export interface Digest {
  id: string;
  profile_id: string | null;
  generated_at: string;
  format: string;
  item_count: number;
  deadline_count: number;
  body: string;
}

export interface Vocabulary {
  concepts: string[];
  job_kinds: string[];
}

export interface CvDocument {
  id: string;
  label: string;
  version: number;
  byte_size: number;
  media_type: string;
  created_at: string;
}

export interface ApplicationRecord {
  id: string;
  opportunity_id: string;
  profile_id: string | null;
  status: string;
  route: string | null;
  route_reason: string | null;
  cv_document_id: string | null;
  dossier_body: string | null;
  dossier_facts: unknown;
  notes: string | null;
  follow_up_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface ApplicationEvent {
  id: string;
  kind: string;
  detail: Record<string, unknown>;
  created_at: string;
}

export interface Attempt {
  id: string;
  idempotency_key: string;
  state: string;
  destination: string;
  payload_hash: string;
  authorization_kind: string | null;
  receipt: string | null;
  diagnostic: string | null;
  evidence: Record<string, unknown>;
  started_at: string;
  finished_at: string | null;
}

export interface ReviewTask {
  id: string;
  application_id: string;
  field_name: string;
  question_text: string;
  field_type: string;
  required: boolean;
  sensitive: boolean;
  draft_answer: string | null;
  answer: string | null;
  status: string;
  created_at: string;
  resolved_at: string | null;
}

export interface ApplicantFact {
  id: string;
  key: string;
  value: string;
  category: string;
  sensitive: boolean;
  approved: boolean;
  provenance: string;
  version: number;
  created_at: string;
}

export interface AuthorizationPolicy {
  id: string;
  name: string;
  enabled: boolean;
  source_keys: string[];
  require_all_fields_mapped: boolean;
  forbid_sensitive_questions: boolean;
  forbid_free_text: boolean;
  max_per_day: number;
  max_total: number | null;
  submissions_made: number;
  expires_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export interface NotificationRecord {
  id: string;
  application_id: string | null;
  kind: "action_required" | "submitted" | "failed" | "submission_uncertain";
  dedupe_key: string;
  payload: Record<string, unknown>;
  created_at: string;
  delivered_at: string | null;
}

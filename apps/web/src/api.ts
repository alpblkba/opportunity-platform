export class ApiError extends Error {
  constructor(message: string, readonly status: number, readonly needs?: string) { super(message); }
}
export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, { ...init, headers: { "content-type": "application/json", ...init?.headers } });
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new ApiError(body.detail || response.statusText, response.status, body.needs);
  }
  return response.status === 204 ? undefined as T : response.json();
}
const post = <T,>(path: string, body = {}) => request<T>(path, { method: "POST", body: JSON.stringify(body) });
export const api = {
  session: () => request<{ email: string; display_name: string; via: string }>("/auth/session"),
  login: (email: string, password: string) => post("/auth/login", { email, password }),
  logout: () => post("/auth/logout"),
  home: (filters: Filters = {}) => {
    const query = new URLSearchParams(Object.entries(filters).filter(([, v]) => v !== undefined && v !== "" && v !== false).map(([k, v]) => [k, String(v)])).toString();
    return request<Home>(`/home${query ? `?${query}` : ""}`);
  },
  visited: () => post<{ previous_visit_at: string | null }>("/home/visited"),
  mark: (id: string, mark: "saved" | "review", on: boolean) => post(`/home/mark/${id}`, { mark, on }),
  dismiss: (id: string) => post<Rule>(`/home/dismiss/${id}`),
  blockEmployer: (employer: string) => post<Rule>("/home/block-employer", { employer }),
  addRule: (kind: "title_term" | "description_term", value: string) => post<Rule>("/rules", { kind, value, note: "Added in the workspace" }),
  unrule: (id: string) => request(`/rules/${id}`, { method: "DELETE" }),
  notifications: () => request<NotificationPreferences>("/home/notifications"),
  saveNotifications: (p: NotificationPreferences) => post<NotificationPreferences>("/home/notifications", p),
  profile: (id: string) => request<Record<string, unknown>>(`/profiles/${id}`),
  saveProfile: (id: string, profile: Record<string, unknown>) => request(`/profiles/${id}`, { method: "PATCH", body: JSON.stringify(profile) }),
  sources: () => request<Source[]>("/sources"),
  setSourceEnabled: (id: string, enabled: boolean) => request<Source>(`/sources/${id}`, { method: "PATCH", body: JSON.stringify({ enabled }) }),
  refreshSource: (id: string) => post(`/sources/${id}/refresh`),
  facts: () => request<Fact[]>("/facts"),
  saveFact: (key: string, value: string, category: string) => post<Fact>("/facts", { key, value, category, provenance: "entered by the owner" }),
  approveFact: (id: string, approved: boolean) => post<Fact>(`/facts/${id}/approve`, { approved }),
  select: (profile_id: string) => post("/home/profile", { profile_id }),
  authorize: (source_key: string, profile_id: string) => post("/home/authorize", { source_key, profile_id }),
  disable: (id: string) => post(`/policies/${id}/state`, { enabled: false, revoke: true }),
  apply: (id: string) => post(`/home/apply/${id}`),
  answer: (id: string, answer: string) => post(`/home/answer/${id}`, { answer }),
  trackExternal: (job: ExternalJob) => post<{ application_id: string; duplicate: boolean }>("/home/track-external", job),
  track: (id: string, status: string, notes: string) => post(`/applications/${id}/tracking`, { status, notes }),
  pause: (paused: boolean) => post("/home/pause", { paused }),
  upload: (file: File) => request("/documents/cv?filename=" + encodeURIComponent(file.name), {
    method: "POST", headers: { "content-type": "application/octet-stream" }, body: file,
  }),
};
export interface Application { id: string; opportunity_id: string; status: string; notes?: string | null; reason?: string; title: string; employer: string; url: string; cv_label?: string | null }
export interface ExternalJob { title: string; employer: string; url: string; deadline_at: string | null }
export interface Question { id: string; application_id: string; question_text: string; field_type: string; options: string[] }
export interface Filters { stage?: string; country?: string; employer?: string; source?: string; eligibility?: string; q?: string; saved?: boolean }
export interface Rule { id: string; kind: string; value: string; opportunity_id?: string | null; title?: string | null }
export interface NotificationPreferences { new_match_digest: boolean; min_priority: number; digest_hour: number; time_zone: string; deadline_reminders: boolean; reminder_days: number[] }
export interface Source { id: string; key: string; kind: string; display_name: string; enabled: boolean; health: string; support: string; last_attempt_at: string | null; last_success_at: string | null; last_outcome: string | null; last_diagnostic: string | null; consecutive_failures: number }
export interface Fact { id: string; key: string; value: string; category: string; sensitive: boolean; approved: boolean; provenance: string; version: number }
export interface Job { id: string; canonical_title: string; organization: string; city?: string; country_code?: string; remote_raw?: string; apply_url: string; first_seen_at?: string; deadline_at?: string | null; last_seen_at: string; score: number; technical: number; profile_match: number; career_stage: string; career_priority: number; eligibility: "eligible" | "needs_review"; explanation: string; gaps: string[]; source_key: string; provider: string }
export interface Home {
  cv: { id: string; label: string; created_at: string } | null;
  cv_ready: boolean;
  profiles: { id: string; name: string }[];
  selected_profile: string;
  jobs: Job[];
  matches: number;
  discovered: number;
  applications: Application[];
  source_health: { enabled: number; failing: number; never_succeeded: number; stale: number; last_success_at: string | null };
  application_counts: { confirmed: number; reported: number; uncertain: number; failed: number; needs_review: number; queued: number; manual: number };
  questions: Question[];
  policies: { id: string; name: string; enabled: boolean; source_keys: string[]; profile_id: string; expires_at: string }[];
  last_updated: string | null;
  email_configured: boolean;
  submissions_paused: boolean;
  max_automatic_per_day: number;
  mail: { application_id: string; state: string; error?: string }[];
  total_ranked?: number;
  filter_options?: { stages: string[]; countries: string[]; sources: string[] };
  marks?: { opportunity_id: string; mark: "saved" | "review" }[];
  exclusions?: Rule[];
  previous_visit_at?: string | null;
  notification_preferences?: NotificationPreferences;
}

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
  home: () => request<Home>("/home"),
  select: (profile_id: string) => post("/home/profile", { profile_id }),
  authorize: (source_key: string, profile_id: string) => post("/home/authorize", { source_key, profile_id }),
  disable: (id: string) => post(`/policies/${id}/state`, { enabled: false, revoke: true }),
  apply: (id: string) => post(`/home/apply/${id}`),
  answer: (id: string, answer: string) => post(`/home/answer/${id}`, { answer }),
  pause: (paused: boolean) => post("/home/pause", { paused }),
  upload: (file: File) => request("/documents/cv?filename=" + encodeURIComponent(file.name), {
    method: "POST", headers: { "content-type": "application/octet-stream" }, body: file,
  }),
};
export interface Application { id: string; opportunity_id: string; status: string; reason?: string; title: string; employer: string; url: string }
export interface Question { id: string; application_id: string; question_text: string; field_type: string; options: string[] }
export interface Job { id: string; canonical_title: string; organization: string; city?: string; country_code?: string; remote_raw?: string; apply_url: string; last_seen_at: string; score: number; technical: number; profile_match: number; career_stage: string; career_priority: number; eligibility: "eligible" | "needs_review"; explanation: string; gaps: string[]; source_key: string; provider: string }
export interface Home {
  cv: { id: string; label: string; created_at: string } | null;
  cv_ready: boolean;
  profiles: { id: string; name: string }[];
  selected_profile: string;
  jobs: Job[];
  matches: number;
  discovered: number;
  applications: Application[];
  questions: Question[];
  policies: { id: string; name: string; enabled: boolean; source_keys: string[]; profile_id: string; expires_at: string }[];
  last_updated: string | null;
  email_configured: boolean;
  submissions_paused: boolean;
  max_automatic_per_day: number;
  mail: { application_id: string; state: string; error?: string }[];
}

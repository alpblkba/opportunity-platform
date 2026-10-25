import type {
  ApplicantFact,
  CvDocument,
  ApplicationEvent,
  ApplicationRecord,
  Attempt,
  AuthorizationPolicy,
  CoverageRow,
  NotificationRecord,
  ReviewTask,
  Digest,
  MatchPage,
  Profile,
  Rule,
  Vocabulary,
  IngestSummary,
  OpportunityDetail,
  OpportunityPage,
  Source,
  SourceRun,
  Stats,
  ValidationReport,
} from "./types";

/** What the gate said was missing, when it said anything. */
export type Needs = "native" | "identity";

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly needs?: Needs,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
  });
  if (!response.ok) {
    // The API returns a structured error body; surfacing its detail is what makes a bad board token a readable message instead of a silent empty list.
    let detail = response.statusText;
    let needs: Needs | undefined;
    try {
      const body = (await response.json()) as { detail?: string; needs?: Needs };
      if (body.detail) detail = body.detail;
      // Which credential is missing decides which screen to show: a password form, or a note that
      // the identity layer in front has expired and the page should be reloaded.
      needs = body.needs;
    } catch {
      // A non-JSON error body is not itself an error worth reporting over the status.
    }
    throw new ApiError(detail, response.status, needs);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export interface DiscoverFilters {
  q: string;
  country: string;
  city: string;
  kind: string;
  source: string;
  state: string;
  sort: string;
  includeClosed: boolean;
  page: number;
}

export const emptyFilters: DiscoverFilters = {
  q: "",
  country: "",
  city: "",
  kind: "",
  source: "",
  state: "",
  sort: "newest",
  includeClosed: false,
  page: 1,
};

export function discoverQueryString(filters: DiscoverFilters): string {
  const params = new URLSearchParams();
  if (filters.q.trim()) params.set("q", filters.q.trim());
  if (filters.country) params.set("country", filters.country);
  if (filters.city.trim()) params.set("city", filters.city.trim());
  if (filters.kind) params.set("type", filters.kind);
  if (filters.source) params.set("source", filters.source);
  if (filters.state) params.set("state", filters.state);
  if (filters.sort) params.set("sort", filters.sort);
  if (filters.includeClosed) params.set("include_closed", "true");
  params.set("page", String(filters.page));
  params.set("per_page", "25");
  return params.toString();
}

export interface Session {
  email: string;
  display_name: string;
  role: string;
  via: string;
}

export const api = {
  session: () => request<Session>("/api/auth/session"),
  login: (email: string, password: string) =>
    request<Session>("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  logout: () => request<void>("/api/auth/logout", { method: "POST" }),

  stats: () => request<Stats>("/api/stats"),
  coverage: () => request<CoverageRow[]>("/api/coverage"),
  opportunities: (filters: DiscoverFilters) =>
    request<OpportunityPage>(`/api/opportunities?${discoverQueryString(filters)}`),
  opportunity: (id: string) => request<OpportunityDetail>(`/api/opportunities/${id}`),
  sources: () => request<Source[]>("/api/sources"),
  createSource: (body: unknown) => request<Source>("/api/sources", { method: "POST", body: JSON.stringify(body) }),
  updateSource: (id: string, body: unknown) =>
    request<Source>(`/api/sources/${id}`, { method: "PATCH", body: JSON.stringify(body) }),
  deleteSource: (id: string) => request<void>(`/api/sources/${id}`, { method: "DELETE" }),
  validateSource: (id: string) => request<ValidationReport>(`/api/sources/${id}/validate`, { method: "POST" }),
  refreshSource: (id: string) => request<IngestSummary>(`/api/sources/${id}/refresh`, { method: "POST" }),
  refreshAll: () =>
    request<{ sources_attempted: number; summaries: IngestSummary[] }>("/api/refresh?all=true", { method: "POST" }),
  sourceRuns: () => request<SourceRun[]>("/api/source-runs?limit=50"),

  cvs: () => request<CvDocument[]>("/api/documents/cv"),
  uploadCv: (file: File) => request<CvDocument>(`/api/documents/cv?filename=${encodeURIComponent(file.name)}`, {
    method: "POST", headers: { "content-type": file.type || "application/octet-stream" }, body: file,
  }),
  updateTracking: (id: string, status: string, notes: string) =>
    request<ApplicationRecord>(`/api/applications/${id}/tracking`, { method: "POST", body: JSON.stringify({ status, notes }) }),

  profiles: () => request<Profile[]>("/api/profiles"),
  createProfile: (body: unknown) => request<Profile>("/api/profiles", { method: "POST", body: JSON.stringify(body) }),
  updateProfile: (id: string, body: unknown) =>
    request<Profile>(`/api/profiles/${id}`, { method: "PATCH", body: JSON.stringify(body) }),
  deleteProfile: (id: string) => request<void>(`/api/profiles/${id}`, { method: "DELETE" }),
  seedProfiles: () => request<{ created: string[] }>("/api/profiles/seed", { method: "POST" }),

  rules: () => request<Rule[]>("/api/rules"),
  createRule: (body: unknown) => request<Rule>("/api/rules", { method: "POST", body: JSON.stringify(body) }),
  deleteRule: (id: string) => request<void>(`/api/rules/${id}`, { method: "DELETE" }),

  matches: (profileId: string, q: string, includeExcluded: boolean) => {
    const params = new URLSearchParams({ profile_id: profileId, limit: "40" });
    if (q.trim()) params.set("q", q.trim());
    if (includeExcluded) params.set("include_excluded", "true");
    return request<MatchPage>(`/api/matches?${params.toString()}`);
  },
  feedback: (body: unknown) => request<unknown>("/api/feedback", { method: "POST", body: JSON.stringify(body) }),
  sampleUnseen: (profileId: string) =>
    request<{ id: string; canonical_title: string; organization: string }[]>(
      `/api/feedback/sample?profile_id=${profileId}&limit=5`,
    ),

  digests: () => request<Digest[]>("/api/digests"),
  generateDigest: (profileId: string, format: string) =>
    request<Digest>(`/api/digests/generate?profile_id=${profileId}&format=${format}`, { method: "POST" }),
  exportUrl: (profileId: string, format: string) => `/api/export?profile_id=${profileId}&format=${format}`,

  vocabulary: () => request<Vocabulary>("/api/vocabulary"),

  applications: () => request<ApplicationRecord[]>("/api/applications"),
  application: (id: string) => request<ApplicationRecord>(`/api/applications/${id}`),
  applicationEvents: (id: string) => request<ApplicationEvent[]>(`/api/applications/${id}/events`),
  applicationAttempts: (id: string) => request<Attempt[]>(`/api/applications/${id}/attempts`),
  createApplication: (opportunityId: string, profileId?: string) =>
    request<ApplicationRecord>("/api/applications", {
      method: "POST",
      body: JSON.stringify({ opportunity_id: opportunityId, profile_id: profileId ?? null }),
    }),
  prepare: (id: string, url: string) =>
    request<{ route: string; reason: string; mapped: number; open_questions: number }>(
      `/api/applications/${id}/prepare`,
      { method: "POST", body: JSON.stringify({ url }) },
    ),

  reviewTasks: (applicationId?: string) =>
    request<ReviewTask[]>(`/api/review-tasks${applicationId ? `?application_id=${applicationId}` : ""}`),
  answerReviewTask: (id: string, answer: string) =>
    request<ReviewTask>(`/api/review-tasks/${id}/answer`, { method: "POST", body: JSON.stringify({ answer }) }),

  facts: () => request<ApplicantFact[]>("/api/facts"),
  createFact: (body: unknown) => request<ApplicantFact>("/api/facts", { method: "POST", body: JSON.stringify(body) }),
  approveFact: (id: string, approved: boolean) =>
    request<ApplicantFact>(`/api/facts/${id}/approve`, { method: "POST", body: JSON.stringify({ approved }) }),

  policies: () => request<AuthorizationPolicy[]>("/api/policies"),
  createPolicy: (body: unknown) =>
    request<AuthorizationPolicy>("/api/policies", { method: "POST", body: JSON.stringify(body) }),
  setPolicyState: (id: string, enabled: boolean, revoke = false) =>
    request<AuthorizationPolicy>(`/api/policies/${id}/state`, {
      method: "POST",
      body: JSON.stringify({ enabled, revoke }),
    }),

  notifications: (undeliveredOnly = false) =>
    request<NotificationRecord[]>(`/api/notifications${undeliveredOnly ? "?undelivered_only=true" : ""}`),
  markDelivered: (id: string) => request<void>(`/api/notifications/${id}/delivered`, { method: "POST" }),
};

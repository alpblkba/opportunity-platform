import { useCallback, useEffect, useRef, useState } from "react";
import { api, ApiError, type Filters, type Home, type Question } from "./api";
import LandingPage from "./components/LandingPage";
import LoginPage from "./components/LoginPage";
import { ExclusionList, ExternalJobForm, FactsPanel, LocationPreferences, NotificationSettings, SourcesPanel } from "./components/Settings";

const labels: Record<string, string> = { saved: "Saved", preparing: "Checking form", ready: "Ready · authorization needed", needs_review: "Needs input", submitting: "Submitting", submitted: "Submitted", submission_unknown: "Unconfirmed · check with employer", failed: "Failed", unsupported: "Manual completion", interview: "Interview", rejected: "Rejected", withdrawn: "Withdrawn" };
const date = (value: string) => new Date(value).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
function QuestionForm({ question, onAnswer }: { question: Question; onAnswer: (answer: string) => Promise<void> }) {
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  return <form className="question" onSubmit={async e => { e.preventDefault(); setBusy(true); try { await onAnswer(answer); } finally { setBusy(false); } }}>
    <label htmlFor={question.id}>{question.question_text}</label>
    <div className="answer-row">{question.options.length > 0 ? <select required id={question.id} value={answer} onChange={e => setAnswer(e.target.value)}><option value="">Choose an answer</option>{question.options.map(o => <option key={o} value={o}>{o}</option>)}</select> : <input required id={question.id} value={answer} onChange={e => setAnswer(e.target.value)} autoComplete="off" />}
    <button disabled={busy || !answer.trim()}>{busy ? "Saving…" : "Save answer"}</button></div>
  </form>;
}

function TrackingForm({ application, onChanged }: { application: Home["applications"][number]; onChanged: () => Promise<void> }) {
  const statuses = ["saved", "submitted", "interview", "rejected", "withdrawn"];
  const [status, setStatus] = useState(statuses.includes(application.status) ? application.status : "");
  const [notes, setNotes] = useState(application.notes ?? "");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  useEffect(() => { setStatus(statuses.includes(application.status) ? application.status : ""); setNotes(application.notes ?? ""); }, [application.status, application.notes]);
  return <details className="manual-tracking"><summary>Update tracking</summary>
    <p className="muted small">Record what you did on the employer’s website. These controls send nothing to employers.</p>
    <form onSubmit={async event => {
      event.preventDefault(); setBusy(true); setMessage("");
      try { await api.track(application.id, status, notes); await onChanged(); setMessage("Tracking saved. Nothing was submitted."); }
      catch (e) { setMessage(e instanceof Error ? e.message : "Could not save tracking."); }
      finally { setBusy(false); }
    }}>
      <label>Recorded status<select value={status} onChange={e => setStatus(e.target.value)} required disabled={busy}>
        <option value="" disabled>Choose a status</option>
        {statuses.map(value => <option key={value} value={value}>{value === "submitted" ? "Applied manually on employer site" : value === "saved" ? "Saved for manual follow-up" : labels[value]}</option>)}
      </select></label>
      <label>Application notes<textarea value={notes} onChange={e => setNotes(e.target.value)} maxLength={20000} disabled={busy} /></label>
      <button disabled={busy || !status || ["preparing", "submitting"].includes(application.status)}>{busy ? "Saving…" : "Save tracking"}</button>
    </form>
    {message && <p role="status" className="small">{message}</p>}
  </details>;
}

type Route = "landing" | "login" | "workspace";
const routeOf = (path: string): Route => path === "/login" ? "login" : path === "/" || path === "/index.html" ? "landing" : "workspace";

export default function App() {
  const [route, setRoute] = useState<Route>(() => routeOf(window.location.pathname));
  // null when nobody is signed in; `via` says whether the identity came from a password or the Cloudflare layer.
  const [session, setSession] = useState<"checking" | null | { via: string }>("checking");
  const [expired, setExpired] = useState(false);
  const [upstream, setUpstream] = useState("");
  const [home, setHome] = useState<Home | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  const [notice, setNotice] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const generation = useRef(0);
  const [filters, setFilters] = useState<Filters>({});
  const [query, setQuery] = useState("");
  const filtersRef = useRef<Filters>({});
  // "New" is measured against the visit before this one, fixed for the whole session.
  const seenBefore = useRef<string | null | undefined>(undefined);
  const go = useCallback((path: string, replace = false) => {
    if (window.location.pathname !== path) window.history[replace ? "replaceState" : "pushState"]({}, "", path);
    setRoute(routeOf(path));
  }, []);
  useEffect(() => { const onPop = () => setRoute(routeOf(window.location.pathname)); window.addEventListener("popstate", onPop); return () => window.removeEventListener("popstate", onPop); }, []);
  // The server decides every time: the bundle is public, so nothing the browser believes about a session counts.
  const check = useCallback(async () => {
    try { setSession(await api.session()); setExpired(false); }
    catch (e) { setSession(null); if (e instanceof ApiError && e.status === 401) setUpstream(e.needs && e.needs !== "native" ? e.message : ""); else setError("The application is unavailable. Try again shortly."); }
  }, []);
  const lost = useCallback(() => { setSession(null); setHome(null); setExpired(true); }, []);
  const reload = useCallback(async () => {
    const current = ++generation.current;
    try {
      const data = await api.home(filtersRef.current);
      if (current !== generation.current) return;
      setHome(data);
      if (seenBefore.current === undefined) { seenBefore.current = data.previous_visit_at ?? null; void api.visited().catch(() => undefined); }
    }
    catch (e) { if (e instanceof ApiError && e.status === 401) lost(); else setError(e instanceof Error ? e.message : "Could not load opportunities."); }
  }, [lost]);
  useEffect(() => { void check(); }, [check]);
  useEffect(() => { const timer = window.setTimeout(() => setFilters(f => f.q === query ? f : { ...f, q: query }), 300); return () => clearTimeout(timer); }, [query]);
  // Only a filter change reloads here; the 15-second refresh reads the same ref.
  useEffect(() => { filtersRef.current = filters; if (home) void reload(); }, [filters]);
  const native = session !== "checking" && session?.via === "password";
  // A Cloudflare identity never had a front door at /, and a signed-in owner has nothing to do on /login.
  useEffect(() => { if (session && session !== "checking" && (route === "login" || (route === "landing" && !native))) go("/workspace", true); }, [session, native, route, go]);
  useEffect(() => { if (!session || session === "checking" || route !== "workspace") return; void reload(); const timer = window.setInterval(() => void reload(), 15000); return () => clearInterval(timer); }, [session, route, reload]);
  const act = async (key: string, action: () => Promise<unknown>, message = "") => {
    setBusy(key); setError(""); setNotice("");
    try { await action(); await reload(); setNotice(message); }
    catch (e) { if (e instanceof ApiError && e.status === 401) lost(); setError(e instanceof Error ? e.message : "Please try again."); }
    finally { setBusy(""); }
  };
  // The public page asks the server for nothing private and renders before the session check finishes.
  if (route === "landing" && (session === "checking" || !session || native)) return <LandingPage signedIn={native} onLogin={() => go(native ? "/workspace" : "/login")} />;
  if (session === "checking") return <main className="loading" role="status">Opening Opportunities…</main>;
  if (!session && upstream) return <main className="login" id="main"><div className="login-card"><h1>Alp&apos;s Opportunity Platform</h1><p className="notice error" role="alert">{upstream}</p><p className="login-sub">This deployment checks identity before the application. Reload to go through it again.</p><button type="button" className="primary" onClick={() => window.location.reload()}>Reload</button><p className="login-foot"><a href="https://alpblkba.dev/">← back to alpblkba.dev</a></p></div></main>;
  if (!session) return <><LoginPage expired={expired} onSignedIn={() => void check()} />{error && <p className="login-error" role="alert">{error}</p>}</>;
  if (!home) return <main className="loading"><p role="status">Loading your opportunities…</p>{error && <p role="alert">{error}</p>}</main>;
  // Every application, not the recent page; an uncertain or failed attempt is not an application sent.
  const counts = home.application_counts;
  const applied = counts.confirmed + counts.reported;
  const needing = new Set(home.questions.map(q => q.application_id)).size;
  const activePolicies = home.policies.filter(p => p.enabled && new Date(p.expires_at) > new Date());
  const profileName = home.profiles.find(p => p.id === home.selected_profile)?.name;
  const savedIds = new Set((home.marks ?? []).filter(m => m.mark === "saved").map(m => m.opportunity_id));
  const options = home.filter_options ?? { stages: [], countries: [], sources: [] };
  const filtered = Object.values(filters).some(v => v !== undefined && v !== "" && v !== false);
  const setFilter = (key: keyof Filters, value: string | boolean) => setFilters(f => ({ ...f, [key]: value || undefined }));
  return <main className="shell" id="main">
    <header className="topbar"><span className="brand"><span className="brand-mark">O</span> Opportunity Platform</span><button className="quiet" onClick={() => void api.logout().catch(() => undefined).then(() => { setSession(null); setHome(null); setExpired(false); go("/"); })}>Sign out</button></header>
    <section className="workspace" aria-labelledby="heading">
      <div className="heading-row"><div><h1 id="heading">Opportunities</h1><p className="muted">Your job search, automatically.</p></div><span className={`badge ${home.cv_ready ? "success" : "neutral"}`}>{home.cv_ready ? "CV uploaded" : "Add your CV"}</span></div>
      {error && <p className="message error" role="alert">{error}</p>}{notice && <p className="message" role="status">{notice}</p>}
      <div className="cv-row"><div>{home.cv ? <><a className="cv-name" href={`/api/documents/${home.cv.id}/download`}>{home.cv.label}</a><p className="muted small">Stored privately · Updated {date(home.cv.created_at)}</p></> : <><strong>Your CV starts the search</strong><p className="muted small">PDF or text · Upload once, update anytime</p></>}</div>
        <input ref={input} className="file-input" type="file" accept=".pdf,.txt" aria-label="Upload CV" onChange={e => { const file = e.target.files?.[0]; if (file) void act("cv", () => api.upload(file), "CV updated. Your matches now use this version."); e.target.value = ""; }} />
        <button className={home.cv ? "" : "primary"} disabled={!!busy} onClick={() => input.current?.click()}>{busy === "cv" ? "Reading CV…" : home.cv ? "Update CV" : "Upload CV"}</button>
      </div>
      <label className="muted profile-label" htmlFor="profile">Searching for</label>
      <select id="profile" value={home.selected_profile} disabled={!!busy} onChange={e => void act("profile", () => api.select(e.target.value))}>{home.profiles.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select>
      <div className="metrics" aria-label="Application overview"><div><strong>{home.cv_ready ? home.matches : "—"}</strong><span>Matches</span></div><div><strong>{applied}</strong><span>Applied</span></div>{counts.uncertain > 0 && <div><strong>{counts.uncertain}</strong><span>Outcome unknown</span></div>}<div><strong>{needing}</strong><span>Needs input</span></div></div>
      {(activePolicies.length > 0 || home.submissions_paused) && <div className="authorizations"><div><span><span className="dot" />{home.submissions_paused ? "All submissions paused" : "Automatic applications on"}<small>{home.submissions_paused ? "Nothing is prepared or sent until you resume." : `At most ${home.max_automatic_per_day} automatic applications per day across all employers.`}</small></span><button disabled={!!busy} onClick={() => void act("pause", () => api.pause(!home.submissions_paused), home.submissions_paused ? "Submissions resumed." : "All submissions paused.")}>{home.submissions_paused ? "Resume" : "Pause all"}</button></div></div>}
      {activePolicies.length > 0 && <div className="authorizations">{activePolicies.map(p => <div key={p.id}><span><span className="dot" />Auto-apply: {p.name}<small>{home.profiles.find(x => x.id === p.profile_id)?.name} · Until {date(p.expires_at)}</small></span><button disabled={!!busy} onClick={() => void act(p.id, () => api.disable(p.id))}>Stop</button></div>)}</div>}
      {home.questions.length > 0 && <section className="questions"><h2>Needs your input <span className="count">{home.questions.length}</span></h2>{home.questions.map(q => { const app = home.applications.find(a => a.id === q.application_id); return <div key={q.id} className="question-card"><p className="muted small">{app?.employer} · {app?.title}</p>{q.field_type === "unsupported" ? <p className="small">{q.question_text} · <a href={app?.url} target="_blank" rel="noreferrer">Complete on employer website ↗</a></p> : <QuestionForm question={q} onAnswer={answer => act(q.id, () => api.answer(q.id, answer), "Answer saved. The application will resume automatically.")} />}</div>; })}</section>}
      <section aria-labelledby="ranked-heading"><div className="section-heading"><h2 id="ranked-heading">Ranked opportunities</h2><span className="muted small">{home.last_updated ? `Updated ${date(home.last_updated)}` : "Finding current opportunities…"}</span></div>{home.source_health.failing > 0 && <p className="notice" role="status">{home.source_health.failing} of {home.source_health.enabled} employer sources keep failing to refresh; their listings are kept but may be out of date.</p>}
        {home.cv_ready && <div className="filters" role="search" aria-label="Filter opportunities">
          <input type="search" aria-label="Search opportunities" placeholder="Search title or employer" value={query} onChange={e => setQuery(e.target.value)} />
          <select aria-label="Career stage" value={filters.stage ?? ""} onChange={e => setFilter("stage", e.target.value)}><option value="">Any stage</option>{options.stages.map(v => <option key={v} value={v}>{v}</option>)}</select>
          <select aria-label="Country" value={filters.country ?? ""} onChange={e => setFilter("country", e.target.value)}><option value="">Any country</option>{options.countries.map(v => <option key={v} value={v}>{v}</option>)}</select>
          <select aria-label="Eligibility" value={filters.eligibility ?? ""} onChange={e => setFilter("eligibility", e.target.value)}><option value="">Any eligibility</option><option value="eligible">Eligible</option><option value="needs_review">Needs review</option></select>
          <label className="check"><input type="checkbox" checked={!!filters.saved} onChange={e => setFilter("saved", e.target.checked)} />Saved only</label>
          {filtered && <span className="muted small" role="status">Showing {home.matches} of {home.total_ranked ?? home.matches}</span>}
        </div>}
        {!home.cv_ready ? <div className="empty"><span className="empty-icon">↥</span><h3>{home.cv ? "Update your CV to calculate matches" : "Your next opportunity starts here"}</h3><p>Upload your CV to see opportunities for this profile.</p></div> : home.jobs.length === 0 ? <div className="empty"><h3>{home.discovered ? "No strong matches for this profile yet" : "Searching for opportunities"}</h3><p>{home.discovered ? "Try another career profile. New opportunities are checked automatically." : "Current employer listings will appear here automatically."}</p></div> : <div className="jobs">{home.jobs.map((job, i) => {
          const app = home.applications.find(a => a.opportunity_id === job.id);
          const policy = activePolicies.find(p => p.profile_id === home.selected_profile && p.source_keys.includes(job.source_key));
          return <article className="job" key={job.id}><div className="rank">{String(i + 1).padStart(2, "0")}</div><div className="job-body"><div className="job-top"><span className="employer">{job.organization}{seenBefore.current && job.first_seen_at && job.first_seen_at > seenBefore.current && <span className="badge new">New</span>}</span><span className={`badge ${job.eligibility !== "needs_review" && job.score >= 65 ? "success" : "neutral"}`}>{job.eligibility === "needs_review" ? "Needs review" : job.score >= 65 ? "Priority match" : "Potential match"}</span></div><h3><a href={job.apply_url} target="_blank" rel="noreferrer">{job.canonical_title} <span aria-hidden="true">↗</span></a></h3><p className="muted small">{[job.city, job.country_code, job.remote_raw].filter(Boolean).join(" · ") || "Location not specified"} · {job.provider} · Seen {date(job.last_seen_at)}{job.deadline_at && <> · <strong>Deadline {date(job.deadline_at)}</strong></>}</p><p className="match-note">{job.explanation}</p>{job.technical !== undefined && <p className="muted small">Priority {Math.round(job.score)} · technical fit {Math.round(job.technical)} · career stage {Math.round(job.career_priority)} · profile {Math.round(job.profile_match)}</p>}{job.gaps.length > 0 && <p className="gaps">{job.gaps.join(" · ")}</p>}
            <div className="job-actions">{app ? <span className={`status ${app.status === "submitted" ? "success-text" : ""}`}>{labels[app.status] || app.status}</span> : <button disabled={!!busy} onClick={() => void act(job.id, () => api.apply(job.id), "Application queued. Any required questions will appear here.")}>Prepare application</button>}<button className="quiet" aria-pressed={savedIds.has(job.id)} disabled={!!busy} onClick={() => void act(`save-${job.id}`, () => api.mark(job.id, "saved", !savedIds.has(job.id)), savedIds.has(job.id) ? "Removed from saved." : "Saved. Deadline reminders cover saved opportunities.")}>{savedIds.has(job.id) ? "Saved ✓" : "Save"}</button><button className="quiet" disabled={!!busy} onClick={() => void act(`dismiss-${job.id}`, () => api.dismiss(job.id), "Dismissed. Undo it under Dismissed and hidden.")}>Dismiss</button><a className="source-link" href={job.apply_url} target="_blank" rel="noreferrer">Employer page ↗</a></div>
            <details className="more"><summary>More</summary><button className="quiet" disabled={!!busy} onClick={() => void act(`block-${job.id}`, () => api.blockEmployer(job.organization), `${job.organization} is hidden from every profile. Undo it under Dismissed and hidden.`)}>Hide all from {job.organization}</button></details>
            {!policy && ["greenhouse", "lever", "ashby"].includes(job.provider) && app?.status !== "unsupported" && <details className="auto-policy"><summary>Enable automatic applications at {job.organization}</summary><p>Apply to strong matches for <strong>{profileName}</strong> at {job.organization}, using your current CV and your answers. Up to 3 per day, 30 total, for 30 days. Required questions pause the application.</p><button disabled={!!busy} onClick={() => void act(`authorize-${job.id}`, () => api.authorize(job.source_key, home.selected_profile), `Automatic applications enabled at ${job.organization}.`)}>Authorize {job.organization}</button></details>}
          </div></article>;
        })}</div>}
      </section>
      <ExternalJobForm onTracked={reload} />
      {home.applications.length > 0 && <section className="application-list"><h2>Applications</h2>{home.applications.map(app => { const mail = home.mail.find(n => n.application_id === app.id); return <div className="application" key={app.id}><div className="application-info"><strong>{app.title}</strong><p className="muted small">{app.employer}</p>{app.status === "unsupported" && <ol className="checklist small" aria-label={`Checklist for ${app.title}`}><li><a href={app.url} target="_blank" rel="noreferrer">Open the job ↗</a></li><li>Use your CV {app.cv_label ? `“${app.cv_label}”` : "(none recorded yet)"}{app.cv_label && home.cv && app.cv_label !== home.cv.label ? `; a newer CV, “${home.cv.label}”, is stored` : ""}</li><li>Copy approved details from “About you” in Settings</li><li>Apply there, then record it below as applied</li></ol>}<TrackingForm application={app} onChanged={reload} />{app.reason && ["failed", "unsupported", "ready"].includes(app.status) && <p className="small muted">{app.reason}</p>}</div><div className="application-state"><span className={app.status === "submitted" ? "success-text" : ""}>{labels[app.status] || app.status}</span>{mail && <small className="muted">{mail.state === "delivered" ? "Email delivered" : mail.state === "accepted" ? "Email accepted · awaiting delivery" : mail.state === "suppressed" ? "Not emailed: from before email delivery" : mail.state === "seen" ? "Seen here; not emailed" : ["bounced", "failed"].includes(mail.state) ? "Email delivery failed" : home.email_configured ? "Email queued" : "Email delivery not configured"}</small>}{["unsupported", "submission_unknown"].includes(app.status) && <a href={app.url} target="_blank" rel="noreferrer">Open application ↗</a>}{app.status === "failed" && <button onClick={() => void act(app.id, () => api.apply(app.opportunity_id))}>Retry</button>}</div></div>; })}</section>}
      <section className="settings" aria-labelledby="settings-heading"><h2 id="settings-heading">Settings</h2>
        <LocationPreferences key={home.selected_profile} profileId={home.selected_profile} profileName={profileName} onSaved={reload} />
        {home.notification_preferences && <NotificationSettings initial={home.notification_preferences} onSaved={reload} />}
        <FactsPanel />
        <SourcesPanel />
        <ExclusionList rules={home.exclusions ?? []} onChanged={reload} />
      </section>
      <footer className="workspace-footer"><span className="dot" />{home.discovered.toLocaleString()} current opportunities · Refreshes automatically{!home.email_configured && <span className="mail-status">Email notifications not configured</span>}</footer>
    </section>
  </main>;
}

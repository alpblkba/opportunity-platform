import { useState } from "react";
import { api, type Fact, type NotificationPreferences, type Rule, type Source } from "../api";

const when = (value: string | null) => value ? new Date(value).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "never";
const list = (value: string) => value.split(",").map(v => v.trim()).filter(Boolean);
const failure = (e: unknown, fallback: string) => e instanceof Error && e.message ? e.message : fallback;

/** Run one save; the message says what the server confirmed, never what was merely attempted. */
function useSave() {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const run = async (action: () => Promise<unknown>, done: string) => {
    setBusy(true); setMessage("");
    try { await action(); setMessage(done); } catch (e) { setMessage(failure(e, "Could not save. Please try again.")); }
    finally { setBusy(false); }
  };
  return { busy, message, run };
}

export function ExclusionList({ rules, onChanged }: { rules: Rule[]; onChanged: () => Promise<void> }) {
  const { busy, message, run } = useSave();
  const [term, setTerm] = useState("");
  const [where, setWhere] = useState<"title_term" | "description_term">("title_term");
  const label = (r: Rule) => r.kind === "hidden" ? `Dismissed: ${r.title ?? "an opportunity"}` : r.kind === "company" ? `Employer hidden: ${r.value}` : `${r.kind === "title_term" ? "Title" : "Description"} excludes “${r.value}”`;
  return <details className="panel"><summary>Dismissed and hidden <span className="count">{rules.length}</span></summary>
    {rules.length === 0 && <p className="muted small">Nothing is dismissed or hidden. Dismissing a match, hiding an employer or a word puts it here, where it can be undone.</p>}
    <ul className="plain-list">{rules.map(r => <li key={r.id}><span>{label(r)}</span><button disabled={busy} onClick={() => void run(async () => { await api.unrule(r.id); await onChanged(); }, "Restored. It will appear in your matches again if it still fits.")}>Undo</button></li>)}</ul>
    <form className="settings-form" onSubmit={e => { e.preventDefault(); const value = term.trim(); void run(async () => { await api.addRule(where, value); setTerm(""); await onChanged(); }, `Postings with “${value}” in the ${where === "title_term" ? "title" : "description"} are hidden from every profile.`); }}>
      <label>Hide postings containing a word<input value={term} onChange={e => setTerm(e.target.value)} maxLength={200} placeholder="for example: sales, senior manager" autoComplete="off" /></label>
      <label>Look for it in<select value={where} onChange={e => setWhere(e.target.value as typeof where)}><option value="title_term">the title</option><option value="description_term">the description</option></select></label>
      <button disabled={busy || !term.trim()}>Hide them</button>
    </form>
    {message && <p role="status" className="small">{message}</p>}
  </details>;
}

export function NotificationSettings({ initial, onSaved }: { initial: NotificationPreferences; onSaved: () => Promise<void> }) {
  const [p, setP] = useState(initial);
  const [days, setDays] = useState(initial.reminder_days.join(", "));
  const { busy, message, run } = useSave();
  return <details className="panel"><summary>Email alerts</summary>
    <form className="settings-form" onSubmit={e => { e.preventDefault(); void run(async () => { const saved = await api.saveNotifications({ ...p, reminder_days: list(days).map(Number) }); setP(saved); setDays(saved.reminder_days.join(", ")); await onSaved(); }, "Alert preferences saved."); }}>
      <label className="check"><input type="checkbox" checked={p.new_match_digest} onChange={e => setP({ ...p, new_match_digest: e.target.checked })} />Daily digest of new matches</label>
      <label>Only matches with priority of at least<input type="number" min={0} max={100} value={p.min_priority} onChange={e => setP({ ...p, min_priority: Number(e.target.value) })} /></label>
      <label>Send the digest after (hour, 0–23)<input type="number" min={0} max={23} value={p.digest_hour} onChange={e => setP({ ...p, digest_hour: Number(e.target.value) })} /></label>
      <label>Time zone<input value={p.time_zone} onChange={e => setP({ ...p, time_zone: e.target.value })} autoComplete="off" /></label>
      <label className="check"><input type="checkbox" checked={p.deadline_reminders} onChange={e => setP({ ...p, deadline_reminders: e.target.checked })} />Remind me before deadlines of saved opportunities</label>
      <label>Days before the deadline (comma separated)<input value={days} onChange={e => setDays(e.target.value)} inputMode="numeric" autoComplete="off" /></label>
      <p className="muted small">Alerts go to the address configured on the server. Applications that need you are always emailed.</p>
      <button disabled={busy}>{busy ? "Saving…" : "Save alerts"}</button>
    </form>
    {message && <p role="status" className="small">{message}</p>}
  </details>;
}

const remotePolicies: [string, string][] = [["any", "On-site, hybrid or remote"], ["remote_from_current_country_only", "Remote from my current country"], ["on_site_or_relocate", "On-site, or I relocate"]];

export function LocationPreferences({ profileId, profileName, onSaved }: { profileId: string; profileName?: string; onSaved: () => Promise<void> }) {
  const [profile, setProfile] = useState<Record<string, unknown> | null>(null);
  const [countries, setCountries] = useState("");
  const [cities, setCities] = useState("");
  const { busy, message, run } = useSave();
  const load = async () => {
    const loaded = await api.profile(profileId);
    setProfile(loaded); setCountries(((loaded.preferred_countries as string[]) ?? []).join(", ")); setCities(((loaded.preferred_cities as string[]) ?? []).join(", "));
  };
  return <details className="panel" onToggle={e => { if ((e.target as HTMLDetailsElement).open) void load().catch(() => undefined); }}><summary>Location preferences · {profileName}</summary>
    {!profile ? <p className="muted small" role="status">Loading…</p> : <form className="settings-form" onSubmit={e => { e.preventDefault(); void run(async () => {
      const { id: _id, created_at: _c, updated_at: _u, ...rest } = profile;
      await api.saveProfile(profileId, { ...rest, preferred_countries: list(countries).map(c => c.toUpperCase()), preferred_cities: list(cities) });
      await load(); await onSaved();
    }, "Location preferences saved. Matches are re-ranked with them."); }}>
      <label>Preferred countries (codes, comma separated; empty means anywhere)<input value={countries} onChange={e => setCountries(e.target.value)} placeholder="DE, NL, CH" autoComplete="off" /></label>
      <label>Preferred cities (comma separated)<input value={cities} onChange={e => setCities(e.target.value)} autoComplete="off" /></label>
      <label>Work arrangement<select value={String(profile.remote_policy ?? "any")} onChange={e => setProfile({ ...profile, remote_policy: e.target.value })}>{remotePolicies.map(([value, text]) => <option key={value} value={value}>{text}</option>)}</select></label>
      <label className="check"><input type="checkbox" checked={profile.include_unknown_location !== false} onChange={e => setProfile({ ...profile, include_unknown_location: e.target.checked })} />Keep postings whose location is not stated</label>
      <button disabled={busy}>{busy ? "Saving…" : "Save preferences"}</button>
    </form>}
    {message && <p role="status" className="small">{message}</p>}
  </details>;
}

const health: Record<string, string> = { healthy: "Working", degraded: "Partial or failing", blocked: "Blocked", schema_error: "Format changed", unknown: "Not yet checked" };

export function SourcesPanel() {
  const [sources, setSources] = useState<Source[] | null>(null);
  const { busy, message, run } = useSave();
  const load = async () => setSources(await api.sources());
  return <details className="panel" onToggle={e => { if ((e.target as HTMLDetailsElement).open) void load().catch(() => setSources([])); }}><summary>Sources and coverage</summary>
    {!sources ? <p className="muted small" role="status">Loading…</p> : <ul className="plain-list sources">{sources.map(s => {
      const failing = s.enabled && s.consecutive_failures >= 3;
      return <li key={s.id}><div><strong>{s.display_name}</strong><p className="muted small">{s.kind} · {s.enabled ? health[s.health] ?? s.health : "Off"} · last success {when(s.last_success_at)} · last try {when(s.last_attempt_at)}{s.last_outcome === "partial" ? " · partial coverage" : ""}</p>{failing && s.last_diagnostic && <p className="gaps">{s.last_diagnostic}</p>}</div>
        <div className="row-actions"><button disabled={busy} onClick={() => void run(async () => { await api.setSourceEnabled(s.id, !s.enabled); await load(); }, s.enabled ? `${s.display_name} turned off. Its listings are kept.` : `${s.display_name} turned on.`)}>{s.enabled ? "Turn off" : "Turn on"}</button>{s.enabled && <button disabled={busy} onClick={() => void run(async () => { await api.refreshSource(s.id); await load(); }, `${s.display_name} refreshed.`)}>Refresh</button>}</div></li>;
    })}</ul>}
    {message && <p role="status" className="small">{message}</p>}
  </details>;
}

export function FactsPanel() {
  const [facts, setFacts] = useState<Fact[] | null>(null);
  const [draft, setDraft] = useState({ key: "", value: "", category: "identity" });
  const { busy, message, run } = useSave();
  const load = async () => setFacts(await api.facts());
  return <details className="panel" onToggle={e => { if ((e.target as HTMLDetailsElement).open) void load().catch(() => setFacts([])); }}><summary>About you (facts used in applications)</summary>
    <p className="muted small">Only approved facts are ever put into an application form. Nothing here is guessed: unknown stays unknown.</p>
    {!facts ? <p className="muted small" role="status">Loading…</p> : <ul className="plain-list">{facts.map(f => <li key={f.id}><div><strong>{f.key}</strong><p className="small">{f.sensitive ? "•••• (sensitive)" : f.value}</p><p className="muted small">{f.category} · {f.provenance} · {f.approved ? "approved" : "not approved"}</p></div>
      <button disabled={busy} onClick={() => void run(async () => { await api.approveFact(f.id, !f.approved); await load(); }, f.approved ? `${f.key} will no longer be used.` : `${f.key} approved for applications.`)}>{f.approved ? "Withdraw" : "Approve"}</button></li>)}</ul>}
    <form className="settings-form" onSubmit={e => { e.preventDefault(); void run(async () => { await api.saveFact(draft.key.trim(), draft.value.trim(), draft.category); setDraft({ ...draft, key: "", value: "" }); await load(); }, "Saved. Approve it before it is used in an application."); }}>
      <label>Fact (for example: LinkedIn, expected graduation)<input value={draft.key} onChange={e => setDraft({ ...draft, key: e.target.value })} required maxLength={200} autoComplete="off" /></label>
      <label>Value<input value={draft.value} onChange={e => setDraft({ ...draft, value: e.target.value })} required maxLength={2000} autoComplete="off" /></label>
      <label>Kind<select value={draft.category} onChange={e => setDraft({ ...draft, category: e.target.value })}>{["identity", "contact", "education", "experience", "skill", "preference", "authorization", "other"].map(c => <option key={c} value={c}>{c}</option>)}</select></label>
      <button disabled={busy || !draft.key.trim() || !draft.value.trim()}>{busy ? "Saving…" : "Add or correct fact"}</button>
    </form>
    {message && <p role="status" className="small">{message}</p>}
  </details>;
}

/** A job found where this platform does not read, such as LinkedIn: the link is stored, never opened by the server. */
export function ExternalJobForm({ onTracked }: { onTracked: () => Promise<void> }) {
  const empty = { title: "", employer: "", url: "", deadline: "" };
  const [draft, setDraft] = useState(empty);
  const { busy, message, run } = useSave();
  const [done, setDone] = useState("");
  return <details className="panel"><summary>Track a job from LinkedIn or elsewhere</summary>
    <p className="muted small">Nothing is sent from here and the link is not visited for you: you apply on that site and record it under Applications. The same link twice is tracked once.</p>
    <form className="settings-form" onSubmit={e => { e.preventDefault(); setDone(""); void run(async () => {
      const result = await api.trackExternal({ title: draft.title.trim(), employer: draft.employer.trim(), url: draft.url.trim(), deadline_at: draft.deadline ? new Date(`${draft.deadline}T23:59:00`).toISOString() : null });
      setDone(result.duplicate ? "Already tracked; nothing was duplicated." : "Tracked. Follow its checklist under Applications.");
      setDraft(empty); await onTracked();
    }, ""); }}>
      <label>Job title<input value={draft.title} onChange={e => setDraft({ ...draft, title: e.target.value })} required maxLength={300} autoComplete="off" /></label>
      <label>Employer<input value={draft.employer} onChange={e => setDraft({ ...draft, employer: e.target.value })} required maxLength={200} autoComplete="off" /></label>
      <label>Link to the job<input type="url" value={draft.url} onChange={e => setDraft({ ...draft, url: e.target.value })} required placeholder="https://www.linkedin.com/jobs/view/…" autoComplete="off" /></label>
      <label>Deadline (optional)<input type="date" value={draft.deadline} onChange={e => setDraft({ ...draft, deadline: e.target.value })} /></label>
      <button disabled={busy}>{busy ? "Saving…" : "Track it"}</button>
    </form>
    {(message || done) && <p role="status" className="small">{message || done}</p>}
  </details>;
}

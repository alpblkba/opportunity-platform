import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import ApplicationTracking from "./ApplicationTracking";
import { formatDate, relativeAge } from "../format";
import type { ApplicantFact, ApplicationEvent, ApplicationRecord, Attempt, AuthorizationPolicy, ReviewTask } from "../types";

/** What each application status means in a sentence, so a status word never has to be guessed at on a small screen. */
const STATUS_MEANING: Record<string, string> = {
  saved: "Saved. Nothing has been prepared yet.",
  preparing: "Reading the form now.",
  needs_review: "Waiting on you: some questions need answers.",
  ready: "Every field maps to something you approved. Not submitted.",
  submitting: "Being submitted right now.",
  submitted: "Recorded as applied. History distinguishes manual reports from employer receipts.",
  submission_unknown: "Submitted but unconfirmed. Nobody knows whether it arrived.",
  interview: "You have an interview.",
  rejected: "Rejected.",
  withdrawn: "You withdrew it.",
  failed: "The attempt failed. Nothing was sent, so it is safe to try again.",
};

const STATUS_TONE: Record<string, string> = {
  submitted: "ok",
  needs_review: "warn",
  submission_unknown: "bad",
  failed: "bad",
};

type Panel = "applications" | "actions" | "facts" | "policies" | "inbox";

const PANELS: { id: Panel; label: string }[] = [
  { id: "actions", label: "Actions" },
  { id: "applications", label: "Applications" },
  { id: "facts", label: "Facts" },
  { id: "policies", label: "Policies" },
  { id: "inbox", label: "Inbox" },
];

export default function WorkspacePage() {
  const [panel, setPanel] = useState<Panel>("actions");
  const [openCount, setOpenCount] = useState(0);

  const refreshBadge = useCallback(() => {
    api.reviewTasks().then((tasks) => setOpenCount(tasks.length)).catch(() => setOpenCount(0));
  }, []);
  useEffect(refreshBadge, [refreshBadge]);

  return (
    <section aria-labelledby="workspace-heading">
      <h2 id="workspace-heading">Workspace</h2>
      <nav className="tabs subtabs" aria-label="Workspace sections">
        {PANELS.map((entry) => (
          <button
            key={entry.id}
            type="button"
            aria-current={panel === entry.id ? "page" : undefined}
            onClick={() => setPanel(entry.id)}
          >
            {entry.label}
            {entry.id === "actions" && openCount > 0 && <span className="count">{openCount}</span>}
          </button>
        ))}
      </nav>

      {panel === "actions" && <ActionsPanel onChanged={refreshBadge} />}
      {panel === "applications" && <ApplicationsPanel />}
      {panel === "facts" && <FactsPanel />}
      {panel === "policies" && <PoliciesPanel />}
      {panel === "inbox" && <InboxPanel />}
    </section>
  );
}

function ActionsPanel({ onChanged }: { onChanged: () => void }) {
  const [tasks, setTasks] = useState<ReviewTask[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(() => {
    setLoading(true);
    api
      .reviewTasks()
      .then((next) => {
        setTasks(next);
        setError(null);
      })
      .catch((reason: Error) => setError(reason.message))
      .finally(() => setLoading(false));
  }, []);

  useEffect(load, [load]);

  return (
    <div>
      <p className="notice">
        Questions an application form asked that could not be answered from facts you approved. Nothing here was
        guessed. Answering a question saves the answer; it does not submit anything.
      </p>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {loading && <p className="notice">Loading.</p>}
      {!loading && tasks.length === 0 && <p className="notice empty">Nothing needs your attention.</p>}
      <ul className="cards">
        {tasks.map((task) => (
          <ActionCard
            key={task.id}
            task={task}
            onAnswered={() => {
              load();
              onChanged();
            }}
          />
        ))}
      </ul>
    </div>
  );
}

function ActionCard({ task, onAnswered }: { task: ReviewTask; onAnswered: () => void }) {
  const [answer, setAnswer] = useState(task.draft_answer ?? "");
  const [saving, setSaving] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const save = async () => {
    setSaving(true);
    setNote(null);
    try {
      await api.answerReviewTask(task.id, answer);
      // Deliberately explicit: a person on a phone must not come away thinking this sent an application.
      setNote("Answer saved. This application has not been submitted.");
      onAnswered();
    } catch (reason) {
      setNote((reason as Error).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <li className="card">
      <h3>{task.question_text}</h3>
      <p className="badges">
        <span className="badge">{task.field_type}</span>
        {task.required && <span className="badge warn">Required</span>}
        {task.sensitive && <span className="badge bad">Sensitive, confirm it yourself</span>}
        <span className="badge">Asked {relativeAge(task.created_at)}</span>
      </p>
      {task.sensitive && (
        <p className="meta">
          This is the kind of question that must come from you. A value you approved earlier may be offered as a
          starting point, but it is never filled in automatically.
        </p>
      )}
      <label>
        Your answer
        {task.field_type === "textarea" ? (
          <textarea rows={4} value={answer} onChange={(event) => setAnswer(event.target.value)} />
        ) : (
          <input value={answer} onChange={(event) => setAnswer(event.target.value)} />
        )}
      </label>
      <div className="actions">
        <button type="button" className="primary" disabled={saving || !answer.trim()} onClick={save}>
          {saving ? "Saving" : "Save answer"}
        </button>
      </div>
      {note && (
        <p className="notice" role="status">
          {note}
        </p>
      )}
    </li>
  );
}

function ApplicationsPanel() {
  const [applications, setApplications] = useState<ApplicationRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  useEffect(() => {
    api
      .applications()
      .then(setApplications)
      .catch((reason: Error) => setError(reason.message));
  }, []);

  return (
    <div>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {applications.length === 0 && !error && (
        <p className="notice empty">No applications yet. Start one from a match in the Matches tab.</p>
      )}
      <ul className="cards">
        {applications.map((application) => (
          <li className="card" key={application.id}>
            <ApplicationTracking application={application} onSaved={(updated) => {
              setApplications((current) => current.map((item) => item.id === updated.id ? updated : item));
              setExpanded(null);
            }} />
            <h3>
              <span className={`badge ${STATUS_TONE[application.status] ?? ""}`}>
                {application.status.replace(/_/g, " ")}
              </span>
            </h3>
            <p className="meta">{STATUS_MEANING[application.status] ?? "Status unknown."}</p>
            <p className="badges">
              {application.route && <span className="badge">Route: {application.route}</span>}
              <span className="badge">Updated {relativeAge(application.updated_at)}</span>
            </p>
            {application.route_reason && <p className="meta">Why: {application.route_reason}</p>}
            <div className="actions">
              <button
                type="button"
                onClick={() => setExpanded(expanded === application.id ? null : application.id)}
              >
                {expanded === application.id ? "Hide history" : "History and receipts"}
              </button>
            </div>
            {expanded === application.id && <ApplicationDetail id={application.id} />}
          </li>
        ))}
      </ul>
    </div>
  );
}

function ApplicationDetail({ id }: { id: string }) {
  const [events, setEvents] = useState<ApplicationEvent[]>([]);
  const [attempts, setAttempts] = useState<Attempt[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.applicationEvents(id), api.applicationAttempts(id)])
      .then(([nextEvents, nextAttempts]) => {
        setEvents(nextEvents);
        setAttempts(nextAttempts);
      })
      .catch((reason: Error) => setError(reason.message));
  }, [id]);

  return (
    <div className="detail-block">
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {attempts.length > 0 && (
        <>
          <h4>Submission attempts</h4>
          {attempts.map((attempt) => (
            <div key={attempt.id} className="notice">
              <p style={{ margin: 0 }}>
                <strong>{attempt.state.replace(/_/g, " ")}</strong> via{" "}
                {attempt.authorization_kind === "policy" ? "a standing policy" : "a one-use authorization"}
              </p>
              {attempt.receipt && <p style={{ margin: 0 }}>Receipt: {attempt.receipt}</p>}
              {attempt.state === "unknown" && (
                <p style={{ margin: 0 }}>
                  This one is unconfirmed. Open the employer's page yourself to check before anything is tried again.
                </p>
              )}
              {attempt.diagnostic && <p className="meta">{attempt.diagnostic}</p>}
              <p className="meta">
                <a href={attempt.destination} target="_blank" rel="noopener noreferrer">
                  Open the application page
                </a>
              </p>
            </div>
          ))}
        </>
      )}
      <h4>History</h4>
      <ol className="timeline">
        {events.map((event) => (
          <li key={event.id}>
            <span className="badge">{event.kind.replace(/[:_]/g, " ")}</span>{" "}
            <span className="meta">{formatDate(event.created_at)}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}

function FactsPanel() {
  const [facts, setFacts] = useState<ApplicantFact[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [sensitive, setSensitive] = useState(false);

  const load = useCallback(() => {
    api
      .facts()
      .then(setFacts)
      .catch((reason: Error) => setError(reason.message));
  }, []);
  useEffect(load, [load]);

  const add = async (event: React.FormEvent) => {
    event.preventDefault();
    try {
      await api.createFact({ key, value, category: "other", sensitive });
      setKey("");
      setValue("");
      setSensitive(false);
      load();
    } catch (reason) {
      setError((reason as Error).message);
    }
  };

  return (
    <div>
      <p className="notice">
        Facts are the only things that can be written into an application form. A new or changed fact starts
        unapproved and is never used until you approve it, which is what stops anything being invented about you.
      </p>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      <form className="stacked-form" onSubmit={add}>
        <label>
          Question or field name
          <input value={key} onChange={(event) => setKey(event.target.value)} placeholder="full name" required />
        </label>
        <label>
          Value
          <input value={value} onChange={(event) => setValue(event.target.value)} required />
        </label>
        <label className="checkbox">
          <input type="checkbox" checked={sensitive} onChange={(event) => setSensitive(event.target.checked)} />
          Sensitive, always confirm before use
        </label>
        <button type="submit" className="primary">
          Add fact
        </button>
      </form>

      <ul className="cards">
        {facts.map((fact) => (
          <li className="card" key={fact.id}>
            <h3>{fact.key}</h3>
            <p className="meta">{fact.value}</p>
            <p className="badges">
              <span className={`badge ${fact.approved ? "ok" : "warn"}`}>
                {fact.approved ? "Approved" : "Not approved, will not be used"}
              </span>
              <span className="badge">v{fact.version}</span>
              {fact.sensitive && <span className="badge bad">Sensitive</span>}
            </p>
            <div className="actions">
              <button
                type="button"
                onClick={() => api.approveFact(fact.id, !fact.approved).then(load).catch(() => undefined)}
              >
                {fact.approved ? "Withdraw approval" : "Approve"}
              </button>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function PoliciesPanel() {
  const [policies, setPolicies] = useState<AuthorizationPolicy[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [sources, setSources] = useState("");

  const load = useCallback(() => {
    api
      .policies()
      .then(setPolicies)
      .catch((reason: Error) => setError(reason.message));
  }, []);
  useEffect(load, [load]);

  const create = async (event: React.FormEvent) => {
    event.preventDefault();
    try {
      await api.createPolicy({
        name,
        source_keys: sources.split(",").map((s) => s.trim()).filter(Boolean),
        max_per_day: 3,
        expires_in_days: 30,
      });
      setName("");
      setSources("");
      load();
    } catch (reason) {
      setError((reason as Error).message);
    }
  };

  return (
    <div>
      <p className="notice">
        A standing authorization lets routine applications on the sources you name go out without you approving each
        one. It is created switched off, covers only the sources listed, and can be paused or revoked at any moment.
        Anything with an unanswered or sensitive question, or asking for writing, still comes to you.
      </p>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      <form className="stacked-form" onSubmit={create}>
        <label>
          Name
          <input value={name} onChange={(event) => setName(event.target.value)} required />
        </label>
        <label>
          Source keys it covers, comma separated
          <input
            value={sources}
            onChange={(event) => setSources(event.target.value)}
            placeholder="tenstorrent-greenhouse"
            required
          />
        </label>
        <button type="submit" className="primary">
          Create, switched off
        </button>
      </form>

      <ul className="cards">
        {policies.map((policy) => (
          <li className="card" key={policy.id}>
            <h3>{policy.name}</h3>
            <p className="badges">
              <span className={`badge ${policy.revoked_at ? "bad" : policy.enabled ? "ok" : "warn"}`}>
                {policy.revoked_at ? "Revoked" : policy.enabled ? "Active" : "Switched off"}
              </span>
              <span className="badge">
                {policy.submissions_made} sent, {policy.max_per_day} a day
              </span>
              {policy.expires_at && <span className="badge">Expires {formatDate(policy.expires_at)}</span>}
            </p>
            <p className="meta">Covers: {policy.source_keys.join(", ") || "nothing"}</p>
            {!policy.revoked_at && (
              <div className="actions">
                <button
                  type="button"
                  onClick={() => api.setPolicyState(policy.id, !policy.enabled).then(load).catch(() => undefined)}
                >
                  {policy.enabled ? "Pause" : "Switch on"}
                </button>
                <button
                  type="button"
                  onClick={() => api.setPolicyState(policy.id, false, true).then(load).catch(() => undefined)}
                >
                  Revoke permanently
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

function InboxPanel() {
  const [items, setItems] = useState<import("../types").NotificationRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .notifications()
      .then(setItems)
      .catch((reason: Error) => setError(reason.message));
  }, []);
  useEffect(load, [load]);

  const tone: Record<string, string> = {
    submitted: "ok",
    action_required: "warn",
    failed: "bad",
    submission_uncertain: "bad",
  };

  return (
    <div>
      <p className="notice">
        Everything worth telling you about, kept until you have seen it. Each event appears once.
      </p>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {items.length === 0 && !error && <p className="notice empty">Nothing here.</p>}
      <ul className="cards">
        {items.map((item) => (
          <li className="card" key={item.id}>
            <p className="badges">
              <span className={`badge ${tone[item.kind] ?? ""}`}>{item.kind.replace(/_/g, " ")}</span>
              <span className="badge">{relativeAge(item.created_at)}</span>
              {item.delivered_at && <span className="badge">Seen</span>}
            </p>
            <ul className="payload">
              {Object.entries(item.payload).map(([key, value]) => (
                <li key={key}>
                  <span className="key">{key.replace(/_/g, " ")}</span>
                  {typeof value === "string" ? value : JSON.stringify(value)}
                </li>
              ))}
            </ul>
            {!item.delivered_at && (
              <div className="actions">
                <button type="button" onClick={() => api.markDelivered(item.id).then(load).catch(() => undefined)}>
                  Mark seen
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

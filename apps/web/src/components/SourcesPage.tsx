import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import { formatDate, outcomeLabel, supportLabel } from "../format";
import type { IngestSummary, Source, SourceRun, ValidationReport } from "../types";

type ActionResult = ValidationReport | IngestSummary | Source | void;

const KIND_FIELDS: Record<string, { field: string; label: string; help: string }> = {
  greenhouse: { field: "board_token", label: "Board token", help: "The path segment in the employer's Greenhouse job board URL." },
  lever: { field: "site_id", label: "Site id", help: "The path segment in the employer's jobs.lever.co URL." },
  ashby: { field: "board_slug", label: "Board slug", help: "The path segment in the employer's jobs.ashbyhq.com URL." },
  manual: { field: "url", label: "Careers URL", help: "No adapter runs against a manual entry; it records a link for you to follow." },
};

export default function SourcesPage({ onChanged }: { onChanged: () => void }) {
  const [sources, setSources] = useState<Source[]>([]);
  const [runs, setRuns] = useState<SourceRun[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [reports, setReports] = useState<Record<string, string>>({});

  const load = useCallback(() => {
    Promise.all([api.sources(), api.sourceRuns()])
      .then(([nextSources, nextRuns]) => {
        setSources(nextSources);
        setRuns(nextRuns);
        setError(null);
      })
      .catch((reason: Error) => setError(reason.message));
  }, []);

  useEffect(load, [load]);

  const act = async (id: string, action: () => Promise<ActionResult>, label: string) => {
    setBusy(id + label);
    try {
      const result = await action();
      setReports((current) => ({ ...current, [id]: describe(label, result) }));
      load();
      onChanged();
    } catch (reason) {
      setReports((current) => ({ ...current, [id]: `${label} failed: ${(reason as Error).message}` }));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section aria-labelledby="sources-heading">
      <h2 id="sources-heading">Sources</h2>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      <AddSourceForm
        onCreated={() => {
          load();
          onChanged();
        }}
      />

      <div className="table-wrap">
        <table>
          <caption className="meta">
            Support says what has been proven about a source, not what the adapter could do. A manual entry never
            reports discovered jobs.
          </caption>
          <thead>
            <tr>
              <th scope="col">Key</th>
              <th scope="col">Kind</th>
              <th scope="col">Support</th>
              <th scope="col">Health</th>
              <th scope="col">Last attempt</th>
              <th scope="col">Last success</th>
              <th scope="col">Last outcome</th>
              <th scope="col">Actions</th>
            </tr>
          </thead>
          <tbody>
            {sources.map((source) => (
              <tr key={source.id}>
                <th scope="row">
                  {source.key}
                  <br />
                  <span className="meta">{source.display_name}</span>
                </th>
                <td>{source.kind}</td>
                <td>{supportLabel(source.support)}</td>
                <td>{source.health}</td>
                <td>{formatDate(source.last_attempt_at)}</td>
                <td>{formatDate(source.last_success_at)}</td>
                <td>
                  {outcomeLabel(source.last_outcome)}
                  {source.last_diagnostic && <div className="meta">{source.last_diagnostic}</div>}
                  {reports[source.id] && <div className="meta">{reports[source.id]}</div>}
                </td>
                <td>
                  <div className="actions">
                    <button
                      type="button"
                      disabled={busy !== null}
                      onClick={() => act(source.id, () => api.validateSource(source.id), "Validate")}
                    >
                      Validate
                    </button>
                    <button
                      type="button"
                      disabled={busy !== null}
                      onClick={() => act(source.id, () => api.refreshSource(source.id), "Refresh")}
                    >
                      Refresh
                    </button>
                    <button
                      type="button"
                      disabled={busy !== null}
                      onClick={() =>
                        act(source.id, () => api.updateSource(source.id, { enabled: !source.enabled }), "Toggle")
                      }
                    >
                      {source.enabled ? "Disable" : "Enable"}
                    </button>
                  </div>
                </td>
              </tr>
            ))}
            {sources.length === 0 && (
              <tr>
                <td colSpan={8} className="notice empty">
                  No source registered yet. Add one above, or import the verified seed file with{" "}
                  <code>opportunity source import</code>.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <h3>Recent scan attempts</h3>
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th scope="col">Started</th>
              <th scope="col">Source</th>
              <th scope="col">Outcome</th>
              <th scope="col">Complete sweep</th>
              <th scope="col">Records</th>
              <th scope="col">Reported total</th>
              <th scope="col">Diagnostic</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((run) => (
              <tr key={run.id}>
                <td>{new Date(run.started_at).toLocaleString()}</td>
                <td>{run.source_key}</td>
                <td>{outcomeLabel(run.outcome)}</td>
                <td>{run.complete ? "yes" : "no"}</td>
                <td>{run.records_seen}</td>
                <td>{run.total_reported ?? "not reported"}</td>
                <td>{run.diagnostic ?? ""}</td>
              </tr>
            ))}
            {runs.length === 0 && (
              <tr>
                <td colSpan={7} className="notice empty">
                  No scan has run yet.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function describe(label: string, result: ActionResult): string {
  if (!result) return `${label} done`;
  if ("key" in result) return `${label}: ${result.key} is now ${result.enabled ? "enabled" : "disabled"}`;
  if ("reachable" in result) {
    return `${label}: ${outcomeLabel(result.outcome)}, ${result.records_seen} records, complete sweep ${
      result.complete ? "yes" : "no"
    }${result.diagnostic ? `, ${result.diagnostic}` : ""}`;
  }
  return `${label}: ${outcomeLabel(result.outcome)}, ${result.records_seen} records, ${result.postings_created} new, ${
    result.postings_updated
  } updated, ${result.postings_closed} closed`;
}

function AddSourceForm({ onCreated }: { onCreated: () => void }) {
  const [kind, setKind] = useState("greenhouse");
  const [key, setKey] = useState("");
  const [organization, setOrganization] = useState("");
  const [identifier, setIdentifier] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const spec = KIND_FIELDS[kind] ?? KIND_FIELDS.greenhouse!;

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      const created = await api.createSource({
        key,
        display_name: organization,
        organization,
        endpoint: { kind, [spec.field]: identifier },
        enabled: kind !== "manual",
        refresh_hours: 6,
        tags: [],
      });
      // Registration is only half the job: a read-only check is what turns a mistyped identifier into a visible error.
      let note = `Registered ${created.key}.`;
      if (kind !== "manual") {
        const report = await api.validateSource(created.id);
        note += ` Validation: ${outcomeLabel(report.outcome)}, ${report.records_seen} records.`;
      }
      setMessage(note);
      setKey("");
      setOrganization("");
      setIdentifier("");
      onCreated();
    } catch (reason) {
      setMessage(`Could not register: ${(reason as Error).message}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="filters" onSubmit={submit} aria-labelledby="add-source-heading">
      <h3 id="add-source-heading" style={{ gridColumn: "1 / -1", margin: 0 }}>
        Add a source
      </h3>
      <label>
        Kind
        <select value={kind} onChange={(event) => setKind(event.target.value)}>
          <option value="greenhouse">Greenhouse</option>
          <option value="lever">Lever</option>
          <option value="ashby">Ashby</option>
          <option value="manual">Manual watch (no adapter)</option>
        </select>
      </label>
      <label>
        Key
        <input value={key} onChange={(event) => setKey(event.target.value)} required placeholder="employer-greenhouse" />
      </label>
      <label>
        Organization
        <input value={organization} onChange={(event) => setOrganization(event.target.value)} required />
      </label>
      <label>
        {spec.label}
        <input value={identifier} onChange={(event) => setIdentifier(event.target.value)} required />
        <span className="meta">{spec.help}</span>
      </label>
      <button type="submit" className="primary" disabled={busy}>
        {busy ? "Registering" : "Register and validate"}
      </button>
      {message && (
        <p className="notice" style={{ gridColumn: "1 / -1", margin: 0 }} role="status">
          {message}
        </p>
      )}
    </form>
  );
}

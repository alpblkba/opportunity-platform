import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import { formatDate, freshnessLabel, outcomeLabel, relativeAge, supportLabel } from "../format";
import type { CoverageRow } from "../types";

const EXPLANATIONS: Record<string, string> = {
  verified_live: "An adapter read this source successfully. The counts below come from that read.",
  fixture_only:
    "The adapter passes golden fixtures, but no live read has been verified for this source. Zero here means untested against the real source, not empty.",
  blocked: "The source refused access. Previously discovered records are kept and are not treated as closed.",
  manual_link: "No adapter. The registration records a link to follow by hand and discovers nothing.",
  not_configured: "Registered but never attempted. Zero here means unchecked, not empty.",
};

export default function CoveragePage() {
  const [rows, setRows] = useState<CoverageRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .coverage()
      .then((value) => {
        setRows(value);
        setError(null);
      })
      .catch((reason: Error) => setError(reason.message));
  }, []);

  useEffect(load, [load]);

  const refreshAll = async () => {
    setBusy(true);
    setReport(null);
    try {
      const result = await api.refreshAll();
      const complete = result.summaries.filter((s) => s.complete).length;
      setReport(
        `Attempted ${result.sources_attempted} sources: ${complete} complete sweeps, ${
          result.summaries.length - complete
        } incomplete or failed. Incomplete sweeps did not close any record.`,
      );
      load();
    } catch (reason) {
      setReport(`Refresh failed: ${(reason as Error).message}`);
    } finally {
      setBusy(false);
    }
  };

  const groups = Object.keys(EXPLANATIONS).filter((support) => rows.some((row) => row.support === support));

  return (
    <section aria-labelledby="coverage-heading">
      <h2 id="coverage-heading">Coverage</h2>
      <p className="notice">
        This table reports what has actually been checked. A source with zero postings has either been read and found
        empty, never been read at all, or only ever been tested against fixtures, and those cases are labelled
        differently on purpose. Freshness is a separate question from support: a source can be verified live and still
        be out of date.
      </p>
      <div className="actions">
        <button type="button" className="primary" onClick={refreshAll} disabled={busy}>
          {busy ? "Refreshing" : "Refresh all enabled sources"}
        </button>
      </div>
      {report && (
        <p className="notice" role="status">
          {report}
        </p>
      )}
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}

      {groups.map((support) => (
        <div key={support}>
          <h3>{supportLabel(support as CoverageRow["support"])}</h3>
          <p className="meta">{EXPLANATIONS[support]}</p>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th scope="col">Source</th>
                  <th scope="col">Kind</th>
                  <th scope="col">Enabled</th>
                  <th scope="col">Open postings</th>
                  <th scope="col">Open topics</th>
                  <th scope="col">Last complete sweep</th>
                  <th scope="col">Freshness</th>
                  <th scope="col">Next run</th>
                  <th scope="col">Last outcome</th>
                </tr>
              </thead>
              <tbody>
                {rows
                  .filter((row) => row.support === support)
                  .map((row) => (
                    <tr key={row.source_id}>
                      <th scope="row">
                        {row.display_name}
                        <br />
                        <span className="meta">{row.key}</span>
                      </th>
                      <td>{row.kind}</td>
                      <td>{row.enabled ? "yes" : "no"}</td>
                      <td>{row.open_postings}</td>
                      <td>{row.open_topics > 0 ? row.open_topics : ""}</td>
                      <td>{formatDate(row.last_complete_sweep_at)}</td>
                      <td>
                        <span className={`badge ${row.freshness === "fresh" ? "ok" : "warn"}`}>
                          {freshnessLabel(row.freshness)}
                        </span>
                        <div className="meta">every {row.refresh_hours}h</div>
                      </td>
                      <td>
                        {row.backoff_until ? (
                          <span className="badge bad">
                            backing off after {row.consecutive_failures} failures, retry {relativeAge(row.backoff_until)}
                          </span>
                        ) : (
                          formatDate(row.next_run_at)
                        )}
                      </td>
                      <td>
                        {outcomeLabel(row.last_outcome)}
                        {row.last_diagnostic && <div className="meta">{row.last_diagnostic}</div>}
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        </div>
      ))}
      {rows.length === 0 && !error && <p className="notice empty">No source registered yet.</p>}
    </section>
  );
}

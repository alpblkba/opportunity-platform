import { useCallback, useEffect, useState } from "react";

import { api, emptyFilters, type DiscoverFilters } from "../api";
import { deadlineLabel, formatDate, isAdvertisedVacancy, kindLabel, locationLabel, orUnknown, relativeAge, stateLabel } from "../format";
import type { Opportunity, OpportunityDetail, OpportunityPage, Source } from "../types";

const KINDS = [
  "employment",
  "internship",
  "working_student",
  "master_thesis",
  "research_assistant",
  "phd",
  "part_time",
  "fellowship",
  "other",
];

export default function DiscoverPage() {
  const [filters, setFilters] = useState<DiscoverFilters>(emptyFilters);
  const [page, setPage] = useState<OpportunityPage | null>(null);
  const [sources, setSources] = useState<Source[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(() => {
    setLoading(true);
    api
      .opportunities(filters)
      .then((value) => {
        setPage(value);
        setError(null);
      })
      .catch((reason: Error) => setError(reason.message))
      .finally(() => setLoading(false));
  }, [filters]);

  useEffect(load, [load]);
  useEffect(() => {
    api.sources().then(setSources).catch(() => setSources([]));
  }, []);

  const update = <K extends keyof DiscoverFilters>(key: K, value: DiscoverFilters[K]) =>
    setFilters((current) => ({ ...current, [key]: value, page: key === "page" ? (value as number) : 1 }));

  const totalPages = page ? Math.max(1, Math.ceil(page.total / page.per_page)) : 1;

  return (
    <section aria-labelledby="discover-heading">
      <h2 id="discover-heading">Discover</h2>
      <form
        className="filters"
        onSubmit={(event) => {
          event.preventDefault();
          load();
        }}
      >
        <label>
          Search
          <input
            type="search"
            value={filters.q}
            placeholder="firmware, Masterarbeit, CXL"
            onChange={(event) => update("q", event.target.value)}
          />
        </label>
        <label>
          Country
          <input
            type="text"
            value={filters.country}
            placeholder="DE"
            maxLength={2}
            onChange={(event) => update("country", event.target.value.toUpperCase())}
          />
        </label>
        <label>
          City
          <input type="text" value={filters.city} onChange={(event) => update("city", event.target.value)} />
        </label>
        <label>
          Type
          <select value={filters.kind} onChange={(event) => update("kind", event.target.value)}>
            <option value="">Any type</option>
            {KINDS.map((kind) => (
              <option key={kind} value={kind}>
                {kindLabel(kind)}
              </option>
            ))}
          </select>
        </label>
        <label>
          Source
          <select value={filters.source} onChange={(event) => update("source", event.target.value)}>
            <option value="">Any source</option>
            {sources.map((source) => (
              <option key={source.id} value={source.id}>
                {source.display_name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Sort
          <select value={filters.sort} onChange={(event) => update("sort", event.target.value)}>
            <option value="newest">Recently seen</option>
            <option value="oldest">First seen</option>
            <option value="relevance">Text relevance</option>
            <option value="title">Title</option>
          </select>
        </label>
        <label>
          Offer
          <select value={filters.state} onChange={(event) => update("state", event.target.value)}>
            <option value="">Any kind of offer</option>
            <option value="advertised">Advertised vacancies only</option>
            <option value="open_topic">Open topics only</option>
            <option value="potential_supervisor">Supervisor contacts only</option>
          </select>
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={filters.includeClosed}
            onChange={(event) => update("includeClosed", event.target.checked)}
          />
          Include closed
        </label>
        <button type="submit" className="primary">
          Apply
        </button>
      </form>

      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {loading && <p className="notice">Loading opportunities.</p>}
      {!loading && !error && page && page.items.length === 0 && (
        <p className="notice empty">
          No opportunity matches these filters. This is not the same as having no sources or a failed scan: check the
          Coverage tab to see which sources were actually read.
        </p>
      )}

      {page && page.items.length > 0 && (
        <>
          <p className="stat-strip">
            <span>
              {page.total} matching, showing page {page.page} of {totalPages}
            </span>
          </p>
          <ul className="cards">
            {page.items.map((item) => (
              <OpportunityCard key={item.id} opportunity={item} />
            ))}
          </ul>
          <div className="pager">
            <button type="button" disabled={page.page <= 1} onClick={() => update("page", page.page - 1)}>
              Previous
            </button>
            <span>
              Page {page.page} of {totalPages}
            </span>
            <button type="button" disabled={page.page >= totalPages} onClick={() => update("page", page.page + 1)}>
              Next
            </button>
          </div>
        </>
      )}
    </section>
  );
}

function OpportunityCard({ opportunity }: { opportunity: Opportunity }) {
  const [detail, setDetail] = useState<OpportunityDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);

  const loadDetail = () => {
    if (detail) return;
    api
      .opportunity(opportunity.id)
      .then(setDetail)
      .catch((reason: Error) => setDetailError(reason.message));
  };

  return (
    <li className="card">
      <h3>{opportunity.canonical_title}</h3>
      <p className="meta">
        <span>{opportunity.organization}</span>
        <span>{locationLabel(opportunity)}</span>
        <span>{kindLabel(opportunity.kind)}</span>
      </p>
      <p className="badges">
        <span className="badge">{opportunity.source_names.join(", ") || "source unknown"}</span>
        <span className="badge">First seen {formatDate(opportunity.first_seen_at)}</span>
        <span className="badge">Last verified {relativeAge(opportunity.last_seen_at)}</span>
        <span className="badge">Published {formatDate(opportunity.posted_at)}</span>
        {opportunity.posting_count > 1 && (
          <span className="badge">{opportunity.posting_count} postings merged</span>
        )}
        {/* A thesis topic or a supervisor contact must never be read as a funded vacancy, so anything that is not an advertised post says what it actually is. */}
        {!isAdvertisedVacancy(opportunity.advertised_state) && (
          <span className="badge warn">{stateLabel(opportunity.advertised_state)}</span>
        )}
        {opportunity.supervisor && <span className="badge">Supervisor {opportunity.supervisor}</span>}
        {opportunity.title_only && <span className="badge warn">No description from source</span>}
        {opportunity.closed_at && <span className="badge bad">Closed {formatDate(opportunity.closed_at)}</span>}
        {opportunity.deadline_at && (
          <span className={`badge ${(deadlineLabel(opportunity.deadline_at) ?? "").startsWith("Deadline passed") ? "bad" : "warn"}`}>
            {deadlineLabel(opportunity.deadline_at)}
          </span>
        )}
      </p>
      <div className="actions">
        {opportunity.apply_url ? (
          <a className="badge" href={opportunity.apply_url} target="_blank" rel="noopener noreferrer">
            Open employer posting
          </a>
        ) : (
          <span className="badge warn">No application URL from source</span>
        )}
        <button type="button" onClick={loadDetail}>
          Show sources and description
        </button>
      </div>
      {detailError && (
        <p className="notice error" role="alert">
          {detailError}
        </p>
      )}
      {detail && (
        <details className="detail" open>
          <summary>{detail.postings.length} source posting(s)</summary>
          {detail.postings.map((posting) => (
            <div key={posting.id}>
              <p className="meta">
                <span>
                  {posting.source_name} ({posting.source_kind})
                </span>
                <span>id {posting.native_id}</span>
                <span>{orUnknown(posting.location_raw)}</span>
                <span>{orUnknown(posting.employment_type_raw)}</span>
                <span>{stateLabel(posting.advertised_state)}</span>
                {posting.supervisor && <span>Supervisor: {posting.supervisor}</span>}
                {posting.contact && <span>Contact: {posting.contact}</span>}
                {posting.deadline_at && <span>{deadlineLabel(posting.deadline_at)}</span>}
                <a href={posting.url} target="_blank" rel="noopener noreferrer">
                  Original posting
                </a>
              </p>
              {/* Source text is rendered as plain text, never as HTML: an employer description is untrusted input. */}
              <pre>{posting.description_text ?? "The source did not provide a description."}</pre>
            </div>
          ))}
        </details>
      )}
    </li>
  );
}

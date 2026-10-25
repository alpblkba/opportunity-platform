import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import SaveApplicationButton from "./SaveApplicationButton";
import { deadlineLabel, formatDate, kindLabel, locationLabel, relativeAge, stateLabel, isAdvertisedVacancy } from "../format";
import type { Digest, MatchPage, Profile, ScoredOpportunity } from "../types";

export default function MatchesPage({ onChanged }: { onChanged: () => void }) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [profileId, setProfileId] = useState<string>("");
  const [query, setQuery] = useState("");
  const [includeExcluded, setIncludeExcluded] = useState(false);
  const [page, setPage] = useState<MatchPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [digest, setDigest] = useState<Digest | null>(null);

  useEffect(() => {
    api
      .profiles()
      .then((next) => {
        setProfiles(next);
        setProfileId((current) => current || next[0]?.id || "");
      })
      .catch((reason: Error) => setError(reason.message));
  }, []);

  const load = useCallback(() => {
    if (!profileId) return;
    setLoading(true);
    api
      .matches(profileId, query, includeExcluded)
      .then((next) => {
        setPage(next);
        setError(null);
      })
      .catch((reason: Error) => setError(reason.message))
      .finally(() => setLoading(false));
  }, [profileId, query, includeExcluded]);

  useEffect(load, [profileId, includeExcluded]);

  const makeDigest = async (format: string) => {
    if (!profileId) return;
    try {
      setDigest(await api.generateDigest(profileId, format));
      onChanged();
    } catch (reason) {
      setError((reason as Error).message);
    }
  };

  return (
    <section aria-labelledby="matches-heading">
      <h2 id="matches-heading">Matches</h2>

      <form
        className="filters"
        onSubmit={(event) => {
          event.preventDefault();
          load();
        }}
      >
        <label>
          Profile
          <select value={profileId} onChange={(event) => setProfileId(event.target.value)}>
            {profiles.length === 0 && <option value="">No profile yet</option>}
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.display_name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Narrow by text
          <input type="search" value={query} onChange={(event) => setQuery(event.target.value)} />
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={includeExcluded}
            onChange={(event) => setIncludeExcluded(event.target.checked)}
          />
          Show only what my rules excluded
        </label>
        <button type="submit" className="primary">
          Rank
        </button>
      </form>

      {profiles.length === 0 && (
        <p className="notice empty">
          No profile exists yet. Open the Profiles tab and add the four starting tracks, then edit them.
        </p>
      )}
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {loading && <p className="notice">Ranking opportunities.</p>}

      {page && (
        <>
          <p className="notice">
            {page.ranking}. Scored {page.total_considered} open opportunities: {page.eligible} eligible,{" "}
            {page.excluded} excluded by your rules. A score orders results under this profile; it is not a probability.
          </p>
          <div className="actions">
            <button type="button" onClick={() => makeDigest("markdown")}>
              Generate Markdown digest
            </button>
            <button type="button" onClick={() => makeDigest("csv")}>
              Generate CSV digest
            </button>
            <a className="badge" href={api.exportUrl(profileId, "markdown")}>
              Download Markdown
            </a>
            <a className="badge" href={api.exportUrl(profileId, "csv")}>
              Download CSV
            </a>
          </div>

          {digest && (
            <details className="detail" open>
              <summary>
                Digest generated {formatDate(digest.generated_at)}: {digest.item_count} items,{" "}
                {digest.deadline_count} with a deadline inside 30 days
              </summary>
              <pre>{digest.body}</pre>
            </details>
          )}

          {page.items.length === 0 && (
            <p className="notice empty">
              {includeExcluded
                ? "Your rules excluded nothing from this profile."
                : "Nothing matched this profile. That is not the same as there being no openings: widen the profile, or check the Coverage tab to see which sources were actually read."}
            </p>
          )}

          <ul className="cards">
            {page.items.map((item) => (
              <MatchCard key={item.id} item={item} profileId={profileId} onChanged={load} />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

function MatchCard({
  item,
  profileId,
  onChanged,
}: {
  item: ScoredOpportunity;
  profileId: string;
  onChanged: () => void;
}) {
  const [note, setNote] = useState<string | null>(null);
  const positive = item.match.factors.filter((f) => f.contribution > 0);
  const negative = item.match.factors.filter((f) => f.contribution < 0);

  const record = async (verdict: string) => {
    try {
      await api.feedback({
        opportunity_id: item.id,
        profile_id: profileId,
        verdict,
        sampled_from: "surfaced",
      });
      setNote(verdict === "interested" ? "Marked interested." : "Marked not interested.");
    } catch (reason) {
      setNote((reason as Error).message);
    }
  };

  const hide = async () => {
    try {
      await api.createRule({ kind: "hidden", value: "", opportunity_id: item.id, note: "hidden from the matches list" });
      onChanged();
    } catch (reason) {
      setNote((reason as Error).message);
    }
  };

  return (
    <li className="card">
      <h3>
        {item.match.eligible ? `${Math.round(item.match.score)} · ` : ""}
        {item.canonical_title}
      </h3>
      <p className="meta">
        <span>{item.organization}</span>
        <span>{locationLabel(item)}</span>
        <span>{kindLabel(item.kind)}</span>
      </p>

      <p className="badges">
        <span className="badge">{item.source_names.join(", ") || "source unknown"}</span>
        <span className="badge">Last verified {relativeAge(item.last_seen_at)}</span>
        {!isAdvertisedVacancy(item.advertised_state) && (
          <span className="badge warn">{stateLabel(item.advertised_state)}</span>
        )}
        {item.supervisor && <span className="badge">Supervisor {item.supervisor}</span>}
        {item.deadline_at && <span className="badge warn">{deadlineLabel(item.deadline_at)}</span>}
        {item.title_only && <span className="badge warn">No description from source</span>}
      </p>

      {!item.match.eligible && (
        <div className="notice error">
          {item.match.exclusions.map((exclusion, index) => (
            <p key={index} style={{ margin: 0 }}>
              Excluded: {exclusion.reason}
              {exclusion.evidence ? ` (${exclusion.evidence})` : ""}
              {exclusion.reversible ? ". You can lift this in Profiles." : ""}
            </p>
          ))}
        </div>
      )}

      {positive.length > 0 && (
        <details className="detail">
          <summary>Why this ranked here, scored by {item.match.model}</summary>
          <ul>
            {positive.map((factor, index) => (
              <li key={index}>
                <strong>{factor.name}</strong>: {factor.detail}
                {factor.evidence.length > 0 && <span className="meta"> Evidence: {factor.evidence.join("; ")}</span>}
              </li>
            ))}
            {negative.map((factor, index) => (
              <li key={`n${index}`}>
                <strong>{factor.name}</strong> counted against it: {factor.detail}
              </li>
            ))}
          </ul>
        </details>
      )}

      {item.match.uncertainties.length > 0 && (
        <div className="notice">
          {item.match.uncertainties.map((uncertainty, index) => (
            <p key={index} style={{ margin: 0 }}>
              Not stated by the source, {uncertainty.field}: {uncertainty.detail}
            </p>
          ))}
        </div>
      )}

      <div className="actions">
        <SaveApplicationButton opportunityId={item.id} profileId={profileId} />
        {item.apply_url ? (
          <a className="badge" href={item.apply_url} target="_blank" rel="noopener noreferrer">
            Open employer posting
          </a>
        ) : (
          <span className="badge warn">No application URL from source</span>
        )}
        <button type="button" onClick={() => record("interested")}>
          Interested
        </button>
        <button type="button" onClick={() => record("not_interested")}>
          Not interested
        </button>
        {item.match.eligible && (
          <button type="button" onClick={hide}>
            Hide
          </button>
        )}
      </div>
      {note && (
        <p className="meta" role="status">
          {note}
        </p>
      )}
    </li>
  );
}

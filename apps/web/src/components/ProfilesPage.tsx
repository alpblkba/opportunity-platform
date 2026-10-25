import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import CvPanel from "./CvPanel";
import type { Profile, RemotePolicy, Rule, SeniorityPolicy, Vocabulary } from "../types";

const REMOTE_POLICIES: { value: RemotePolicy; label: string }[] = [
  { value: "any", label: "Any arrangement, subject to the location rules" },
  { value: "remote_from_current_country_only", label: "Only roles workable remotely from where I am now" },
  { value: "on_site_or_relocate", label: "On site in a preferred location, or worth relocating for" },
];

const SENIORITY_POLICIES: { value: SeniorityPolicy; label: string }[] = [
  { value: "preference", label: "Treat an advertised level as a preference, keep showing the role" },
  { value: "hard", label: "Hide roles asking for more than my maximum" },
  { value: "ignore", label: "Ignore advertised experience levels entirely" },
];

const RULE_KINDS = [
  { value: "company", label: "Employer (exact name, aliases included)" },
  { value: "title_term", label: "Phrase in the job title" },
  { value: "description_term", label: "Phrase in the description" },
];

const list = (value: string): string[] =>
  value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);

export default function ProfilesPage({ onChanged }: { onChanged: () => void }) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [rules, setRules] = useState<Rule[]>([]);
  const [vocabulary, setVocabulary] = useState<Vocabulary | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [draft, setDraft] = useState<Profile | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    Promise.all([api.profiles(), api.rules()])
      .then(([nextProfiles, nextRules]) => {
        setProfiles(nextProfiles);
        setRules(nextRules);
        setError(null);
        setSelected((current) => current ?? nextProfiles[0]?.id ?? null);
      })
      .catch((reason: Error) => setError(reason.message));
  }, []);

  useEffect(load, [load]);
  useEffect(() => {
    api.vocabulary().then(setVocabulary).catch(() => setVocabulary(null));
  }, []);
  useEffect(() => {
    setDraft(profiles.find((p) => p.id === selected) ?? null);
  }, [selected, profiles]);

  const set = <K extends keyof Profile>(key: K, value: Profile[K]) =>
    setDraft((current) => (current ? { ...current, [key]: value } : current));

  const save = async () => {
    if (!draft) return;
    try {
      await api.updateProfile(draft.id, draft);
      setMessage(`Saved ${draft.display_name}.`);
      load();
      onChanged();
    } catch (reason) {
      setMessage(`Could not save: ${(reason as Error).message}`);
    }
  };

  const seed = async () => {
    try {
      const result = await api.seedProfiles();
      setMessage(
        result.created.length > 0
          ? `Created starting profiles: ${result.created.join(", ")}. Edit them to make them yours.`
          : "All starting profiles already exist; nothing was overwritten.",
      );
      load();
      onChanged();
    } catch (reason) {
      setMessage(`Could not seed: ${(reason as Error).message}`);
    }
  };

  return (
    <section aria-labelledby="profiles-heading">
      <h2 id="profiles-heading">Profiles and rules</h2>
      <CvPanel />
      <p className="notice">
        A profile is what you are looking for. Everything here stays on this machine: your location, work authorization
        and education are used only to decide eligibility and are never sent to a model or published.
      </p>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {message && (
        <p className="notice" role="status">
          {message}
        </p>
      )}

      <div className="actions">
        <label>
          Editing
          <select value={selected ?? ""} onChange={(event) => setSelected(event.target.value || null)}>
            <option value="">Select a profile</option>
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.display_name}
              </option>
            ))}
          </select>
        </label>
        <button type="button" onClick={seed}>
          Add the four starting tracks
        </button>
      </div>

      {draft && (
        <form
          className="filters"
          onSubmit={(event) => {
            event.preventDefault();
            save();
          }}
        >
          <label>
            Name
            <input value={draft.display_name} onChange={(event) => set("display_name", event.target.value)} required />
          </label>
          <label>
            Priority
            <input
              type="number"
              value={draft.priority}
              onChange={(event) => set("priority", Number(event.target.value))}
            />
          </label>
          <label className="checkbox">
            <input type="checkbox" checked={draft.enabled} onChange={(event) => set("enabled", event.target.checked)} />
            Enabled
          </label>

          <label style={{ gridColumn: "1 / -1" }}>
            Interests, comma separated
            <input value={draft.terms.join(", ")} onChange={(event) => set("terms", list(event.target.value))} />
            <span className="meta">
              A term is matched literally and through related wording. Known concepts:{" "}
              {vocabulary?.concepts.join(", ") ?? "loading"}
            </span>
          </label>
          <label style={{ gridColumn: "1 / -1" }}>
            Terms to deprioritize, comma separated
            <input
              value={draft.negative_terms.join(", ")}
              onChange={(event) => set("negative_terms", list(event.target.value))}
            />
            <span className="meta">These count against the score but never hide a role.</span>
          </label>
          <label>
            Preferred technologies
            <input
              value={draft.preferred_technologies.join(", ")}
              onChange={(event) => set("preferred_technologies", list(event.target.value))}
            />
          </label>
          <label>
            Unwanted technologies
            <input
              value={draft.excluded_technologies.join(", ")}
              onChange={(event) => set("excluded_technologies", list(event.target.value))}
            />
          </label>

          <label style={{ gridColumn: "1 / -1" }}>
            Job kinds, comma separated. Empty means any kind
            <input value={draft.job_kinds.join(", ")} onChange={(event) => set("job_kinds", list(event.target.value))} />
            <span className="meta">Available: {vocabulary?.job_kinds.join(", ") ?? "loading"}</span>
          </label>

          <label>
            Preferred countries (two-letter codes)
            <input
              value={draft.preferred_countries.join(", ")}
              onChange={(event) => set("preferred_countries", list(event.target.value).map((c) => c.toUpperCase()))}
            />
          </label>
          <label>
            Preferred cities
            <input
              value={draft.preferred_cities.join(", ")}
              onChange={(event) => set("preferred_cities", list(event.target.value))}
            />
          </label>
          <label>
            Where I am now
            <input
              value={draft.current_country ?? ""}
              maxLength={2}
              placeholder="DE"
              onChange={(event) => set("current_country", event.target.value.toUpperCase() || null)}
            />
            <span className="meta">Used only to judge region-locked remote roles.</span>
          </label>
          <label className="checkbox">
            <input
              type="checkbox"
              checked={draft.relocation_willing}
              onChange={(event) => set("relocation_willing", event.target.checked)}
            />
            Willing to relocate
          </label>
          <label style={{ gridColumn: "1 / -1" }}>
            Work arrangement
            <select value={draft.remote_policy} onChange={(event) => set("remote_policy", event.target.value as RemotePolicy)}>
              {REMOTE_POLICIES.map((policy) => (
                <option key={policy.value} value={policy.value}>
                  {policy.label}
                </option>
              ))}
            </select>
          </label>
          <label className="checkbox" style={{ gridColumn: "1 / -1" }}>
            <input
              type="checkbox"
              checked={draft.include_unknown_location}
              onChange={(event) => set("include_unknown_location", event.target.checked)}
            />
            Keep opportunities whose location could not be read
          </label>

          <label>
            Maximum advertised years of experience
            <input
              type="number"
              min={0}
              max={60}
              value={draft.max_years_experience ?? ""}
              onChange={(event) => set("max_years_experience", event.target.value === "" ? null : Number(event.target.value))}
            />
          </label>
          <label style={{ gridColumn: "1 / -1" }}>
            How to treat an advertised experience level
            <select
              value={draft.seniority_policy}
              onChange={(event) => set("seniority_policy", event.target.value as SeniorityPolicy)}
            >
              {SENIORITY_POLICIES.map((policy) => (
                <option key={policy.value} value={policy.value}>
                  {policy.label}
                </option>
              ))}
            </select>
          </label>
          <label>
            Highest degree held
            <input
              value={draft.education_level ?? ""}
              placeholder="MSc"
              onChange={(event) => set("education_level", event.target.value || null)}
            />
          </label>
          <label>
            Already authorized to work in
            <input
              value={draft.work_authorization.join(", ")}
              placeholder="DE"
              onChange={(event) => set("work_authorization", list(event.target.value).map((c) => c.toUpperCase()))}
            />
            <span className="meta">Used only to flag an unanswered question, never to claim eligibility.</span>
          </label>

          <button type="submit" className="primary">
            Save profile
          </button>
        </form>
      )}

      <h3>Exclusion rules</h3>
      <p className="meta">
        An employer rule matches the exact normalized name including aliases, so excluding one company never hides an
        unrelated one whose name contains the same words.
      </p>
      <RuleForm onCreated={load} />
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th scope="col">Kind</th>
              <th scope="col">Value</th>
              <th scope="col">Note</th>
              <th scope="col">Action</th>
            </tr>
          </thead>
          <tbody>
            {rules.map((rule) => (
              <tr key={rule.id}>
                <td>{rule.kind.replace("_", " ")}</td>
                <td>{rule.value || (rule.kind === "hidden" ? "a single hidden opportunity" : "")}</td>
                <td>{rule.note ?? ""}</td>
                <td>
                  <button
                    type="button"
                    onClick={() => {
                      api.deleteRule(rule.id).then(load).catch((reason: Error) => setMessage(reason.message));
                    }}
                  >
                    Remove
                  </button>
                </td>
              </tr>
            ))}
            {rules.length === 0 && (
              <tr>
                <td colSpan={4} className="notice empty">
                  No exclusion rules. Nothing is being hidden.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function RuleForm({ onCreated }: { onCreated: () => void }) {
  const [kind, setKind] = useState("company");
  const [value, setValue] = useState("");
  const [note, setNote] = useState("");
  const [message, setMessage] = useState<string | null>(null);

  return (
    <form
      className="filters"
      onSubmit={async (event) => {
        event.preventDefault();
        try {
          await api.createRule({ kind, value, note: note || null });
          setValue("");
          setNote("");
          setMessage(null);
          onCreated();
        } catch (reason) {
          setMessage((reason as Error).message);
        }
      }}
    >
      <label>
        Kind
        <select value={kind} onChange={(event) => setKind(event.target.value)}>
          {RULE_KINDS.map((entry) => (
            <option key={entry.value} value={entry.value}>
              {entry.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        Value
        <input value={value} onChange={(event) => setValue(event.target.value)} required />
      </label>
      <label>
        Why (optional)
        <input value={note} onChange={(event) => setNote(event.target.value)} />
      </label>
      <button type="submit" className="primary">
        Add rule
      </button>
      {message && (
        <p className="notice error" style={{ gridColumn: "1 / -1", margin: 0 }} role="alert">
          {message}
        </p>
      )}
    </form>
  );
}

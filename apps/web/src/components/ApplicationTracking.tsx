import { useEffect, useState } from "react";
import { api } from "../api";
import type { ApplicationRecord, OpportunityDetail } from "../types";

export default function ApplicationTracking({ application, onSaved }: { application: ApplicationRecord; onSaved: (value: ApplicationRecord) => void }) {
  const [detail, setDetail] = useState<OpportunityDetail | null>(null);
  const [status, setStatus] = useState(application.status);
  const [notes, setNotes] = useState(application.notes ?? "");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { api.opportunity(application.opportunity_id).then(setDetail).catch(() => setDetail(null)); }, [application.opportunity_id]);
  useEffect(() => { setStatus(application.status); setNotes(application.notes ?? ""); }, [application.status, application.notes]);
  const manual = ["saved", "submitted", "interview", "rejected", "withdrawn"];
  return <div>
    <h3>{detail?.opportunity.canonical_title ?? `Opportunity ${application.opportunity_id}`}</h3>
    {detail && <p>{detail.opportunity.organization} {detail.opportunity.apply_url &&
      <a href={detail.opportunity.apply_url} target="_blank" rel="noopener noreferrer">Open employer posting</a>}</p>}
    <p className="meta">Manual tracking only. Apply on the employer’s website yourself, then record what happened here. No application is sent by these controls.</p>
    <form className="filters" onSubmit={async (event) => {
      event.preventDefault(); setBusy(true); setMessage(null);
      try { onSaved(await api.updateTracking(application.id, status, notes)); setMessage("Tracking saved. Nothing was submitted by this application."); }
      catch (e) { setMessage((e as Error).message); }
      finally { setBusy(false); }
    }}>
      <label>Recorded status<select value={status} onChange={(e) => setStatus(e.target.value)}>
        {!manual.includes(status) && <option value={status} disabled>{status.replace(/_/g, " ")}</option>}
        {manual.map((value) => <option key={value} value={value}>{value === "submitted" ? "Applied manually on employer site" : value}</option>)}
      </select></label>
      <label>Application notes<textarea value={notes} onChange={(e) => setNotes(e.target.value)} maxLength={20000} /></label>
      <button type="submit" disabled={busy || !manual.includes(status)}>Save tracking</button>
    </form>
    {message && <p role="status" className="notice">{message}</p>}
  </div>;
}

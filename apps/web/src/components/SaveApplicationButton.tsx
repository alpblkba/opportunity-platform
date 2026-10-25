import { useState } from "react";
import { api } from "../api";

export default function SaveApplicationButton({ opportunityId, profileId }: { opportunityId: string; profileId?: string }) {
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return <span>
    <button type="button" disabled={busy} onClick={async () => {
      setBusy(true);
      try { await api.createApplication(opportunityId, profileId); setMessage("Saved in Workspace → Applications. Nothing was submitted."); }
      catch (e) { setMessage((e as Error).message); }
      finally { setBusy(false); }
    }}>Save to applications</button>
    {message && <span role="status" className="meta">{message}</span>}
  </span>;
}

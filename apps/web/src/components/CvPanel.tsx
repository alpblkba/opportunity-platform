import { useEffect, useState } from "react";
import { api } from "../api";
import type { CvDocument } from "../types";

export default function CvPanel() {
  const [documents, setDocuments] = useState<CvDocument[]>([]);
  const [file, setFile] = useState<File | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  useEffect(() => { api.cvs().then(setDocuments).catch((e: Error) => setError(e.message)); }, []);
  return <section aria-labelledby="cv-heading">
    <h3 id="cv-heading">Your CV</h3>
    <p className="notice">Stored privately on this server. Uploading does not submit an application or extract qualifications.
      Matching uses the interests and rules you edit below; it is not semantic CV ranking. Add applicant details in Workspace → Facts.</p>
    <form className="actions" onSubmit={async (event) => {
      event.preventDefault(); if (!file) return;
      setBusy(true); setError(null); setMessage(null);
      try {
        await api.uploadCv(file); setDocuments(await api.cvs());
        setMessage("CV uploaded. Earlier versions are kept. Nothing was submitted.");
      } catch (e) { setError((e as Error).message); }
      finally { setBusy(false); }
    }}>
      <label>CV file (PDF or text, up to 5 MiB)
        <input type="file" accept=".pdf,.txt,application/pdf,text/plain" required onChange={(e) => setFile(e.target.files?.[0] ?? null)} />
      </label>
      <button type="submit" disabled={!file || busy}>{busy ? "Uploading…" : "Upload CV"}</button>
    </form>
    {error && <p role="alert" className="notice error">{error}</p>}
    {message && <p role="status" className="notice">{message}</p>}
    <ul>{documents.map((doc) => <li key={doc.id}>
      <a href={`/api/documents/${doc.id}/download`} download={doc.label}>{doc.label}</a>
      {` · version ${doc.version} · ${Math.ceil(doc.byte_size / 1024)} KiB`}
    </li>)}</ul>
  </section>;
}

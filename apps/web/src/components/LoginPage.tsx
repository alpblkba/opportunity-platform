import { useEffect, useRef, useState } from "react";

import { api } from "../api";

/** Where someone lands if they decide this is not for them. The personal site, which is what this
 *  link has always been labelled; the /opportunities/ page it used to point at is going away, and
 *  this application's own landing page replaced it. */
const PERSONAL_SITE = "https://alpblkba.dev/";

interface Props {
  /** Set when the session ran out underneath them rather than when they arrived fresh. */
  expired?: boolean;
  onSignedIn: () => void;
}

export default function LoginPage({ expired = false, onSignedIn }: Props) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const emailField = useRef<HTMLInputElement>(null);

  useEffect(() => emailField.current?.focus(), []);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.login(email, password);
      onSignedIn();
    } catch (reason) {
      // Whatever the server said, and nothing invented: it deliberately does not say whether the
      // address exists, and guessing here would undo that.
      setError(reason instanceof Error ? reason.message : "that did not work");
      setPassword("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <main className="login" id="main">
      <form className="login-card" onSubmit={submit} aria-labelledby="login-heading">
        <p className="login-prompt" aria-hidden="true">
          alpblkba@opportunities:~$
        </p>
        <h1 id="login-heading">Alp&apos;s Opportunity Platform</h1>
        <p className="login-sub">Private. One account. Everything here belongs to its owner.</p>

        {expired && !error && (
          <p className="notice" role="status">
            That session ran out. Sign in again to pick up where you were.
          </p>
        )}
        {error && (
          <p className="notice error" role="alert">
            {error}
          </p>
        )}

        <label htmlFor="login-email">Email</label>
        <input
          id="login-email"
          ref={emailField}
          type="email"
          name="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          autoComplete="username"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          required
          disabled={busy}
        />

        <label htmlFor="login-password">Password</label>
        <input
          id="login-password"
          type="password"
          name="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          autoComplete="current-password"
          required
          disabled={busy}
        />

        <button type="submit" className="primary" disabled={busy}>
          {busy ? "Checking…" : "Sign in"}
        </button>

        <p className="login-foot">
          <a href={PERSONAL_SITE}>← back to alpblkba.dev</a>
        </p>
      </form>
    </main>
  );
}

import { useCallback, useEffect, useState } from "react";

import { api, ApiError } from "./api";
import CoveragePage from "./components/CoveragePage";
import LandingPage from "./components/LandingPage";
import LoginPage from "./components/LoginPage";
import DiscoverPage from "./components/DiscoverPage";
import MatchesPage from "./components/MatchesPage";
import ProfilesPage from "./components/ProfilesPage";
import WorkspacePage from "./components/WorkspacePage";
import SourcesPage from "./components/SourcesPage";
import { formatDate } from "./format";
import type { Stats } from "./types";

const TABS = [
  { id: "workspace", label: "Workspace" },
  { id: "matches", label: "Matches" },
  { id: "discover", label: "Discover" },
  { id: "profiles", label: "Profiles" },
  { id: "sources", label: "Sources" },
  { id: "coverage", label: "Coverage" },
] as const;

type TabId = (typeof TABS)[number]["id"];

/** Who is signed in: unknown while the first check is in flight, null when nobody is. */
type SessionState = "checking" | { email: string; display_name: string; via: string } | null;

/** The three pages this application has URLs for. Everything else is a deep link into the
 *  workspace, which the single-page app owns once someone is signed in. */
type Route = "landing" | "login" | "workspace";

function routeOf(path: string): Route {
  if (path === "/login") return "login";
  if (path === "/" || path === "/index.html") return "landing";
  return "workspace";
}

/** What the gate wants before it will let anyone in. */
type Gate =
  /** A password, on a login form this application owns. */
  | { kind: "password" }
  /** The identity layer in front, which this application cannot offer. Reloading goes to it. */
  | { kind: "upstream"; detail: string }
  /** The gate could not be asked at all. */
  | { kind: "unreachable"; detail: string };

export default function App() {
  const [route, setRoute] = useState<Route>(() => routeOf(window.location.pathname));
  const [tab, setTab] = useState<TabId>("workspace");
  const [stats, setStats] = useState<Stats | null>(null);
  const [statsError, setStatsError] = useState<string | null>(null);
  const [session, setSession] = useState<SessionState>("checking");
  const [expired, setExpired] = useState(false);
  /** What to show when nobody is signed in, decided by what the server said was missing. */
  const [gate, setGate] = useState<Gate>({ kind: "password" });

  /** Move without reloading, and leave an entry so the back button means something. */
  const go = useCallback((path: string, replace = false) => {
    if (window.location.pathname !== path) {
      if (replace) window.history.replaceState({}, "", path);
      else window.history.pushState({}, "", path);
    }
    setRoute(routeOf(path));
  }, []);

  // The back and forward buttons move between these pages like any other site's.
  useEffect(() => {
    const onPop = () => setRoute(routeOf(window.location.pathname));
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  // Ask the server, every time. The bundle is public where a native session is the boundary, so
  // nothing the browser believes about being logged in means anything until the API agrees.
  const checkSession = useCallback(
    () =>
      api
        .session()
        .then((value) => {
          setSession(value);
          setExpired(false);
          return true;
        })
        .catch((error: unknown) => {
          setSession(null);
          // Three different situations, and showing a password form for all of them was the bug:
          // where Cloudflare Access is the boundary there is no password to type, so a form would
          // be a dead end for the one person allowed in.
          if (error instanceof ApiError && error.status === 401) {
            setGate(
              error.needs === "native"
                ? { kind: "password" }
                : { kind: "upstream", detail: error.message },
            );
          } else {
            setGate({
              kind: "unreachable",
              detail: `Cannot reach the API: ${error instanceof Error ? error.message : "no answer"}.`,
            });
          }
          return false;
        }),
    [],
  );
  useEffect(() => {
    void checkSession();
  }, [checkSession]);

  const reloadStats = useCallback(() => {
    api
      .stats()
      .then((value) => {
        setStats(value);
        setStatsError(null);
      })
      .catch((error: Error) => {
        // The session can run out between one request and the next; that is not an error to show
        // beside the data, it is a reason to ask for a password again.
        if (error instanceof ApiError && error.status === 401) {
          setSession(null);
          setExpired(true);
          setGate(
            error.needs === "native"
              ? { kind: "password" }
              : { kind: "upstream", detail: error.message },
          );
          return;
        }
        setStatsError(error.message);
      });
  }, []);

  useEffect(() => {
    if (session && session !== "checking") reloadStats();
  }, [session, reloadStats]);

  const signOut = () => {
    void api
      .logout()
      .catch(() => undefined)
      .then(() => {
        setSession(null);
        setExpired(false);
        setStats(null);
        // Out means out: back to the public page, not to a workspace URL that would bounce.
        go("/");
      });
  };

  // Whether the landing page belongs to someone who is already signed in depends on how they got
  // in. With a native session it does: / is this application's front door, and an owner who types
  // the address should see it rather than be bounced past it. Where the identity came from the
  // layer in front, the address was never a front door - it went straight to the workspace, and a
  // rollback has to keep doing that.
  const nativeSession = session !== "checking" && session !== null && session.via === "password";
  useEffect(() => {
    if (session && session !== "checking" && !nativeSession && route !== "workspace") {
      go("/workspace", true);
    }
    // A native session on the login page has nothing left to do there.
    if (nativeSession && route === "login") {
      go("/workspace", true);
    }
  }, [session, nativeSession, route, go]);

  // The public page is public: it asks nothing of the server and shows nothing that belongs to
  // anyone. It is rendered before the session check finishes so it does not flicker through a
  // loading state on the one page that has nothing to load.
  if (route === "landing" && (session === null || session === "checking" || nativeSession)) {
    return (
      <LandingPage
        // Someone already signed in has no password left to type: the button takes them in.
        signedIn={nativeSession}
        onLogin={() => go(nativeSession ? "/workspace" : "/login")}
      />
    );
  }

  // One check decides everything: the interface does not exist for someone who is not signed in.
  if (session === "checking") {
    return (
      <main className="login" id="main">
        <p className="login-card notice">Checking your session…</p>
      </main>
    );
  }
  if (session === null && gate.kind !== "password") {
    // Nothing this application can fix: the identity layer in front decides, and it is reached by
    // loading the page again rather than by typing anything here.
    return (
      <main className="login" id="main">
        <div className="login-card">
          <p className="login-prompt" aria-hidden="true">
            alpblkba@opportunities:~$
          </p>
          <h1>Alp&apos;s Opportunity Platform</h1>
          <p className="notice error" role="alert">
            {gate.detail}
          </p>
          <p className="login-sub">
            {gate.kind === "upstream"
              ? "This deployment is behind an identity check that happens before the application. Reload to go through it again."
              : "The application could not be reached. It may be restarting."}
          </p>
          <button type="button" className="primary" onClick={() => window.location.reload()}>
            Reload
          </button>
          <p className="login-foot">
            <a href="https://alpblkba.dev/">← back to alpblkba.dev</a>
          </p>
        </div>
      </main>
    );
  }
  if (session === null) {
    return (
      <LoginPage
        expired={expired}
        onSignedIn={() => {
          void checkSession().then(() => {
            // /login is where they arrived; the workspace is where they are now.
            if (window.location.pathname === "/login") {
              window.history.replaceState({}, "", "/");
            }
          });
        }}
      />
    );
  }

  return (
    <>
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="app">
        <h1>Opportunity platform</h1>
        <p className="who">
          <span>{session.display_name}</span>
          <button type="button" className="linklike" onClick={signOut}>
            Sign out
          </button>
        </p>
        <nav className="tabs" aria-label="Sections">
          {TABS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              aria-current={tab === entry.id ? "page" : undefined}
              onClick={() => setTab(entry.id)}
            >
              {entry.label}
            </button>
          ))}
        </nav>
      </header>
      <main id="main">
        {statsError && (
          <p className="notice error" role="alert">
            Cannot reach the API: {statsError}. Start it with <code>./run.sh</code>.
          </p>
        )}
        {stats && (
          <p className="stat-strip">
            <span>{stats.opportunities_open} open opportunities</span>
            <span>{stats.postings_open} postings</span>
            <span>{stats.organizations} organizations</span>
            <span>
              {stats.sources_verified_live} of {stats.sources_total} sources verified live
            </span>
            <span>{stats.open_topics} open topics</span>
            <span>Last complete sweep: {formatDate(stats.last_complete_sweep_at)}</span>
            {stats.sources_stale > 0 && <span>{stats.sources_stale} stale</span>}
            {stats.sources_in_backoff > 0 && <span>{stats.sources_in_backoff} backing off</span>}
          </p>
        )}
        {tab === "workspace" && <WorkspacePage />}
        {tab === "matches" && <MatchesPage onChanged={reloadStats} />}
        {tab === "discover" && <DiscoverPage />}
        {tab === "profiles" && <ProfilesPage onChanged={reloadStats} />}
        {tab === "sources" && <SourcesPage onChanged={reloadStats} />}
        {tab === "coverage" && <CoveragePage />}
      </main>
    </>
  );
}

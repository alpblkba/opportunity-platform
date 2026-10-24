/**
 * Navigation gate.
 *
 * The worker refuses to open anything that is not a loopback address. This is the rule that keeps an autonomous build session away from a real employer application form, and it is enforced in code rather than by instruction, because an instruction is not a control. Opening it up is a deliberate act by the owner, not a flag this process can set for itself.
 */

/** Hosts the worker may navigate to. Nothing else is reachable. */
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "[::1]", "::1", "0:0:0:0:0:0:0:1"]);

export class NavigationBlocked extends Error {
  constructor(readonly url: string, readonly why: string) {
    super(`refusing to navigate to ${url}: ${why}`);
    this.name = "NavigationBlocked";
  }
}

/**
 * Whether the owner has deliberately enabled navigation beyond loopback.
 *
 * Deliberately read from the environment on every call rather than cached, and deliberately requiring an exact value, so that enabling it is a visible act by whoever starts the process. No autonomous session sets this.
 */
export function externalNavigationEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.BROWSER_WORKER_ALLOW_EXTERNAL === "owner-supervised";
}

/** Throws unless the URL is a loopback address, or the owner has explicitly opened the gate. */
export function assertNavigable(rawUrl: string, env: NodeJS.ProcessEnv = process.env): URL {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new NavigationBlocked(rawUrl, "it is not a valid URL");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new NavigationBlocked(rawUrl, `the ${url.protocol} scheme is not allowed`);
  }
  const host = url.hostname.toLowerCase();
  if (LOOPBACK_HOSTS.has(host)) {
    return url;
  }
  if (externalNavigationEnabled(env)) {
    return url;
  }
  throw new NavigationBlocked(
    rawUrl,
    "only loopback addresses are reachable. Opening a real employer application form requires the owner to start this worker with BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised and to be present while it runs",
  );
}

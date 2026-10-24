/** The personal site, which this page belongs beside rather than inside. */
const PERSONAL_SITE = "https://alpblkba.dev/";
/** What this thing is, written up over there. */
const ABOUT = "https://alpblkba.dev/projects/opportunity-platform/";
/** The joke. There is exactly one other account and it is not yours. */
const OTHER_USER = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

interface Props {
  /** True when a native session already exists, so the button goes in rather than to the form. */
  signedIn?: boolean;
  /** Moves to the login page, or straight to the workspace, without reloading the application. */
  onLogin: () => void;
}

export default function LandingPage({ signedIn = false, onLogin }: Props) {
  const destination = signedIn ? "/workspace" : "/login";
  return (
    <main className="landing" id="main">
      <nav className="landing-nav" aria-label="Elsewhere">
        <a href={PERSONAL_SITE}>back to home</a>
      </nav>

      <div className="landing-middle">
        <h1>Alp&apos;s Opportunity Platform</h1>
        <p className="landing-description">
          A personal platform for discovering, evaluating and tracking the things worth applying to.
        </p>

        <div className="landing-actions">
          <a
            className="landing-button primary"
            href={destination}
            onClick={(event) => {
              // Ordinary same-tab navigation, handled in the application so the page does not
              // reload. The modifier keys are left alone so a middle or command click still opens
              // a new tab, which is what every other link on the web does.
              if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) {
                return;
              }
              event.preventDefault();
              onLogin();
            }}
          >
            Log in as Alp (alpblkba)
          </a>
          <a className="landing-button" href={OTHER_USER}>
            Log in as another user
          </a>
        </div>
      </div>

      <div className="landing-foot">
        <p className="landing-tagline">bro casually replacing Linkedin</p>
        <p className="landing-about">
          <a href={ABOUT}>what is this?</a>
        </p>
        <img
          className="landing-meme"
          src="/linkedin-vibe.webp"
          width={461}
          height={565}
          alt="A man on a porch looking back over his shoulder, captioned: I bring a sort of “fuck linkedin” vibe to linkedin that linkedin don't really like."
        />
      </div>
    </main>
  );
}

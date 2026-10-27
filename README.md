# Opportunity Platform

Private, single-owner job discovery and applications. Sign in, upload a PDF or text CV once, and choose one of five career profiles. The stored CV drives current ranked opportunities, application status and employer questions on one screen.

Run locally with `./run.sh` (Rust, Node.js, PostgreSQL and `pdftotext` for PDFs). Open http://127.0.0.1:8787. Existing accounts remain valid; create an initial owner with `target/release/opportunity user create --email <your-address>` after loading `.env`.

Automatic applications require an explicit employer/profile authorization in the application. Greenhouse, Lever and Ashby hosted forms are inspected locally; unsupported controls, login and CAPTCHA require manual completion. A visible employer confirmation is required for success. Set `APPLICATION_ENV=development` to prohibit real submissions during testing.

For email notifications, configure `MAIL_API_KEY`, `MAIL_FROM`, and `MAIL_TO` in the ignored `.env`. The default delivery API is Resend; `MAIL_API_URL` can point to a compatible transport. Acceptance is recorded separately from provider-confirmed delivery. No notification credentials are included.

Product acceptance journey: `TEST_OFFLINE=1 node scripts/test-product.mjs` (run in CI). It uses an isolated PostgreSQL database, a synthetic CV and ranking corpus, and loopback-only application/mail fixtures, with every source disabled. Without `TEST_OFFLINE` it also waits for live discovery from real public listings, as a smoke test. Real applications and real recipient emails are never sent. Ranking rules: [docs/RANKING_POLICY.md](docs/RANKING_POLICY.md).

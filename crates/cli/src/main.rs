use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use opportunity_adapters::HttpClient;
use opportunity_api::{router, AppState};
use opportunity_core::sources::{NewSource, SourceUpdate};
use opportunity_core::{config, ingest, query, sources, Db};
use opportunity_domain::SourceEndpoint;

#[derive(Parser)]
#[command(
    name = "opportunity",
    version,
    about = "Personal job, internship and thesis discovery"
)]
struct Cli {
    /// PostgreSQL connection string. Defaults to DATABASE_URL from the environment.
    #[arg(long, global = true, env = "DATABASE_URL")]
    database_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply pending migrations and exit.
    Migrate,
    /// Apply migrations and serve the API, and the built frontend when one is present.
    Serve {
        #[arg(long, env = "API_HOST", default_value = "127.0.0.1")]
        host: String,
        #[arg(long, env = "API_PORT", default_value_t = 8787)]
        port: u16,
        /// Directory holding the built frontend. Serving it from the API keeps the whole app on one port.
        #[arg(long, env = "WEB_DIST", default_value = "apps/web/dist")]
        web_dist: PathBuf,
        /// Turn off the background refresh loop. Sources are then only scanned on demand.
        #[arg(long, default_value_t = false)]
        no_scheduler: bool,
    },
    /// Inspect and edit the source registry.
    #[command(subcommand)]
    Source(SourceCommand),
    /// Scan sources and persist the results.
    Scan {
        /// Source key. Omit to scan every enabled source that has an adapter.
        #[arg(long)]
        source: Option<String>,
    },
    /// Print counts of what the database currently holds.
    Stats,
    /// Print operational status: queue, sources, notifications, release. Run on the VM with docker exec.
    Ops,
    /// Print the coverage table: what is verified live, fixture only, blocked or manual.
    Coverage,
    /// Create and manage the accounts that may log in. The only way an account comes into existence.
    #[command(subcommand)]
    User(UserCommand),
}

#[derive(Subcommand, Debug)]
enum UserCommand {
    /// Create an account. The password is read from the terminal or from a pipe, never from an
    /// argument, because an argument is in the shell history and in the process list.
    Create {
        #[arg(long)]
        email: String,
        /// How the interface greets them. Defaults to the part before the @.
        #[arg(long)]
        name: Option<String>,
        /// owner for the person this service belongs to, agent for a non-interactive client.
        /// Additional human accounts are refused until cross-account isolation is enforced.
        #[arg(long, default_value = "owner")]
        role: String,
    },
    /// Replace an account's password, ending every session it had.
    Passwd {
        #[arg(long)]
        email: String,
    },
    /// List the accounts. No hashes, no sessions, no secrets.
    List,
    /// End every session belonging to an account, without changing its password.
    RevokeSessions {
        #[arg(long)]
        email: String,
    },
}

#[derive(Subcommand)]
enum SourceCommand {
    /// List registered sources with their health and support labels.
    List,
    /// Import sources from a TOML file. Existing keys are left untouched.
    Import {
        #[arg(default_value = "config/sources.seed.toml")]
        path: PathBuf,
    },
    /// Register one source from the command line.
    Add {
        #[arg(long)]
        key: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        organization: String,
        /// Greenhouse board token, Lever site id, or Ashby board slug.
        #[arg(long)]
        identifier: Option<String>,
        /// URL for an academic or manual watch entry.
        #[arg(long)]
        url: Option<String>,
        /// Academic sources only: name of a built-in selector preset, for example `successfactors`. Explicit selectors are set from the Sources screen or a TOML import.
        #[arg(long)]
        pattern: Option<String>,
        #[arg(long, default_value_t = false)]
        enabled: bool,
    },
    /// Enable or disable a source by key.
    Enable {
        key: String,
        #[arg(long, default_value_t = true)]
        enabled: bool,
    },
    /// Make one read-only request against a source and report what came back. Nothing is persisted.
    Validate { key: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    let database_url = cli
        .database_url
        .clone()
        .context("no database url: set DATABASE_URL or pass --database-url (see .env.example)")?;
    let db = opportunity_core::connect(&database_url)
        .await
        .context("could not connect to PostgreSQL")?;

    match cli.command {
        Command::Migrate => {
            opportunity_core::migrate(&db).await?;
            println!("migrations applied");
        }
        Command::Serve {
            host,
            port,
            web_dist,
            no_scheduler,
        } => serve(db, &host, port, web_dist, !no_scheduler).await?,
        Command::Source(command) => source_command(&db, command).await?,
        Command::Scan { source } => scan(&db, source.as_deref()).await?,
        Command::Stats => println!(
            "{}",
            serde_json::to_string_pretty(&query::stats(&db).await?)?
        ),
        Command::Ops => println!(
            "{}",
            serde_json::to_string_pretty(&opportunity_core::ops::status(&db).await?)?
        ),
        Command::Coverage => print_coverage(&db).await?,
        Command::User(command) => user_command(&db, command).await?,
    }
    Ok(())
}

/// Read a password without echoing it, and without it reaching the shell history, the process list
/// or a deployment archive. A pipe is accepted so a password manager can supply it directly.
fn read_password(prompt: &str) -> Result<String> {
    use std::io::{BufRead, IsTerminal, Write};
    if std::io::stdin().is_terminal() {
        print!("{prompt}");
        std::io::stdout().flush()?;
        let password = rpassword::read_password().context("could not read the password")?;
        print!("repeat: ");
        std::io::stdout().flush()?;
        let again = rpassword::read_password()?;
        if password != again {
            anyhow::bail!("those did not match");
        }
        Ok(password)
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        Ok(line.trim_end_matches(['\n', '\r']).to_owned())
    }
}

async fn user_command(db: &Db, command: UserCommand) -> Result<()> {
    use opportunity_core::auth::{self, Role};
    match command {
        UserCommand::Create { email, name, role } => {
            let role = Role::parse(&role).map_err(anyhow::Error::msg)?;
            let display_name =
                name.unwrap_or_else(|| email.split('@').next().unwrap_or("owner").to_owned());
            // An agent has no password: it is given a session out of band rather than logging in.
            let password = match role {
                Role::Agent => None,
                _ => Some(read_password("password: ")?),
            };
            let account =
                auth::create_account(db, &email, &display_name, role, password.as_deref())
                    .await
                    .map_err(anyhow::Error::msg)?;
            println!("created {} as {}", account.email, account.role.as_str());
            if account.role == Role::Agent {
                println!("an agent account has no password; issue it a session deliberately.");
            }
        }
        UserCommand::Passwd { email } => {
            let password = read_password("new password: ")?;
            let account = auth::set_password(db, &email, &password)
                .await
                .map_err(anyhow::Error::msg)?;
            println!(
                "changed the password for {}; every session it had is now ended",
                account.email
            );
        }
        UserCommand::List => {
            let accounts = auth::list_accounts(db).await.map_err(anyhow::Error::msg)?;
            if accounts.is_empty() {
                println!("no accounts yet. `opportunity user create --email <address>`");
            }
            for account in accounts {
                println!(
                    "{:<32} {:<8} {}",
                    account.email,
                    account.role.as_str(),
                    if account.disabled {
                        "disabled"
                    } else {
                        "active"
                    }
                );
            }
        }
        UserCommand::RevokeSessions { email } => {
            let accounts = auth::list_accounts(db).await.map_err(anyhow::Error::msg)?;
            let account = accounts
                .into_iter()
                .find(|a| a.email.eq_ignore_ascii_case(email.trim()))
                .context("no account for that address")?;
            let ended = auth::revoke_all_sessions(db, account.id)
                .await
                .map_err(anyhow::Error::msg)?;
            println!("ended {ended} session(s) for {}", account.email);
        }
    }
    Ok(())
}

async fn serve(
    db: Db,
    host: &str,
    port: u16,
    web_dist: PathBuf,
    scheduler_enabled: bool,
) -> Result<()> {
    opportunity_core::migrate(&db)
        .await
        .context("migrations failed")?;
    let web_dist = web_dist.join("index.html").exists().then_some(web_dist);
    if web_dist.is_none() {
        tracing::warn!("no built frontend found; serving the API only. Run `npm --prefix apps/web run build` to serve the UI from this process.");
    }
    opportunity_core::product::initialize(&db).await?;
    let http = HttpClient::new()?;
    if scheduler_enabled {
        opportunity_core::product::spawn(db.clone());
        // State lives in the database, so this resumes the previous process's schedule rather than restarting it.
        opportunity_core::scheduler::spawn(db.clone(), http.clone());
        println!("background refresh is on; each source follows its own refresh_hours cadence");
    } else {
        println!("background refresh is off; refresh from the UI, the API or `opportunity scan`");
    }
    // Decided before anything binds, so a misconfiguration stops the process rather than exposing an origin.
    let access = opportunity_api::access::mode_from_env().map_err(anyhow::Error::msg)?;
    opportunity_api::access::guard_bind_address(&access, host).map_err(anyhow::Error::msg)?;
    println!("access: {}", access.describe());
    if access.uses_sessions() {
        // A deployment whose login page nobody can get past is worth saying out loud here rather
        // than leaving to be discovered at the form.
        match opportunity_core::auth::account_count(&db).await {
            Ok(0) => println!(
                "accounts: none yet. Create the owner with `opportunity user create --email <address>`, on this machine."
            ),
            Ok(n) => println!("accounts: {n}"),
            Err(error) => println!("accounts: could not be counted: {error}"),
        }
    }

    // Probed once at startup so the interface can state plainly whether a model is involved, rather than discovering it per request.
    let embeddings =
        opportunity_core::embeddings::EmbeddingProvider::from_env().map(std::sync::Arc::new);
    match &embeddings {
        Some(provider) => match provider.status().await {
            status if status.is_available() => {
                println!("semantic reranking is on: {}", status.label())
            }
            status => println!("{}", status.label()),
        },
        None => println!("semantic reranking is off: no embedding provider is configured"),
    }
    // Said at startup rather than discovered when the owner first taps prepare, because a deployment without the browser is a different product: everything still works except filling a form for you.
    if opportunity_core::worker::worker_available() {
        println!("assisted application is on: forms are inspected and filled by the bundled browser worker");
    } else {
        println!("assisted application is off: no browser worker in this deployment. Applications are tracked and dossiers prepared, and every application is a manual handoff to the employer's own page.");
    }
    // A cookie marked Secure is only sent back over HTTPS, and the `__Host-` prefix needs it. The
    // deciding question is how this process is reached, not which mode it is in: anything bound to
    // loopback is a development server on plain HTTP, and anything else is behind the tunnel.
    // SECURE_COOKIES=yes forces it on for a loopback server behind an HTTPS proxy.
    let secure_cookies = match std::env::var("SECURE_COOKIES").as_deref() {
        Ok("yes") => true,
        Ok("no") => false,
        _ => !matches!(host, "127.0.0.1" | "::1" | "localhost"),
    };
    let state = AppState {
        db,
        http,
        embeddings,
        access,
        web_dist,
        secure_cookies,
        logins: std::sync::Arc::new(opportunity_api::auth::LoginLimiter::default()),
    };
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("invalid host or port")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not bind {addr}"))?;
    println!("listening on http://{addr}");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("shutting down");
}

async fn source_command(db: &Db, command: SourceCommand) -> Result<()> {
    match command {
        SourceCommand::List => {
            for source in sources::list(db).await? {
                println!(
                    "{:<26} {:<14} enabled={:<5} health={:<12} support={:<14} last_outcome={}",
                    source.key,
                    source.kind,
                    source.enabled,
                    source.health,
                    source.support,
                    source.last_outcome.as_deref().unwrap_or("never attempted")
                );
            }
        }
        SourceCommand::Import { path } => {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let report = config::import_sources(db, &text).await?;
            println!("created: {:?}", report.created);
            println!("already present: {:?}", report.skipped_existing);
            for (key, reason) in &report.rejected {
                println!("rejected {key}: {reason}");
            }
        }
        SourceCommand::Add {
            key,
            kind,
            organization,
            identifier,
            url,
            pattern,
            enabled,
        } => {
            let endpoint = build_endpoint(
                &kind,
                identifier.as_deref(),
                url.as_deref(),
                pattern.as_deref(),
            )?;
            let source = sources::create(
                db,
                NewSource {
                    key,
                    display_name: organization.clone(),
                    organization,
                    endpoint,
                    enabled,
                    refresh_hours: 6,
                    tags: Vec::new(),
                },
            )
            .await?;
            println!("registered {} ({})", source.key, source.id);
        }
        SourceCommand::Enable { key, enabled } => {
            let source = sources::get_by_key(db, &key)
                .await?
                .context("no source with that key")?;
            let updated = sources::update(
                db,
                source.id,
                SourceUpdate {
                    enabled: Some(enabled),
                    ..Default::default()
                },
            )
            .await?;
            println!("{} enabled={}", updated.key, updated.enabled);
        }
        SourceCommand::Validate { key } => {
            let source = sources::get_by_key(db, &key)
                .await?
                .context("no source with that key")?;
            let endpoint = source.typed_endpoint()?;
            if !endpoint.kind().has_adapter() {
                println!(
                    "{}: kind '{}' has no adapter, so nothing was requested from the source. This is a manual watch entry whose link you follow by hand.",
                    source.key,
                    endpoint.kind()
                );
                return Ok(());
            }
            endpoint.validate().map_err(anyhow::Error::msg)?;
            let client = HttpClient::new()?;
            let scan = opportunity_adapters::scan(&client, &endpoint).await;
            println!(
                "{}: outcome={} complete={} records={} diagnostic={}",
                source.key,
                scan.outcome,
                scan.outcome.is_complete_sweep(),
                scan.stats.records_seen,
                scan.diagnostic.as_deref().unwrap_or("none")
            );
        }
    }
    Ok(())
}

fn build_endpoint(
    kind: &str,
    identifier: Option<&str>,
    url: Option<&str>,
    pattern: Option<&str>,
) -> Result<SourceEndpoint> {
    let identifier = || -> Result<String> {
        identifier
            .map(str::to_owned)
            .context("--identifier is required for this source kind")
    };
    let url = || -> Result<String> {
        url.map(str::to_owned)
            .context("--url is required for this source kind")
    };
    let endpoint = match kind {
        "greenhouse" => SourceEndpoint::Greenhouse {
            board_token: identifier()?,
        },
        "lever" => SourceEndpoint::Lever {
            site_id: identifier()?,
        },
        "ashby" => SourceEndpoint::Ashby {
            board_slug: identifier()?,
        },
        "academic_html" => SourceEndpoint::AcademicHtml {
            url: url()?,
            pattern: pattern.map(str::to_owned),
            selectors: None,
        },
        "manual" => SourceEndpoint::Manual { url: url()? },
        "workday" => anyhow::bail!(
            "workday needs tenant, instance and site; register it from the Sources screen"
        ),
        other => anyhow::bail!("unknown source kind '{other}'"),
    };
    endpoint.validate().map_err(anyhow::Error::msg)?;
    Ok(endpoint)
}

async fn scan(db: &Db, source_key: Option<&str>) -> Result<()> {
    let client = HttpClient::new()?;
    let targets = match source_key {
        Some(key) => {
            let source = sources::get_by_key(db, key)
                .await?
                .context("no source with that key")?;
            anyhow::ensure!(
                source.typed_endpoint()?.kind().has_adapter(),
                "source '{}' has kind '{}', which has no adapter; it is a manual watch entry to follow by hand",
                source.key,
                source.kind
            );
            vec![source]
        }
        None => sources::list(db)
            .await?
            .into_iter()
            .filter(|s| {
                s.enabled
                    && s.typed_endpoint()
                        .map(|e| e.kind().has_adapter())
                        .unwrap_or(false)
            })
            .collect(),
    };
    if targets.is_empty() {
        println!(
            "no enabled source with an adapter; register one with `opportunity source import`"
        );
        return Ok(());
    }
    for source in &targets {
        let endpoint = source.typed_endpoint()?;
        let result = opportunity_adapters::scan(&client, &endpoint).await;
        let summary = ingest::ingest(db, source, result).await?;
        println!("{}", serde_json::to_string(&summary)?);
    }
    Ok(())
}

async fn print_coverage(db: &Db) -> Result<()> {
    println!(
        "{:<26} {:<14} {:<16} {:<12} {:>8}  last complete sweep",
        "key", "kind", "support", "health", "open"
    );
    for row in query::coverage(db).await? {
        println!(
            "{:<26} {:<14} {:<16} {:<12} {:>8}  {}",
            row.key,
            row.kind,
            row.support,
            row.health,
            row.open_postings,
            row.last_complete_sweep_at
                .map(|t| t.to_rfc3339())
                .unwrap_or_else(|| "never".into())
        );
    }
    Ok(())
}

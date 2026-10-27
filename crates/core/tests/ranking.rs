//! Ranking coverage at catalogue sizes beyond the e2-micro shortlist cap, and recall on a labelled
//! corpus. Synthetic data only. The timings printed here are this machine's, not the VM's.

use std::time::Instant;

use opportunity_core::{product, profiles, Db};
use sqlx::{Connection, Executor, PgConnection};
use uuid::Uuid;

struct TestDb {
    pool: Db,
    admin_url: String,
    name: String,
}

impl TestDb {
    async fn create() -> Option<Self> {
        let base = std::env::var("DATABASE_URL").ok()?;
        let (prefix, _) = base.rsplit_once('/')?;
        let admin_url = format!("{prefix}/postgres");
        let name = format!("opportunity_test_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("connect to maintenance database");
        admin
            .execute(format!(r#"create database "{name}""#).as_str())
            .await
            .expect("create test database");
        let pool = opportunity_core::connect(&format!("{prefix}/{name}"))
            .await
            .expect("connect");
        opportunity_core::migrate(&pool).await.expect("migrate");
        Some(Self {
            pool,
            admin_url,
            name,
        })
    }

    async fn cleanup(self) {
        self.pool.close().await;
        if let Ok(mut admin) = PgConnection::connect(&self.admin_url).await {
            let _ = admin
                .execute(
                    format!(r#"drop database if exists "{}" with (force)"#, self.name).as_str(),
                )
                .await;
        }
    }
}

macro_rules! require_db {
    () => {
        match TestDb::create().await {
            Some(db) => db,
            None => {
                eprintln!("SKIPPED: DATABASE_URL is not set, so this PostgreSQL integration test did not run");
                return;
            }
        }
    };
}

async fn synthetic_cv(pool: &Db) {
    let candidate = serde_json::json!({
        "name": null, "email": null, "phone": null, "degree": null, "experience_years": 2,
        "education": ["M.Sc. Computer Science (synthetic)"], "employment": ["Network software engineer (synthetic)"],
        "projects": ["RISC-V core in SystemVerilog (synthetic)"],
        "skills": ["C", "C++", "Linux", "RTOS", "SystemVerilog", "RISC-V", "Python", "CUDA", "DRAM"],
    });
    sqlx::query("insert into documents (owner_id, kind, label, file_path, content_hash, byte_size, media_type, candidate, approved)
                 select id, 'cv', 'synthetic.txt', 'private/documents/synthetic.txt', 'synthetic', 1, 'text/plain', $1, true from owners limit 1")
        .bind(candidate).execute(pool).await.unwrap();
}

/// One opportunity with one open posting; `age_minutes` orders recency.
async fn opportunity(
    pool: &Db,
    title: &str,
    kind: &str,
    description: &str,
    age_minutes: i64,
) -> Uuid {
    let (org, source) = fixtures(pool).await;
    let id: Uuid = sqlx::query_scalar(
        "insert into opportunities (organization_id, dedupe_key, canonical_title, kind, advertised_state, search_text, first_seen_at, last_seen_at)
         values ($1, $2, $3, $4, 'advertised', $3 || ' ' || $5, now() - make_interval(mins => $6::int), now() - make_interval(mins => $6::int)) returning id")
        .bind(org).bind(Uuid::new_v4().to_string()).bind(title).bind(kind).bind(description).bind(age_minutes)
        .fetch_one(pool).await.unwrap();
    sqlx::query("insert into postings (source_id, native_id, opportunity_id, url, title, description_text, content_hash) values ($1, $2, $3, 'https://jobs.lever.co/example/1', $4, $5, 'x')")
        .bind(source).bind(id.to_string()).bind(id).bind(title).bind(description).execute(pool).await.unwrap();
    id
}

async fn fixtures(pool: &Db) -> (Uuid, Uuid) {
    let org: Uuid = sqlx::query_scalar("insert into organizations (normalized_name, display_name) values ('example silicon', 'Example Silicon') on conflict (normalized_name) do update set updated_at = now() returning id")
        .fetch_one(pool).await.unwrap();
    let source: Uuid = sqlx::query_scalar("insert into sources (key, display_name, kind, endpoint) values ('example-lever', 'Example', 'lever', '{\"kind\":\"lever\",\"site\":\"example\"}') on conflict (key) do update set display_name = excluded.display_name returning id")
        .fetch_one(pool).await.unwrap();
    (org, source)
}

/// `count` newer postings that pass the firmware prefilter on body terms alone, half of them
/// unrelated listings (bulk SQL, so 10,000 rows take seconds, not minutes).
async fn catalogue(pool: &Db, count: i64) {
    let (org, source) = fixtures(pool).await;
    sqlx::query(
        "with rows as (
           select g, case when g % 2 = 0 then 'Software Engineer, Platform Tools' else 'Marketing Internship' end as title,
                  case when g % 2 = 0 then 'Our platform team occasionally touches firmware and embedded tooling.' else 'Social media and brand campaigns.' end as body
           from generate_series(1, $3::int) g)
         insert into opportunities (organization_id, dedupe_key, canonical_title, kind, advertised_state, search_text, first_seen_at, last_seen_at)
         select $1, 'bulk-' || g, title, 'employment', 'advertised', title || ' ' || body, now() - make_interval(secs => g), now() - make_interval(secs => g) from rows")
        .bind(org).bind(source).bind(count).execute(pool).await.unwrap();
    sqlx::query("insert into postings (source_id, native_id, opportunity_id, url, title, description_text, content_hash)
                 select $1, o.id::text, o.id, 'https://jobs.lever.co/example/' || o.id, o.canonical_title, substr(o.search_text, length(o.canonical_title) + 2), 'x'
                 from opportunities o where o.dedupe_key like 'bulk-%'")
        .bind(source).execute(pool).await.unwrap();
}

async fn profile(pool: &Db, key: &str) -> Uuid {
    profiles::get_by_key(pool, key).await.unwrap().unwrap().id
}

/// The e2-micro cap. Process-wide, which is why this binary sets it once and no test here changes it.
fn production_cap() {
    std::env::set_var("MATCH_SHORTLIST", "2000");
}

#[tokio::test]
async fn an_old_relevant_internship_survives_the_shortlist_cap_at_every_catalogue_size() {
    production_cap();
    for size in [500_i64, 2_000, 5_000, 10_000] {
        let db = require_db!();
        profiles::seed_default_profiles(&db.pool).await.unwrap();
        synthetic_cv(&db.pool).await;
        catalogue(&db.pool, size).await;
        // Older than every bulk row: under recency ordering it is the first thing a full cap drops.
        let needle = opportunity(
            &db.pool,
            "Firmware Engineering Intern",
            "internship",
            "Embedded C firmware on an RTOS; bootloader bring-up on RISC-V boards.",
            10_000_000,
        )
        .await;
        let firmware = profile(&db.pool, "firmware").await;
        let started = Instant::now();
        let ranked = product::ranked(&db.pool, firmware).await.unwrap();
        let elapsed = started.elapsed();
        let position = ranked.iter().position(|j| j.job.id == needle);
        eprintln!(
            "catalogue {size:>6} + 1: ranked {:>5} in {:>4} ms; needle at {position:?}",
            ranked.len(),
            elapsed.as_millis()
        );
        assert_eq!(
            position,
            Some(0),
            "catalogue {size}: the old internship was dropped or outranked"
        );
        db.cleanup().await;
    }
}

#[tokio::test]
async fn labelled_corpus_recall_precision_and_stage_order() {
    production_cap();
    let db = require_db!();
    profiles::seed_default_profiles(&db.pool).await.unwrap();
    synthetic_cv(&db.pool).await;
    let corpus: &[(&str, &str, &str, &[&str])] = &[
        (
            "Firmware Intern",
            "internship",
            "Embedded C on an ARM microcontroller with an RTOS.",
            &["firmware"],
        ),
        (
            "Werkstudent Embedded Software",
            "working_student",
            "Embedded Linux device driver development in C.",
            &["firmware"],
        ),
        (
            "BSP Engineer",
            "employment",
            "Board support package (BSP) bring-up and bootloader work in C.",
            &["firmware"],
        ),
        (
            "Junior Embedded Engineer",
            "employment",
            "Firmware for sensor nodes in C and C++.",
            &["firmware"],
        ),
        (
            "Senior Firmware Engineer",
            "employment",
            "Firmware for SSD controllers in C. 8+ years of firmware experience required.",
            &["firmware"],
        ),
        (
            "Linux Kernel Intern",
            "internship",
            "Memory management and scheduling in the Linux kernel, in C.",
            &["memory"],
        ),
        (
            "DRAM Validation Engineer",
            "employment",
            "Characterize DDR5 and DRAM timing with Python.",
            &["memory"],
        ),
        (
            "CPU Verification Intern",
            "internship",
            "UVM and SystemVerilog verification of a RISC-V CPU.",
            &["architecture"],
        ),
        (
            "RTL Design Engineer",
            "employment",
            "RTL in SystemVerilog for RISC-V cores. 3+ years of RTL experience required.",
            &["architecture"],
        ),
        (
            "FPGA Prototyping Working Student",
            "working_student",
            "SystemVerilog on FPGA prototypes of an SoC.",
            &["architecture"],
        ),
        (
            "ASIC Physical Design Staff Engineer",
            "employment",
            "ASIC timing closure and floorplanning.",
            &["architecture"],
        ),
        (
            "AI Accelerator Compiler Intern",
            "internship",
            "MLIR compiler passes for an AI accelerator, C++.",
            &["ai-systems"],
        ),
        (
            "GPU Kernel Engineer",
            "employment",
            "CUDA kernels for inference on GPU clusters.",
            &["ai-systems"],
        ),
        (
            "Masterarbeit: Cache-Kohärenz für RISC-V",
            "master_thesis",
            "Masterarbeit zu Cache-Kohärenz in RISC-V Multicore-Architektur.",
            &["architecture", "research"],
        ),
        (
            "PhD Position in Computer Architecture",
            "phd",
            "Funded doctoral position on CPU microarchitecture.",
            &["architecture", "research"],
        ),
        (
            "Marketing Internship",
            "internship",
            "Social media and brand campaigns.",
            &[],
        ),
        (
            "Frontend Developer Intern",
            "internship",
            "React and TypeScript web interfaces.",
            &[],
        ),
        (
            "Senior Software Engineer, Payments",
            "employment",
            "Java microservices for payments. Mentor interns.",
            &[],
        ),
        (
            "Sales Engineer, Embedded Boards",
            "employment",
            "Sell embedded boards to customers; extensive travel.",
            &[],
        ),
    ];
    for (i, (title, kind, description, _)) in corpus.iter().enumerate() {
        opportunity(&db.pool, title, kind, description, i as i64).await;
    }
    let mut report = Vec::new();
    for key in [
        "firmware",
        "memory",
        "architecture",
        "ai-systems",
        "research",
    ] {
        let ranked = product::ranked(&db.pool, profile(&db.pool, key).await)
            .await
            .unwrap();
        let titles: Vec<&str> = ranked
            .iter()
            .map(|j| j.job.canonical_title.as_str())
            .collect();
        let relevant: Vec<&str> = corpus
            .iter()
            .filter(|c| c.3.contains(&key))
            .map(|c| c.0)
            .collect();
        let recalled = relevant.iter().filter(|t| titles.contains(t)).count();
        let top: Vec<&str> = titles.iter().take(10).copied().collect();
        let precise = top.iter().filter(|t| relevant.contains(t)).count();
        report.push(format!(
            "{key:>12}: recall {recalled}/{} · top-10 precision {precise}/{} · ranked {:?}",
            relevant.len(),
            top.len(),
            titles
        ));
        // Every labelled relevant posting is ranked, and no unrelated internship reaches any profile.
        assert_eq!(
            recalled,
            relevant.len(),
            "{key} missed relevant postings: {titles:?}"
        );
        for unrelated in [
            "Marketing Internship",
            "Frontend Developer Intern",
            "Senior Software Engineer, Payments",
        ] {
            assert!(!titles.contains(&unrelated), "{key} ranked {unrelated}");
        }
    }
    eprintln!("{}", report.join("\n"));

    // Same technical area: internship ahead of junior ahead of senior.
    let firmware = product::ranked(&db.pool, profile(&db.pool, "firmware").await)
        .await
        .unwrap();
    let at = |title: &str| {
        firmware
            .iter()
            .position(|j| j.job.canonical_title == title)
            .unwrap()
    };
    assert!(at("Firmware Intern") < at("Junior Embedded Engineer"));
    assert!(at("Junior Embedded Engineer") < at("Senior Firmware Engineer"));
    // A mandatory years requirement is never settled by general experience.
    let architecture = product::ranked(&db.pool, profile(&db.pool, "architecture").await)
        .await
        .unwrap();
    let rtl = architecture
        .iter()
        .find(|j| j.job.canonical_title == "RTL Design Engineer")
        .unwrap();
    assert_eq!(rtl.eligibility, "needs_review");
    assert!(!rtl.auto_eligible);
    // Research weighs an advertised PhD above an internship on career stage (technical fit differs).
    let research = product::ranked(&db.pool, profile(&db.pool, "research").await)
        .await
        .unwrap();
    let stage = |title: &str| {
        research
            .iter()
            .find(|j| j.job.canonical_title == title)
            .unwrap()
            .career_priority
    };
    assert!(stage("PhD Position in Computer Architecture") > stage("CPU Verification Intern"));
    assert!(
        stage("Masterarbeit: Cache-Kohärenz für RISC-V")
            >= stage("PhD Position in Computer Architecture")
    );
    db.cleanup().await;
}

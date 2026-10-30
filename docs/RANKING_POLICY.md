# Ranking policy

The dashboard uses `career-priority-v3`, implemented in [product.rs](../crates/core/src/product.rs). Scores express relative priority for the selected career profile and current CV; they are not hiring probabilities or verified qualifications. Ranking is computed per request. Without a parsed current CV, the dashboard ranking is empty.

## Career profiles

| Profile key | Technical direction terms |
| --- | --- |
| `firmware` | firmware, embedded, RTOS, bootloader, device driver, platform systems, microcontroller, BSP |
| `memory` | memory, DRAM, CXL, DDR, linux kernel, operating systems, Linux, storage |
| `architecture` | CPU, RTL, microarchitecture, RISC-V, Verilog, SystemVerilog, VHDL, FPGA, SoC, verification, ASIC, architecture |
| `ai-systems` | AI, GPU, accelerator, compiler, CUDA, machine learning, co-design, inference |
| `research` | Combined terms from the four engineering profiles |

These fixed direction terms determine technical relevance. They are distinct from editable saved-profile terms, which contribute to the profile score. Stage words such as `internship`, `thesis` and `PhD` alone do not establish technical relevance. Memory uses `linux kernel` rather than bare `kernel`, although the independent `Linux` direction term also matches.

[profiles.rs](../crates/core/src/profiles.rs) seeds the five profiles without overwriting existing profiles. The engineering profiles initially have no job-kind restriction. The research profile initially accepts `master_thesis`, `research_assistant`, `internship` and `phd`. Saved preferences and exclusions can change the candidate set.

## Candidate selection

[matching.rs](../crates/core/src/matching.rs) loads open opportunities whose deadline is absent or in the future and whose search vector matches the profile's direction query. PostgreSQL uses `websearch_to_tsquery('simple', ...)`; multiword direction terms are quoted. Candidates are ordered by a title query match, full-text rank, most recent `last_seen_at`, then ID.

`MATCH_SHORTLIST` bounds this set before scoring: the executable defaults to 10,000 and the e2-micro Compose overlay defaults to 2,000. This ordering retains relevant title matches ahead of recent unrelated records, but the cap can still exclude relevant opportunities. PostgreSQL tokenization and the later normalized phrase matcher are separate operations; the term list and shortlist are recall limits, not a guarantee of exhaustive retrieval.

Only active postings contribute descriptions and source information. The query aggregates description and department text using `max` and chooses the minimum posting URL; it does not merge every source description. The dashboard chooses one active source ordered by posting URL. Candidates without an active source are omitted.

## Technical fit

Technical relevance requires at least one direction term in the title or at least two direction terms in the description. Otherwise technical fit is zero, regardless of CV skills. Phrase matching uses normalized text; C and C++ use the candidate parser's specialized matching.

| Input | Contribution |
| --- | ---: |
| At least one direction term in the title | 40 |
| No title term, but at least two description terms | 20 |
| CV skills found in the title or description | 7 each, at most 35 |
| CV contains projects and at least two skills match | 8 |
| CV contains employment and at least two skills match | 8 |
| Opportunity last seen less than two whole days ago | 5 |
| Each technical review gap | -10 |

The technical score is floored at zero. Candidates below 20 are discarded before stage priority is added. Project and employment bonuses check the presence of those CV sections, not whether individual projects or jobs establish experience in the advertised field. The recency bonus uses discovery history, not the employer's publication date.

## Eligibility and exclusions

The saved-profile assessment in [domain matching](../crates/domain/src/matching.rs) applies job kinds, location rules, seniority policy and global or per-profile exclusion rules. A failed hard rule removes the opportunity from dashboard ranking.

Company exclusions use exact normalized employer identity. Title and description exclusions use whole normalized phrases in the specified field. Hidden-opportunity rules apply to the selected opportunity. Negative terms and unwanted technologies reduce the profile score; they do not independently exclude a result.

Location rules distinguish preferred cities and countries, unrestricted remote work, recognized remote restrictions, relocation and unknown locations. `RemoteFromCurrentCountryOnly` rejects roles not recognized as workable remotely from the saved current country. Otherwise preferred locations, workable remote locations and permitted relocation can retain a role. Unknown locations follow `include_unknown_location`. Remote wording is parsed with a finite vocabulary, so the result requires review when the source's geography is not represented accurately.

The profile scorer records uncertainties separately from exclusions. An uncertainty does not necessarily make `MatchResult.eligible` false. In particular, saved work-authorization information can produce an uncertainty without blocking dashboard `auto_eligible`; dashboard review gaps use the checks below. An `eligible` label therefore means these implemented checks found no dashboard gap, not that every employment condition is confirmed.

### Review gaps

The technical scorer adds a gap for:

- A recognized required number of years, even if the CV lists enough total years, because those years are not attributed to the required field.
- A senior, staff, principal, director, manager or lead title when required years are absent and the CV establishes fewer than five total years.
- `phd required`, `ph.d. required` or `must have a phd` when the parsed CV degree is not exactly `PhD`.
- Description text containing `citizen`, `security clearance`, `right to work`, `work authorization` or `no sponsorship`.
- A title-only posting, defined by ingestion as fewer than 40 characters of stripped description text or no description.

The years parser recognizes a number from 1 through 30 followed by an English `year` or German `jahr` prefix. A sentence containing `preferred`, `a plus`, `nice to have`, `ideally`, `desirable`, `bonus`, `advantage` or `wünschenswert` does not add a required-years gap. These are lexical rules and do not cover every phrasing or numerical range.

After technical scoring, a PhD-stage result whose `advertised_state` is not `advertised` receives an additional `No funded vacancy confirmed` gap. This later gap does not subtract another 10 technical points. Any dashboard gap sets `eligibility=needs_review`. `advertised` is a source classification, not independent verification of funding.

## Profile score

The saved-profile score uses `lexical-concepts-v1`: literal interest terms, related concept phrases, title matches, interest coverage, job kind, location, preferred and unwanted technologies, negative terms, seniority and freshness. Contributions use the stored profile weights. Positive raw totals map to `100 * raw / (raw + 6)`; nonpositive totals and hard exclusions score zero.

The dashboard calls this scorer without embeddings. The separate profile-matching path can rerank up to 50 lexical candidates when a configured embedding provider is available. Enabling that provider does not add semantic scoring to `career-priority-v3`. See [deployment configuration](DEPLOYMENT.md#runtime-configuration).

## Career stage and priority

Career stage uses the source kind and title. For generic `employment`, `other` or `part_time` kinds, the classifier can recover thesis, internship, working-student, research-assistant or PhD categories from the title. Seniority words override a category recovered from that same title; explicit source categories are otherwise trusted. German `wissenschaftlich` and `mitarbeiter` title wording is classified as PhD.

When no title or kind establishes a stage, years mentioned in the description can lower the stage to mid-level at five years or senior at eight years. This fallback includes preferred years. Other description wording, such as mentoring interns, does not establish a stage.

| Career stage | Engineering priority | Research priority |
| --- | ---: | ---: |
| Master's thesis | 100 | 100 |
| Internship | 100 | 85 |
| Working student | 92 | 80 |
| Research assistant | 92 | 92 |
| PhD, `advertised` | 65 | 95 |
| PhD, other advertised state | 65 | 60 |
| New graduate / early career | 78 | 72 |
| Junior / Engineer I / associate engineer | 74 | 68 |
| Level not stated | 52 | 52 |
| Mid / Engineer II / fallback of 5 to 7 years | 36 | 36 |
| Senior / lead / fallback of at least 8 years | 8 | 8 |
| Staff / principal / director | 2 | 2 |

Final priority is `t * technical + p * profile_match + s * career_priority`, clamped to 0 through 100. Technical and profile components are individually clamped to that range before weighting.

| Profiles | `t` | `p` | `s` |
| --- | ---: | ---: | ---: |
| `firmware`, `memory`, `architecture`, `ai-systems` | 0.55 | 0.15 | 0.30 |
| `research` | 0.45 | 0.20 | 0.35 |

Results sort by final score descending, `last_seen_at` descending, then ID. The weights and stage values are policy choices that have not been calibrated against application outcomes.

Dashboard filters apply after scoring and before the 60-result display cap. Stage, country, source and eligibility use case-insensitive equality; employer uses substring matching. `q` searches title, employer and ranking explanation, not the full description. `saved=true` restricts results to saved marks. `total_ranked` counts before these filters and `matches` counts before the display cap. Saving or marking for review does not change scores; dismissal and employer blocking create exclusion rules.

## Automatic applications

Ranking does not grant permission to submit. `auto_eligible` requires technical relevance, at least two matching CV skills, no technical gaps, an unboosted technical score of at least 55, a passing saved-profile assessment with score at least 20, and `advertised_state=advertised`.

Automatic queueing also checks for an enabled, unrevoked, unexpired policy matching the profile and selected source. The application workflow independently checks submission authorization and safety conditions. Before a policy-backed submission, `policy_eligibility_hold` recomputes ranking for the application's profile; a missing or no-longer-eligible opportunity is held for owner review. Career-stage priority cannot satisfy either authorization requirement. Queueing or attempting a submission does not establish success. The dashboard distinguishes provider-confirmed submissions, owner-reported status and uncertain attempts.

## Validation and limitations

[ranking.rs](../crates/core/tests/ranking.rs) contains labelled relevance and synthetic shortlist tests. Unit tests in [product.rs](../crates/core/src/product.rs) cover stage classification, required versus preferred experience, exclusions and automatic eligibility. These fixtures establish expected behavior for their cases, not general recall or production performance.

A technically worded sales title such as `Sales Engineer, Embedded Boards` can still rank for firmware. Missing direction terms can exclude relevant work, and incomplete descriptions reduce evidence. The research profile can rank technical internships because its default kinds include them. Scores and eligibility should be reviewed with the original posting before an application is authorized.

# Opportunity ranking (`career-priority-v2`)

The dashboard's priority is a ranking heuristic for this owner, not a hiring probability or a
qualification score. It is computed per request from the selected profile and the current CV
(`crates/core/src/product.rs`).

## Order of operations

1. **Candidate set.** Only open opportunities whose search text contains one of the profile's
   direction terms are loaded, so the `MATCH_SHORTLIST` cap never drops an older relevant posting
   behind newer irrelevant ones.
2. **Technical fit (0-100).** Direction terms in the title (or two in the body), CV skills found in
   the posting, CV projects/employment, minus 10 per gap. Below 20 the posting is dropped; no stage
   bonus can rescue it. The research profile uses the four technical areas' terms; stage words
   ("internship", "thesis") give no technical credit.
3. **Hard exclusions.** The saved profile's job kinds, location preferences and global or
   per-profile blacklist rules drop the posting.
4. **Review gaps.** Experience years, required PhD, work authorization wording, title-only
   listings, and PhD posts without a confirmed vacancy mark the posting `needs_review`. Unknown is
   never treated as eligible.
5. **Priority** = `t * technical + p * profile + s * career_stage`, clamped to 0-100.

| Profile | t | p | s |
| --- | ---: | ---: | ---: |
| firmware, memory, architecture, ai-systems | 0.55 | 0.15 | 0.30 |
| research | 0.45 | 0.20 | 0.35 |

| Career stage (from kind or title only) | Engineering | Research |
| --- | ---: | ---: |
| Master's thesis | 100 | 100 |
| Internship | 100 | 85 |
| Working student / student role | 92 | 80 |
| Research assistant | 92 | 92 |
| PhD, advertised vacancy | 65 | 95 |
| PhD, topic or supervisor page only | 65 | 60 |
| New graduate / early career | 78 | 72 |
| Junior / Engineer I / associate | 74 | 68 |
| Level not stated | 52 | 52 |
| Mid / Engineer II / 5-7 years required | 36 | 36 |
| Senior / lead / 8+ years required | 8 | 8 |
| Staff / principal / director | 2 | 2 |

These values are an initial policy, not calibrated against outcomes.

## Automatic applications

Ranking never authorizes a submission. Auto-queueing requires the unboosted CV match to be strong
with no gaps, the saved-profile assessment to pass, an advertised vacancy, and a separate,
unexpired employer authorization created by the owner.

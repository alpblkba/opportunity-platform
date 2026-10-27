# Opportunity ranking (`career-priority-v3`)

The dashboard's priority is a ranking heuristic for this owner, not a hiring probability or a
qualification score. It is computed per request from the selected profile and the current CV
(`crates/core/src/product.rs`).

## Order of operations

1. **Candidate set.** Only open opportunities whose search text contains one of the profile's
   direction terms are loaded, ordered by a direction term in the title first, then full-text rank,
   then recency, and capped at `MATCH_SHORTLIST` (2,000 on the e2-micro). The guarantee is that the
   cap drops the weakest candidates, not the oldest: measured on synthetic catalogues of 500, 2,000,
   5,000 and 10,000 postings (`crates/core/tests/ranking.rs`), an old internship with "firmware" in
   its title stayed first at every size, where the previous recency ordering dropped it at 5,000.
   It is not a guarantee that nothing is ever cut: with more title matches than the cap, the
   lowest-ranked title matches are. Ranking 10,000 took about 330 ms on the development Mac, not the VM.
   The prefilter uses exactly the terms technical fit needs, so it excludes nothing fit() would
   score; recall is bounded by the term lists themselves (a posting that says none of them is not found).
2. **Technical fit (0-100).** Direction terms in the title (or two in the body), CV skills found in
   the posting, CV projects/employment, minus 10 per gap. Below 20 the posting is dropped; no stage
   bonus can rescue it. The research profile uses the four technical areas' terms; stage words
   ("internship", "thesis") give no technical credit.
3. **Hard exclusions.** The saved profile's job kinds, location preferences and global or
   per-profile blacklist rules drop the posting.
4. **Review gaps.** Any of the following marks a posting `needs_review`, and unknown is never
   treated as eligible:
   - a *required* number of years: always for review, because the CV's years are not attributed to a
     field and general or telecom employment is not years of RTL or CPU design. A sentence that says
     preferred, a plus, nice to have, ideally or desirable is a wish and adds no gap.
   - a required PhD ("PhD preferred" is not one), work-authorization wording, title-only listings,
     and PhD posts without a confirmed vacancy.
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

## Terms and measured behaviour (v3)

v3 added `microcontroller` and `BSP` (firmware) and `SystemVerilog`, `VHDL`, `FPGA` and `SoC`
(architecture), and narrowed memory's `kernel` to `linux kernel`, because a CUDA kernel is not an
operating-system kernel. On the labelled corpus in `crates/core/tests/ranking.rs`, every labelled
relevant posting is recalled for all five profiles, and no unrelated internship or generic software
role is ranked. Known false positive: a title such as "Sales Engineer, Embedded Boards" matches on
the title term and still ranks under firmware. The research profile also ranks technical internships,
because its saved job kinds include them.

Career stage comes from the source's kind or the title, never the description, so "mentor interns"
or "work with senior engineers" do not change it. A kind recovered from a title ("Senior Manager,
Internship Programs") loses to seniority in that same title.

Automatic submission is stricter than this ranking: `process_application` re-ranks the opportunity
for the application's profile right before a policy-backed submission and sends anything that is
not `auto_eligible` to the owner as `needs_review`.

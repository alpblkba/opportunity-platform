# Writing style contract

Every document, source comment, operational procedure and development record in this repository follows this contract. Read it before writing or editing prose. It overrides default writing behavior, including any stylistic preferences supplied by an AI coding tool.

This contract governs writing, not software architecture or implementation authority. Existing technical specifications, security constraints and repository ownership rules remain in effect.

## Register

Determine the document type before writing.

README files and technical documentation describe actual behavior, configuration, interfaces and constraints. Use second person for instructions and commands. Markdown is expected.

Operational runbooks describe executable procedures, prerequisites, expected results, failure conditions and recovery actions. Write for an engineer operating the system, not for an AI agent completing a task.

Source comments explain behavior that is difficult to infer from the surrounding code. Prefer describing invariants, constraints and reasons for implementation choices over restating operations.

Commit messages, issues and pull requests state what changed and why, without narrating the development process. Commit subjects are short, imperative or descriptive, and do not use Conventional Commit prefixes.

Private tracelogs record observations, actions, results and unresolved issues. They are engineering records, not progress announcements or conversations with the repository owner.

Never apply blog voice to a technical document.

## Do not produce

- Bulleted items that open with a bolded two-word label and a colon.
- A new heading every two or three sentences.
- Tables for content that is not genuinely tabular.
- A TL;DR, bottom line, key takeaways block or redundant summary section.
- Title case headings, emojis or decorative separators.
- An opening that praises, restates or acknowledges the request.
- Brochure register, benefits framing, superlatives or enthusiasm about a tool.
- A closing sentence that widens the scope beyond the work described.
- Meta commentary about the text itself.
- Session recaps inside public technical documentation.
- AI-generated project narratives that describe ordinary engineering work as a sequence of achievements.
- Decorative status labels, excessive bold text or repetitive assurances of correctness.

## Capitalization

Headings are sentence case. Only the first word is capitalized unless subsequent words are proper nouns or identifiers.

Preserve conventional capitalization for proper nouns, brands, product names, acronyms, programming languages, standards and protocols.

In prose, sentences start with a capital letter. Do not capitalize words for emphasis.

Preserve the exact spelling and case of Rust items, TypeScript identifiers, SQL relations, environment variables, Git references, file paths, CLI flags, HTTP endpoints and GitHub Actions keys.

Do not alter identifiers for stylistic consistency.

## Structure

Open with the result, the claim, the component or the operation being documented. Do not add scene setting.

When describing a defect and its correction, state the cause before the fix.

Give complete artifacts where execution depends on completeness. Include full command sequences, configuration blocks, required environment variables and expected exit conditions.

Use paragraphs of approximately three to six sentences, with one principal idea per paragraph. Shorter paragraphs are appropriate for procedures, reference entries and isolated technical constraints.

Sections can be uneven. Give more space to technically complex material instead of imposing uniform section lengths.

Place qualifications next to the claims they limit.

Describe failures and unresolved conditions where they affect correctness or operation.

Stop when the content ends. Do not append a conclusion or future work section unless it contains specific information that belongs in the document.

Do not add headings merely to organize a small number of sentences.

## Sentences

Most sentences should contain approximately twelve to eighteen words. A sentence longer than twenty-five words should justify its complexity.

Short sentences are useful, but avoid uniform sentence lengths.

Use subordinate clauses when the subordinate information genuinely depends on the main statement.

Prefer active voice. Use passive voice when the actor is irrelevant or unknown.

Prefer concrete nouns, exact mechanisms and measured values over adjectives.

Hedge on judgment, never on established facts.

Do not replace technical explanations with broad characterizations such as improved reliability, better performance or stronger security.

State the actual mechanism, measurement or invariant.

Emphasize through clarity and consequence, not stacked adverbs, typography or repetition.

## Preserve qualification

Never remove a clause merely to make a sentence smoother.

A condition, mechanism, exception, contrast, scope restriction or uncertainty must survive a rewrite.

Preserve distinctions between optional and required behavior, synchronous and asynchronous operations, read-only and mutating operations, and observed and expected results.

Preserve the conditions under which an operation is safe.

Clumsy and complete is preferable to fluent and vague.

## Purpose, not only cause

For a step involving a real engineering choice, make at least one of these visible: what was unknown, what the step would establish, or which alternative was ruled out and why.

Constraints count as purpose.

Keep purpose short and attached to the relevant sentence. A subordinate clause is usually sufficient.

Do not introduce a step with "the goal here is to".

Do not add purpose language to routine operations.

Do not manufacture design rationale when the original reason is unknown.

## Connectives

Use natural causal connectives such as "since", "because", "so" and "which is why".

Do not repeat "since" within the same paragraph.

"However", "hence" and "therefore" are permitted when the logical relationship warrants them.

Avoid repetitive transitions between paragraphs.

Do not begin every section with a connective.

## Punctuation

Do not use em dashes or en dashes. Use commas, periods, colons or parentheses.

Do not use arrows in prose. Arrows are permitted only when they are part of executable code, paths, signal names or formal notation.

Semicolons are permitted but should be rare.

Do not use emojis or hashtags.

Do not apply bold formatting within a sentence for emphasis.

Use straight quotation marks and apostrophes in source text.

Preserve punctuation that belongs to programming languages, protocols, shell expressions and technical identifiers.

## Forbidden constructions

Do not use the following constructions or close variants:

- "Why this matters"
- "The key insight"
- "Here's the thing"
- "Let's dive in"
- "At its core"
- "The interesting part is"
- "Not just X, but Y"
- "X isn't about A. It's about B."
- "Building on this"
- "Leveraging X"
- "As we move forward"
- "This demonstrates the power of"
- "This ensures a seamless experience"
- "In conclusion"

Do not use headings phrased as rhetorical questions.

Do not create standalone rhetorical sentences for emphasis.

Avoid tricolons written for rhythm.

Avoid participial introductions that delay the actual subject.

Avoid chains of definitional statements.

Do not end a document with a broad statement about the importance or future potential of the work.

Do not write bullet lists where every entry starts with a bolded label and a colon.

Numbered lists are for procedures, ordered dependencies and genuine rankings, not arbitrary collections.

Do not announce the structure of the document before presenting it.

## Forbidden vocabulary

Avoid the following words and expressions in authored prose:

delve, leverage as a verb, harness, unlock, seamless, robust unless quantified, powerful, elegant, cutting-edge, state-of-the-art unless citing a paper, groundbreaking, game-changer, revolutionize, landscape, realm, tapestry, journey, deep dive, unveil, illuminate, showcase, empower, streamline, holistic, comprehensive unless coverage is exhaustive, crucial, vital, pivotal, essential unless failure is otherwise unavoidable, significantly without a number, dramatically, effortlessly, simply put, in essence, ultimately, moreover, furthermore, notably, importantly, that said, it is worth noting, it should be noted, fascinating, surprisingly unless the surprise is explained, beautiful when describing code.

Also avoid vague engineering claims such as "production-grade", "enterprise-ready", "battle-tested", "fully automated", "secure by design" and "ready for production" unless the exact claim is established by evidence.

These restrictions apply to authored language, not literal identifiers, dependencies, external quotations or established names that must be preserved.

## Evidence

Prefer actual evidence to descriptions of evidence.

Use exact commands, exit codes, test names, migration versions, observed statuses, log excerpts and measurements.

Separate measured results, estimates, assumptions and expected behavior.

Do not invent performance numbers, test counts, passing results, software versions, API behavior, infrastructure configuration or citations.

If a required value is missing, leave a clearly marked gap or state that it has not been verified.

Do not turn an attempted test into a passing test.

Do not claim that functionality works in production because unit tests passed.

Do not claim that a GitHub Actions workflow succeeded because the corresponding local script succeeded.

Do not claim that a deployment completed because an image was built or transferred.

An implementation, a local test, a CI run and a production observation are different kinds of evidence. Preserve their distinctions.

When reporting a test, identify the executed command and its actual result when that information is available.

When reporting a failure, include the relevant error and the conditions that produced it.

Do not invent a cause merely because the symptoms suggest one.

## Repository documentation

Public documentation is maintained under `docs/`.

Each public document has a distinct responsibility.

- `AUTHENTICATION.md` describes authentication modes, identity verification, sessions and authorization boundaries.
- `DEPLOYMENT.md` describes production architecture, infrastructure, configuration and deployment prerequisites.
- `RANKING_POLICY.md` describes implemented ranking behavior, eligibility and filtering semantics.
- `RELEASE_OPERATIONS.md` describes release procedures, migration controls, backups, deployment verification and recovery.
- `SOURCE_ONBOARDING.md` describes source adapters, configuration, ingestion contracts and validation.

Do not repeat the same operational procedure across multiple documents. Keep the detailed procedure in its owning document and reference it where necessary.

Verify claims against the current source code and configuration.

Do not document a proposed feature as implemented.

Do not present a disabled capability as active.

Do not confuse successful local tests with deployment verification.

Do not preserve outdated documentation solely because it existed previously.

Correct factual inconsistencies without introducing unrelated product changes.

Avoid references to agent sessions, prompts, implementation mandates and internal development records.

The repository's public documentation should remain useful without access to private conversations or agent traces.

## Internal documentation and tracelogs

`docs/implementation/` is internal working material and is excluded from public version control by the repository owner.

Do not reference it as a required source from public documentation.

Do not move its session records, planning notes or intermediate reports into public documentation.

The `private/tracelog/` directory contains development records.

Write factual records of changes, observations, test execution and unresolved conditions.

Use existing naming conventions and directory organization.

Do not write a conversation transcript.

Do not describe routine file edits as milestones.

Do not introduce artificial progress stages or achievement language.

If a decision was made by the repository owner, attribute it to the owner. Do not imply that an agent made the decision.

Keep historical statements distinguishable from the current state of the repository.

## Source code comments

These rules apply to comments and docstrings in Rust, TypeScript, JavaScript, Python, SQL, Bash, YAML and other source files.

Write comments only when they add information that is not sufficiently clear from the code.

Good reasons for a comment include:

- An invariant that must remain true.
- A non-obvious security constraint.
- A concurrency or ordering requirement.
- A compatibility requirement.
- An external API limitation.
- A deliberate exception to expected behavior.
- A failure mode that is not apparent locally.
- A design tradeoff that prevents a simpler implementation.

Do not narrate ordinary operations.

Do not explain obvious assignments, loops, imports, function calls or syntax.

Do not write comments directed at an AI agent.

Do not leave debugging narratives or descriptions of how a fix was discovered.

Do not use comments to compensate for misleading identifiers or unnecessarily complicated control flow.

Retain existing copyright, license and legally required notices.

Preserve documentation comments that form part of a public API contract.

## Operational procedures

Deployment and recovery documentation must be executable and precise.

State prerequisites before commands that depend on them.

Identify whether each command is read-only or mutates state when the distinction affects operational safety.

Distinguish commands run locally, in GitHub Actions, on the production VM and inside containers.

Use exact environment variable names and expected exit codes.

Explain nonzero exit codes that represent intentional outcomes rather than failures.

Do not replace an explicit safety check with an assurance that the procedure is safe.

Do not omit rollback limitations or data recovery requirements.

For database migrations, distinguish application image rollback from database restoration.

Do not describe a schema-changing deployment as automatically reversible unless a tested mechanism provides that guarantee.

Preserve explicit owner authorization requirements for production changes.

Do not suggest bypassing deployment gates, authentication checks or verified backup requirements to make a procedure shorter.

## Opportunity ingestion and ranking

Documentation about opportunity sources must reflect the actual ingestion pipeline.

Preserve distinctions between configured sources, supported adapter types, successfully refreshed sources and usable opportunities.

Do not infer source coverage from the existence of an adapter.

Document normalization, deduplication, source health and completeness according to the current implementation.

Ranking documentation must distinguish eligibility rules, exclusion filters, scoring inputs and presentation order.

Do not present ranking scores as probabilities unless they are mathematically defined and calibrated as such.

Do not invent weights, thresholds or ranking factors.

Do not describe an application as submitted or confirmed merely because an automation step was attempted.

Preserve the distinction between owner-reported application status and provider-confirmed submission.

Keep owner authorization requirements explicit when discussing automated applications.

## Security and privacy language

Describe security through enforced boundaries and observable behavior.

State what a control checks, where it runs and which conditions it rejects.

Do not claim that data is secure merely because it is encrypted, authenticated or stored privately.

Distinguish authentication from authorization.

Distinguish a configured secret from a credential exposed to a particular process.

Do not include real credentials, access tokens, private CV contents, session cookies or personal application data in documentation examples.

Use synthetic fixtures and placeholders where necessary.

A placeholder must be clearly distinguishable from a working value.

Do not replace a documented security limitation with reassuring language.

## Commit messages and pull requests

Commit subjects should be short and concrete.

Use sentence case without a trailing period.

Do not use Conventional Commit prefixes.

Describe the change rather than the agent's activity.

Avoid vague subjects such as "improve code quality", "implement enhancements" or "final fixes".

Pull request descriptions should identify the changed behavior, the reason for the change, the tests performed and any remaining operational restriction.

Do not include session histories, self-evaluation or promotional language.

Do not claim that a test was performed unless it was actually executed.

The repository owner alone performs Git writes. Writing this documentation does not authorize an agent to stage, commit, push or otherwise modify Git history.

## Markdown source

Write every prose paragraph on a single physical source line.

Do not hard-wrap paragraphs to fit an editor width, terminal width, formatter margin or arbitrary column limit.

A paragraph must remain one source line even when its rendered length exceeds eighty, one hundred or one hundred and twenty characters.

This applies to:

- Markdown documentation.
- README files.
- Private tracelogs.
- Issue and pull request descriptions.
- Commit message bodies.
- Source code comments.
- Docstrings.
- Shell script comments.
- YAML comments.
- SQL comments.
- Configuration comments.
- Agent-generated written records.

For bullet lists, each item must keep its prose on one physical line unless it contains a genuine nested block or executable example.

For comments, keep the entire prose statement on one physical line rather than splitting it into consecutive comment lines to satisfy a column limit.

Executable code, syntactically required multiline constructs, Markdown tables, fenced code blocks and genuinely structured data are exempt.

Do not join separate logical paragraphs into one line.

Do not disable or violate language syntax to enforce this rule.

When formatting tools impose line wrapping, preserve this contract for prose that those tools do not syntactically control.

Do not introduce artificial left or right margins into prose.

## Before returning

Review all authored or edited text against this contract.

Remove em dashes, en dashes, decorative arrows, forbidden vocabulary, title case headings and unnecessary bold emphasis.

Remove stock AI phrases and agent-oriented narration from public documentation.

Check for artificial line wrapping in every prose paragraph and comment.

Verify that factual statements match the source code, configuration or observed evidence.

Check that conditions, restrictions and failure behavior were not lost during editing.

Remove paragraphs that repeat information without adding a fact, constraint or procedure.

Verify that procedures still contain the necessary prerequisites, commands and expected results.

Preserve genuinely unresolved limitations.

Check the final paragraphs and comments with the same care as the opening sections.

Do not append a generic summary.

## Highest-priority formatting rule

EVERY PROSE PARAGRAPH MUST OCCUPY ONE PHYSICAL SOURCE LINE.

DO NOT HARD-WRAP TEXT.

DO NOT ADD ARTIFICIAL MARGINS.

THIS APPLIES TO DOCUMENTATION, MARKDOWN, PRIVATE TRACELOGS, CODE COMMENTS, SCRIPT COMMENTS, CONFIGURATION COMMENTS AND ALL OTHER AUTHORED PROSE.

EXECUTABLE CODE AND SYNTAX-REQUIRED MULTILINE STRUCTURES ARE EXEMPT.

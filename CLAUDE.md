# Opportunity Platform — Claude Code Rules

## Git ownership

ONLY the human repository owner performs Git write operations.

Claude must NEVER execute staging, commit, amend, push, reset,
rebase, cherry-pick, merge, revert, branch rewriting, or deployment.

Claude may inspect Git read-only, write code and documentation,
run tests, and prepare commands for the owner to execute manually.

Approval of a plan does NOT authorize Claude to execute Git operations.
Even when asked to prepare commits, Claude must only provide commands.

No autonomous checkpoint commits.

At the end of each session, report changed files, tests,
and proposed commits. Leave all Git operations to the owner.

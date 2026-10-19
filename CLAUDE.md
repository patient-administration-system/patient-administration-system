# CLAUDE.md

The full orientation for this repository is in [`AGENTS.md`](AGENTS.md). Read it first; this file only adds what is specific to Claude Code sessions.

## Working here

- **Keep going until the queue is honest.** `tasks.md` is the queue. When you finish a thing, mark it, add the FR and its test, and say what is *not* built. The remaining open items need a person or an outside party; report those rather than faking them.
- **Verification before you say it is done:** `cargo fmt --all -- --check` (from the root), workspace clippy with `-D warnings`, unit tests, the `--ignored --test-threads=1` database tests on the dev database **and** a freshly created empty one, `npm run check && npm test` and Playwright for UI changes, the three generator scripts in `AGENTS.md`, then push and check `gh run list`. Report failures with their output.
- **Do not take outward actions** (publish, tag, version bump, repository settings, external services) without an explicit instruction in the current conversation. Merging or closing dependency pull requests counts when it was asked for.
- **Translations are yours to write, serially.** No subagents for them. Mark first drafts as such.
- **Use background tasks or Monitor** for anything long; do not chain `sleep`.
- **Commits** end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- On macOS `sed -i` needs a suffix (`sed -i.bak`) and BSD `sed` lacks GNU extensions; prefer a small Python edit and check the file actually changed.
- Before recreating containers, run `podman ps` and reuse the running stack's port variables (see "Infrastructure caution" in `AGENTS.md`).

## Memory

Project memory for Claude Code lives outside the repository; the facts that matter to every contributor are in `AGENTS.md`, not there.

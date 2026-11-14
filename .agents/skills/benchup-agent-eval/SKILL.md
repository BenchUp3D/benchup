---
name: benchup-agent-eval
description: Run a round of the agent evaluation of the benchup CLI and MCP tools (crates/benchup-cli/evals) — one subagent per case, a results table compared with earlier rounds, and findings sorted into tool bugs, guide gaps and core limits. Use when asked to evaluate, test or "run the cases/battery/round" for the benchup CLI, MCP or agent guide, or after changing agent tools, their descriptions or docs/agent-guide.md.
---

# Agent evaluation round

The cases, rules and earlier results are in `crates/benchup-cli/evals/` (read its README.md first). A round spends many tokens: run it only when the user asked for it, and say how many agents it will start.

1. **Regression first.** `cargo test -p benchup-cli --test evals`. If it fails, stop and report: a reference solution no longer applies, which is a regression or a reference to update.
2. **Build and prepare.** `cargo build --release -p benchup-cli`, then `python3 crates/benchup-cli/evals/setup.py --round N` (N = the last round in `results/` plus one; `--cases 01,13` for a subset). It prints each case's folder and prompt file.
3. **Launch one subagent per case, in parallel, in the background.** Give each the content of its prompt file verbatim, with nothing added: no hints, no knowledge of the code, no expectations. The agents must learn the tools only from what the tools say.
4. **Check the results yourself; do not trust the reports alone.** For each case, run `benchup call <file> check_model` and `benchup render <file> --view iso` (look at the image) on the file the agent left, and compare with the request.
5. **Write `crates/benchup-cli/evals/results/round-N.md`:**
   - a table per case: commands (errors), result ✅ / ⚠️ / ❌ checked in step 4, and the same columns from the previous round;
   - which of each case's `focus` tools the agent actually used, and where it fell back to a workaround;
   - the findings, sorted into: tool bugs and unclear errors or descriptions (fix in `benchup-tools`); guide gaps or mistakes (`docs/agent-guide.md`); core limits (an issue, with a minimal reproduction); missing features.
6. **Report to the user** with the table and the proposed fixes, and wait before changing code. After fixing, update reference batches if a tool changed on purpose, and add a reference for any new case an agent solved well.

Keep the cases comparable across rounds: do not edit a case's request or seed; add a new case instead.

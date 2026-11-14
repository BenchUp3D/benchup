# Agent evaluation of the `benchup` tools

The CLI and MCP tools are meant to be used by coding agents that have never seen them. These cases check that: realistic requests from a woodworker (in Spanish, as users write them), each solved by an agent that knows only what the tool tells it (`benchup --help`, `benchup guide`, the tool schemas and its errors).

There are two layers:

1. **Regression, on every change** — `cargo test -p benchup-cli --test evals`. Replays the reference solution of each case (`reference/<case>/*.jsonl`, batches that agents or maintainers got right) with the real binary and checks the result with `check_model`. It needs no agent and takes a few seconds. When a tool changes on purpose, update the reference batch.
2. **Agent rounds, on demand** — before a release, or after changing tools, descriptions or the [agent guide](../../../docs/agent-guide.md). One agent per case, run in parallel; each one reports what it did, how many commands it needed and where it got stuck. Not in CI: it costs tokens and the results vary from run to run. To run one:

   ```sh
   cargo build --release -p benchup-cli
   python3 crates/benchup-cli/evals/setup.py --round 5            # or --cases 01,13
   # give each agent the prompt in /tmp/benchup-eval5/prompts/<case>.md, verbatim
   ```

## Files

- `cases.json` — the cases: request, seed project, mode (`cli`, or `mcp` through `mcp_call.py`), the tools it is expected to exercise, and the reference solution with what `check_model` must say about it.
- `prompt.md` — the rules every agent gets, and the report it must return.
- `setup.py` — prepares a round in `/tmp/benchup-eval<round>/` (binary, one folder per case with its seed, prompts). It never builds.
- `mcp_call.py` — a minimal MCP client over stdio, for the MCP cases.
- `fixtures/` — seed projects. `estanteria.bup` is a 1000 × 1800 mm bookcase with two columns of five housed shelves whose ten joints are reversed on purpose (agents should notice and offer `flip_joint`).
- `reference/` — reference batches, per case.
- `results/` — one file per round: the table of commands (errors) per case compared with earlier rounds, and the findings, sorted into tool bugs, guide gaps, core limits and missing features.

## Adding a case

Add it to `cases.json` with a request a real user would write, without naming tools. Give it a reference batch when the result can be checked with `check_model`; keep the request achievable with the current core (a case that hits a core limit belongs in an issue). Cases that only make sense with an agent (judgement, visual checks, a mode such as MCP) can go without a reference.

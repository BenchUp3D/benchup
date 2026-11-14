#!/usr/bin/env python3
"""Prepare a round of the agent evaluation (see README.md).

  python3 crates/benchup-cli/evals/setup.py --round 5 [--cases 01,13] [--binary target/release/benchup]

Creates /tmp/benchup-eval<round>/ with the binary in bin/, one folder per
case with its seed project, and prompts/<case>.md with the exact prompt to
give each agent. Prints a JSON list of {id, dir, prompt}. Run it from the
repository root; it never builds anything.
"""
import argparse, json, os, shutil, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CLI_ACCESS = ("- The `benchup` CLI is on that PATH. Learn it ONLY from `benchup --help`, what the help points you to "
              "(e.g. `benchup guide`, `benchup tools`, `benchup tools <name>`) and the outputs/errors it gives you.")
MCP_ACCESS = ("- You reach the project ONLY through BenchUp's MCP server, as an MCP client would. Use the client "
              "`python3 {bin}/mcp_call.py FILE list` (tools and schemas), `... FILE call TOOL 'JSON'` and "
              "`... FILE render VIEW OUT.png`. Start with the server's get_guide tool. Do NOT run the `benchup` "
              "command directly.")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--round", required=True, type=int)
    parser.add_argument("--cases", help="comma-separated case number prefixes, e.g. 01,13 (default: all)")
    parser.add_argument("--binary", default=str(ROOT / "target/release/benchup"))
    parser.add_argument("--out", help="default /tmp/benchup-eval<round>")
    args = parser.parse_args()
    binary = Path(args.binary)
    if not binary.is_file():
        sys.exit(f"no benchup binary at {binary}; build it with `cargo build --release -p benchup-cli` first")
    manifest = json.loads((HERE / "cases.json").read_text())
    wanted = args.cases.split(",") if args.cases else None
    cases = [c for c in manifest["cases"] if not wanted or any(c["id"].startswith(w) for w in wanted)]
    out = Path(args.out or f"/tmp/benchup-eval{args.round}")
    if out.exists():
        shutil.rmtree(out)
    (out / "bin").mkdir(parents=True)
    (out / "prompts").mkdir()
    shutil.copy2(binary, out / "bin/benchup")
    shutil.copy2(HERE / "mcp_call.py", out / "bin/mcp_call.py")
    template = (HERE / "prompt.md").read_text()
    listing = []
    for case in cases:
        folder = out / case["id"]
        folder.mkdir()
        seed = manifest["seeds"][case["seed"]]
        if seed:
            shutil.copy2(ROOT / seed, folder / case["file"])
        mcp = case["mode"] == "mcp"
        prompt = template.format(
            request=case["request"], dir=folder, bin=out / "bin", root=out.parent / "benchup-eval*",
            access=(MCP_ACCESS if mcp else CLI_ACCESS).format(bin=out / "bin"),
            unit="MCP tool calls" if mcp else "benchup commands",
        )
        if not seed:
            prompt = prompt.replace("Work only there.", "Work only there. The project does not exist yet; create it there.")
        path = out / "prompts" / f"{case['id']}.md"
        path.write_text(prompt)
        listing.append({"id": case["id"], "title": case["title"], "dir": str(folder), "prompt": str(path)})
    print(json.dumps(listing, indent=1, ensure_ascii=False))


if __name__ == "__main__":
    main()

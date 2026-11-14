#!/usr/bin/env python3
"""Minimal MCP client for the evaluation: talks JSON-RPC over stdio to
`benchup mcp FILE`, the way an MCP-capable agent would.

  mcp_call.py FILE list                      tools with their descriptions and schemas
  mcp_call.py FILE call TOOL '{"json": 1}'   call a tool, print its result
  mcp_call.py FILE render VIEW OUT.png       render_view: save the image, print what it reports
"""
import base64, json, subprocess, sys


def session(path, requests):
    lines = [{"jsonrpc": "2.0", "id": 0, "method": "initialize",
              "params": {"protocolVersion": "2025-06-18", "clientInfo": {"name": "eval"}}}]
    lines += [{"jsonrpc": "2.0", "id": i + 1, **r} for i, r in enumerate(requests)]
    text = "".join(json.dumps(line) + "\n" for line in lines)
    out = subprocess.run(["benchup", "mcp", path], input=text, capture_output=True, text=True)
    replies = {m["id"]: m for m in map(json.loads, out.stdout.splitlines()) if "id" in m}
    return [replies.get(i + 1) for i in range(len(requests))]


def main(argv):
    if len(argv) < 3:
        sys.exit(__doc__)
    path, command = argv[1], argv[2]
    if command == "list":
        [reply] = session(path, [{"method": "tools/list"}])
        print(json.dumps(reply["result"]["tools"], indent=1, ensure_ascii=False))
        return 0
    if command == "call" and len(argv) in (4, 5):
        arguments = json.loads(argv[4]) if len(argv) == 5 else {}
        [reply] = session(path, [{"method": "tools/call", "params": {"name": argv[3], "arguments": arguments}}])
        if "error" in reply:
            print(json.dumps(reply["error"], indent=1))
            return 2
        result = reply["result"]
        print(json.dumps(result.get("structuredContent") or result["content"][0]["text"], indent=1, ensure_ascii=False))
        return 1 if result.get("isError") else 0
    if command == "render" and len(argv) == 5:
        [reply] = session(path, [{"method": "tools/call", "params": {"name": "render_view", "arguments": {"view": argv[3]}}}])
        result = reply["result"]
        for part in result["content"]:
            if part["type"] == "image":
                open(argv[4], "wb").write(base64.b64decode(part["data"]))
            else:
                print(part["text"])
        return 1 if result.get("isError") else 0
    sys.exit(__doc__)


if __name__ == "__main__":
    sys.exit(main(sys.argv))

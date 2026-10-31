#!/usr/bin/env python3
"""Drive the reelsmith MCP server over stdio without an AI host (stdlib only).

Useful for smoke tests, evaluations and debugging a tool call exactly as a host would send it.

usage:
  mcp_call.py BINARY --list                              list the tools (name + first line of the description)
  mcp_call.py BINARY TOOL '{"arg": "value"}'             call one tool, print its structured result
  mcp_call.py BINARY --steps steps.json                  call several tools in order (a JSON list of
                                                         {"tool": NAME, "arguments": {...}})
options:
  --transcript FILE   write every request / response (with timings) to FILE as JSON
  --wait-render       when `render` answers with a job_id, poll `get_status` until the job is done

The server's stderr (its logs) goes to BINARY's stderr. Exit code: 0 ok, 1 a tool returned isError, 2 usage.
"""
import json
import subprocess
import sys
import time


def main(argv):
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    binary, rest = argv[1], argv[2:]
    transcript_path = None
    wait_render = False
    args = []
    i = 0
    while i < len(rest):
        if rest[i] == "--transcript":
            transcript_path = rest[i + 1]
            i += 2
        elif rest[i] == "--wait-render":
            wait_render = True
            i += 1
        else:
            args.append(rest[i])
            i += 1

    proc = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
    transcript = []
    next_id = [0]

    def send(method, params=None, notify=False):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        if not notify:
            next_id[0] += 1
            msg["id"] = next_id[0]
        proc.stdin.write(json.dumps(msg) + "\n")
        proc.stdin.flush()
        if notify:
            transcript.append({"notify": msg})
            return None
        t0 = time.time()
        while True:
            line = proc.stdout.readline()
            if not line:
                raise SystemExit(f"the server closed stdout while waiting for {method}")
            resp = json.loads(line)
            if resp.get("id") == msg["id"]:
                transcript.append({"request": msg, "response": resp, "seconds": round(time.time() - t0, 2)})
                return resp
            transcript.append({"server_message": resp})

    def call(name, arguments):
        resp = send("tools/call", {"name": name, "arguments": arguments})
        if "error" in resp:
            raise SystemExit(f"{name}: JSON-RPC error {json.dumps(resp['error'])}")
        res = resp.get("result", {})
        if res.get("isError"):
            text = " ".join(c.get("text", "") for c in res.get("content", []))
            print(f"{name} failed: {text}", file=sys.stderr)
            return None
        if "structuredContent" in res:
            return res["structuredContent"]
        text = "".join(c.get("text", "") for c in res.get("content", []))
        try:
            return json.loads(text)
        except ValueError:
            return text

    def call_and_wait(name, arguments):
        out = call(name, arguments)
        while wait_render and name in ("render", "get_status") and isinstance(out, dict) and out.get("state") == "running":
            out = call("get_status", {"job_id": out["job_id"], "wait_seconds": 30})
        return out

    status = 0
    try:
        send("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                            "clientInfo": {"name": "mcp_call.py", "version": "1"}})
        send("notifications/initialized", notify=True)
        if args == ["--list"]:
            tools = send("tools/list")["result"]["tools"]
            for t in tools:
                print(f"{t['name']}: {t.get('description', '').strip().splitlines()[0] if t.get('description') else ''}")
        elif len(args) == 2 and args[0] == "--steps":
            with open(args[1]) as f:
                steps = json.load(f)
            for step in steps:
                out = call_and_wait(step["tool"], step.get("arguments", {}))
                print(json.dumps({"tool": step["tool"], "result": out}, indent=1, ensure_ascii=False))
                if out is None:
                    status = 1
                    break
        elif len(args) in (1, 2):
            out = call_and_wait(args[0], json.loads(args[1]) if len(args) == 2 else {})
            print(json.dumps(out, indent=1, ensure_ascii=False))
            status = 0 if out is not None else 1
        else:
            print(__doc__, file=sys.stderr)
            status = 2
    finally:
        proc.stdin.close()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        if transcript_path:
            with open(transcript_path, "w") as f:
                json.dump(transcript, f, indent=1, ensure_ascii=False)
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv))

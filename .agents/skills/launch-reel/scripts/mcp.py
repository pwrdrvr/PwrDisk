# One MCP session over `pwrdisk mcp`, running steps from a JSON file:
#   ["tool", name, args] | ["sh", cmd] | ["sleep", s] | ["remove", mode] | ["mark", name]
# "remove" previews, then removes, accepting the confirmation form. That is
# only acceptable because this runs in the lab guest against a contrived
# fixture: never point it at a person's data or a window on a real desktop.
import json, subprocess, sys, os, time, re
steps = json.load(open(sys.argv[1]))
p = subprocess.Popen([os.environ["PWRDISK"], "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
n = 0
def write(msg): p.stdin.write(json.dumps(msg) + "\n"); p.stdin.flush()
def call(method, params):
    global n
    n += 1; my = n; write({"jsonrpc": "2.0", "id": my, "method": method, "params": params})
    while True:
        line = p.stdout.readline()
        if not line: return None
        r = json.loads(line)
        if r.get("method") == "elicitation/create" and "id" in r:
            print("<< elicitation:", r["params"].get("message", "")[:300], flush=True)
            write({"jsonrpc": "2.0", "id": r["id"], "result": {"action": "accept", "content": {"confirmed": True}}})
            continue
        if r.get("id") == my and "method" not in r: return r
def tool(name, args):
    r = call("tools/call", {"name": name, "arguments": args}) or {}
    text = "".join(c.get("text", "") for c in r.get("result", {}).get("content", []))
    print(f"== {name} {json.dumps(args)}\n{text[:2500]}\n{r.get('error') or ''}", flush=True)
    return text
call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {"elicitation": {"form": {}}}, "clientInfo": {"name": "Claude Code", "version": "2"}})
write({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
for s in steps:
    if s[0] == "tool": tool(s[1], s[2])
    elif s[0] == "sh": subprocess.run(s[1], shell=True)
    elif s[0] == "sleep": time.sleep(s[1])
    elif s[0] == "mark":
        with open(os.environ["MARKERS"], "a") as m: m.write(f"{time.time():.3f}\t{s[1]}\n")
        print("[mark]", s[1], flush=True)
    elif s[0] == "remove":
        text = tool("pwrdisk_removal_preview", {"mode": s[1]})
        m = re.search(r"\n(\{\n.*?\n\})\n", text, re.S)
        if not m: print("no preview json", flush=True); continue
        pv = json.loads(m.group(1))
        tool("pwrdisk_remove", {"preview_id": pv["preview_id"], "mode": s[1], "paths": pv["paths"], "confirmation": pv["confirmation"]})
p.stdin.close(); p.wait(timeout=30)

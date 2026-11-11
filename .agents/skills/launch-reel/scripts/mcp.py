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
        if "method" in r and "id" in r:
            # Any other server request (ping, roots/list) must be answered,
            # or the server waits on us while we wait on it.
            ok = r["method"] == "ping"
            write({"jsonrpc": "2.0", "id": r["id"], **({"result": {}} if ok else {"error": {"code": -32601, "message": "not supported"}})})
            continue
        if r.get("id") == my and "method" not in r: return r
def tool(name, args):
    r = call("tools/call", {"name": name, "arguments": args}) or {}
    text = "".join(c.get("text", "") for c in r.get("result", {}).get("content", []))
    print(f"== {name} {json.dumps(args)}\n{text[:2500]}\n{r.get('error') or ''}", flush=True)
    return text, bool(r.get("error") or r.get("result", {}).get("isError") or not r)
def fail(why):
    # A failed removal would still be filmed as Done: stop the shoot instead.
    print("FAIL:", why, flush=True); p.kill(); sys.exit(1)
def preview_json(text):
    # The preview is pretty-printed JSON after the estimate lines; decode the
    # first object that has a preview_id rather than matching its layout.
    d = json.JSONDecoder()
    for i in [m.start() for m in re.finditer(r"\{", text)]:
        try: obj, _ = d.raw_decode(text, i)
        except ValueError: continue
        if isinstance(obj, dict) and "preview_id" in obj: return obj
    return None
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
        text, err = tool("pwrdisk_removal_preview", {"mode": s[1]})
        pv = None if err else preview_json(text)
        if not pv: fail("no removal preview")
        _, err = tool("pwrdisk_remove", {"preview_id": pv["preview_id"], "mode": s[1], "paths": pv["paths"], "confirmation": pv["confirmation"]})
        if err: fail("removal refused")
p.stdin.close(); p.wait(timeout=30)

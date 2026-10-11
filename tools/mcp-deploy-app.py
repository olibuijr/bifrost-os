#!/usr/bin/env python3
"""Build an app, push it to Midget over the HTTP MCP endpoint and open it (no ssh, no release).

usage: tools/mcp-deploy-app.py APP [--skip-build] [--no-open]      (also: bifrost pc mcp deploy-app APP)
APP is the directory name under springboard/apps, as for tools/pc-app.py. Steps: `pc-app.py pack APP`
(fast build + .aap), `dev upload` in 3 MB chunks (sha256 checked), `dev install_package`, `dev restart app:ID`
when the app is already running, then `open`. Needs MIDGET_HOST or a reachable lab address, and the MCP token
(MIDGET_MCP_TOKEN, or read from Midget over ssh).
"""
import base64, hashlib, json, os, subprocess, sys, tomllib, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import midget

CHUNK = 3 << 20


def rpc(token, name, args):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(f"http://{midget.HOST}:7420/mcp", body, {"Content-Type": "application/json", "Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=180) as r:
        res = json.load(r)["result"]
    text = "\n".join(c.get("text", "") for c in res.get("content", []))
    if res.get("isError"):
        sys.exit(f"{name} {args.get('action', '')} failed: {text}")
    return text


def main():
    argv = sys.argv[1:]
    flags = [a for a in argv if a.startswith("--")]
    rest = [a for a in argv if not a.startswith("--")]
    if len(rest) != 1:
        sys.exit(__doc__)
    app = rest[0]
    pack = [sys.executable, os.path.join(ROOT, "tools", "pc-app.py"), "pack", app] + (["--skip-build"] if "--skip-build" in flags else [])
    r = subprocess.run(pack, stdout=subprocess.PIPE, text=True)
    if r.returncode:
        sys.exit(r.returncode)
    package = r.stdout.strip().splitlines()[-1]
    manifest = tomllib.load(open(os.path.join(ROOT, "springboard", "apps", app, "manifest.toml"), "rb"))
    app_id = manifest["id"]
    token = os.environ.get("MIDGET_MCP_TOKEN") or midget.ssh("cat /var/lib/bifrost-mcp/token").strip()
    if not token:
        sys.exit("no MCP token (set MIDGET_MCP_TOKEN or check ssh midget)")
    data = open(package, "rb").read()
    dest = f"/tmp/{os.path.basename(package)}"
    for i in range(0, max(len(data), 1), CHUNK):
        out = rpc(token, "dev", {"action": "upload", "path": dest, "append": i > 0, "content_b64": base64.b64encode(data[i:i + CHUNK]).decode()})
    want = hashlib.sha256(data).hexdigest()
    if want not in out:
        sys.exit(f"sha256 mismatch: sent {want}, Midget says: {out}")
    print(f"uploaded {len(data)} bytes, sha256 ok")
    print(rpc(token, "dev", {"action": "install_package", "path": dest}))
    running = any(l.startswith(app_id + " running") for l in rpc(token, "app", {}).splitlines())
    if running:
        print(rpc(token, "dev", {"action": "restart", "target": f"app:{app_id}"}))
    elif "--no-open" not in flags:
        print(rpc(token, "open", {"app": app_id}))


if __name__ == "__main__":
    main()

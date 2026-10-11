// Client for BifrOSt PC app control sockets (tasks/app-control.md).
//
// Local: one JSON-RPC request per connection on the app's Unix socket. Framing matches
// springboard/sdk/src/control.rs: one newline-terminated request, one reply line, then close.
//
// Remote (e.g. midget): the apps' sockets are owned by the installed account, and the owner
// check (SO_PEERCRED) rejects root. So the remote side runs a small stdlib-only relay over ssh,
// as that account, and the same JSON-RPC happens on the remote host. Node built-ins only.

import { spawn } from "node:child_process";
import { createConnection } from "node:net";
import { readdir } from "node:fs/promises";
import { join } from "node:path";

export const TIMEOUT_MS = 2000;
export const REMOTE_TIMEOUT_MS = 15_000;
export const MAX_REPLY_BYTES = 4 * 1024 * 1024;

// Reverse-DNS app ids only. Refusing anything else keeps agents from addressing arbitrary paths.
const APP_ID = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
// An ssh host alias or user@host. A leading dash would be read as an ssh option.
const HOST = /^[A-Za-z0-9_][A-Za-z0-9._@-]{0,252}$/;

export class RpcError extends Error {
	code: number | undefined;

	constructor(code: number | undefined, message: string) {
		super(message);
		this.name = "RpcError";
		this.code = code;
	}
}

export interface AppInfo {
	id: string;
	sdk_version?: number;
	pid: number;
	data: string;
}

export interface RunningApp {
	/** App id from the socket file name or data directory. */
	id: string;
	/** Present when the app answered app.info. */
	info?: AppInfo;
	/** Present when the socket did not answer (stale file or dead app). */
	error?: string;
}

/** Where to reach apps: this machine (default) or an ssh host such as `midget`. */
export interface Target {
	host?: string;
	/** Local socket directory override (tests). */
	dir?: string;
}

export function socketDir(env: NodeJS.ProcessEnv = process.env, uid: number = process.getuid?.() ?? 1000): string {
	const base = env.XDG_RUNTIME_DIR || `/run/user/${uid}`;
	return join(base, "bifrost", "apps");
}

export function socketPath(app: string, dir: string = socketDir()): string {
	checkAppId(app);
	return join(dir, `${app}.sock`);
}

function checkAppId(app: string): void {
	if (!APP_ID.test(app)) throw new RpcError(undefined, `invalid app id "${app}" (expected e.g. is.olibuijr.notes.desktop)`);
}

function checkHost(host: string): void {
	if (!HOST.test(host)) throw new RpcError(undefined, `invalid host "${host}" (expected an ssh alias such as midget)`);
}

// ---- local transport -------------------------------------------------------------------------

/** One JSON-RPC call on a fresh local connection. Resolves with `result`; rejects with RpcError. */
export function rpc(socket: string, method: string, params: unknown = {}, signal?: AbortSignal): Promise<unknown> {
	return new Promise((resolve, reject) => {
		const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) + "\n";
		const chunks: Buffer[] = [];
		let size = 0;
		let settled = false;

		const sock = createConnection({ path: socket });
		const finish = (err: Error | null, value?: unknown) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			signal?.removeEventListener("abort", onAbort);
			sock.destroy();
			if (err) reject(err);
			else resolve(value);
		};
		const timer = setTimeout(() => finish(new RpcError(undefined, `${method}: no reply within ${TIMEOUT_MS} ms`)), TIMEOUT_MS);
		const onAbort = () => finish(new RpcError(undefined, `${method}: aborted`));
		signal?.addEventListener("abort", onAbort, { once: true });

		sock.on("connect", () => {
			// The server reads up to the newline, replies, then closes. No half-close needed.
			sock.write(request);
		});
		sock.on("data", (chunk: Buffer) => {
			size += chunk.length;
			if (size > MAX_REPLY_BYTES) return finish(new RpcError(undefined, `${method}: reply exceeds ${MAX_REPLY_BYTES} bytes`));
			chunks.push(chunk);
		});
		sock.on("error", (e: NodeJS.ErrnoException) => finish(new RpcError(undefined, `no control socket at ${socket} (${e.code ?? e.message})`)));
		sock.on("end", () => {
			let reply: { result?: unknown; error?: { code?: number; message?: string } };
			try {
				reply = JSON.parse(Buffer.concat(chunks).toString("utf8"));
			} catch {
				const text = Buffer.concat(chunks).toString("utf8").slice(0, 120);
				return finish(new RpcError(undefined, `${method}: malformed reply (${size} bytes: ${JSON.stringify(text)})`));
			}
			if (reply.error) return finish(new RpcError(reply.error.code, `${method}: ${reply.error.message ?? "error"}`));
			finish(null, reply.result);
		});
	});
}

// ---- remote transport (ssh + relay) ----------------------------------------------------------

/**
 * Runs on the remote host with python3, as the installed BifrOSt account when started as root.
 * Reads one JSON request from stdin and writes one JSON reply to stdout. Stdlib only.
 * ops: "list" (app.info for every app socket) and "call" (one method on one app id).
 */
export const RELAY_PY = String.raw`
import glob, json, os, pwd, re, socket, sys

def out(obj):
    sys.stdout.write(json.dumps(obj))
    sys.exit(0)

def drop_to_account():
    if os.geteuid() != 0:
        return
    try:
        fields = dict(l.split("=", 1) for l in open("/var/lib/springboard/account").read().splitlines() if "=" in l)
    except OSError:
        out({"ok": False, "message": "no BifrOSt local account on this host"})
    name = fields.get("username", "")
    if not re.fullmatch(r"[a-z_][a-z0-9_-]*", name):
        out({"ok": False, "message": "invalid BifrOSt local account"})
    p = pwd.getpwnam(name)
    os.setgroups([])
    os.setgid(p.pw_gid)
    os.setuid(p.pw_uid)

def candidates():
    # Sandboxed SDK apps have no XDG_RUNTIME_DIR, so the SDK serves control.sock in its data directory.
    found = {}
    for path in sorted(glob.glob("/var/lib/springboard/appdata/*/control.sock")):
        found.setdefault(os.path.basename(os.path.dirname(path)), path)
    runtime = os.environ.get("XDG_RUNTIME_DIR") or "/run/bifrost/user"
    for path in sorted(glob.glob(os.path.join(runtime, "bifrost", "apps", "*.sock"))):
        found.setdefault(os.path.basename(path)[:-5], path)
    return {k: v for k, v in found.items() if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", k)}

def rpc(path, method, params):
    request = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n"
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(2)
    try:
        s.connect(path)
        s.sendall(request.encode())
        data = b""
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            data += chunk
    except OSError as e:
        return {"transport": "no control socket (" + (e.strerror or str(e)) + ")"}
    finally:
        s.close()
    try:
        return json.loads(data.decode())
    except ValueError:
        return {"transport": "malformed reply"}

req = json.loads(sys.stdin.read() or "{}")
drop_to_account()
sockets = candidates()
if req.get("op") == "list":
    apps = []
    for app_id, path in sorted(sockets.items()):
        reply = rpc(path, "app.info", {})
        if "result" in reply:
            apps.append({"id": app_id, "info": reply["result"]})
        else:
            err = reply.get("transport") or (reply.get("error") or {}).get("message", "error")
            apps.append({"id": app_id, "error": err})
    out({"ok": True, "apps": apps})
elif req.get("op") == "call":
    app = req.get("app")
    path = sockets.get(app)
    if path is None:
        out({"ok": False, "message": str(app) + ": no control socket on this host"})
    reply = rpc(path, req.get("method", ""), req.get("params", {}))
    if "transport" in reply:
        out({"ok": False, "message": str(app) + ": " + reply["transport"]})
    if "error" in reply:
        out({"ok": False, "code": reply["error"].get("code"), "message": reply["error"].get("message", "error")})
    out({"ok": True, "result": reply.get("result")})
else:
    out({"ok": False, "message": "unknown relay op"})
`;

const REMOTE_COMMAND = `python3 -c "import base64;exec(base64.b64decode('${Buffer.from(RELAY_PY).toString("base64")}'))"`;

function sshCommand(): string {
	return process.env.BIFROST_SSH || "ssh";
}

/** ssh options. Default: no prompts, bounded connect. Override with BIFROST_SSH_OPTIONS (space-separated). */
function sshOptions(): string[] {
	const raw = process.env.BIFROST_SSH_OPTIONS ?? "-o BatchMode=yes -o ConnectTimeout=5";
	return raw.split(/\s+/).filter(Boolean);
}

/** One relay request over ssh. Resolves with the relay's JSON reply, or rejects with RpcError. */
export function relay(host: string, request: Record<string, unknown>, signal?: AbortSignal): Promise<any> {
	checkHost(host);
	return new Promise((resolve, reject) => {
		const child = spawn(sshCommand(), [...sshOptions(), "-T", host, REMOTE_COMMAND], { stdio: ["pipe", "pipe", "pipe"] });
		let stdout = "";
		let stderr = "";
		let settled = false;
		const finish = (err: Error | null, value?: unknown) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			signal?.removeEventListener("abort", onAbort);
			if (err) reject(err);
			else resolve(value);
		};
		const kill = () => child.kill("SIGKILL");
		const timer = setTimeout(() => {
			kill();
			finish(new RpcError(undefined, `${host}: no answer within ${REMOTE_TIMEOUT_MS} ms`));
		}, REMOTE_TIMEOUT_MS);
		const onAbort = () => {
			kill();
			finish(new RpcError(undefined, `${host}: aborted`));
		};
		signal?.addEventListener("abort", onAbort, { once: true });

		child.stdout.on("data", (c: Buffer) => {
			stdout += c.toString("utf8");
			if (stdout.length > MAX_REPLY_BYTES) {
				kill();
				finish(new RpcError(undefined, `${host}: reply exceeds ${MAX_REPLY_BYTES} bytes`));
			}
		});
		child.stderr.on("data", (c: Buffer) => (stderr += c.toString("utf8")));
		child.on("error", (e) => finish(new RpcError(undefined, `cannot run ${sshCommand()}: ${e.message}`)));
		child.on("close", (code) => {
			if (code !== 0) {
				const tail = stderr.trim().split("\n").slice(-2).join(" ");
				return finish(new RpcError(undefined, `${host}: ssh failed (exit ${code})${tail ? `: ${tail}` : ""}`));
			}
			let reply: any;
			try {
				reply = JSON.parse(stdout);
			} catch {
				return finish(new RpcError(undefined, `${host}: malformed relay reply`));
			}
			if (reply.ok === false) {
				const err = new RpcError(reply.code, `${host}: ${reply.message ?? "relay error"}`);
				return finish(err);
			}
			finish(null, reply);
		});
		child.stdin.end(JSON.stringify(request));
	});
}

// ---- public API ------------------------------------------------------------------------------

/** Every app with a control socket on the target, with app.info where it answers. Empty when nothing runs. */
export async function listApps(target: Target = {}, signal?: AbortSignal): Promise<RunningApp[]> {
	if (target.host !== undefined) {
		const reply = await relay(target.host, { op: "list" }, signal);
		return (reply.apps ?? []) as RunningApp[];
	}
	return listLocalApps(target.dir ?? socketDir());
}

async function listLocalApps(dir: string): Promise<RunningApp[]> {
	let names: string[];
	try {
		names = await readdir(dir);
	} catch (e) {
		if ((e as NodeJS.ErrnoException).code === "ENOENT") return [];
		throw e;
	}
	const sockets = names.filter((n) => n.endsWith(".sock") && APP_ID.test(n.slice(0, -5)));
	return Promise.all(
		sockets.sort().map(async (name): Promise<RunningApp> => {
			const id = name.slice(0, -5);
			try {
				const info = (await rpc(join(dir, name), "app.info")) as AppInfo;
				return { id, info };
			} catch (e) {
				return { id, error: (e as Error).message };
			}
		}),
	);
}

/** Methods the app serves (app.methods), including the built-in app.* ones. */
export async function appMethods(app: string, target: Target = {}, signal?: AbortSignal): Promise<string[]> {
	checkAppId(app);
	const result = await appCall(app, "app.methods", {}, target, signal);
	return Array.isArray(result) ? (result as string[]) : [];
}

/** Calls one method on a running app on the target. */
export async function appCall(app: string, method: string, params: unknown, target: Target = {}, signal?: AbortSignal): Promise<unknown> {
	checkAppId(app);
	if (target.host !== undefined) {
		const reply = await relay(target.host, { op: "call", app, method, params: params ?? {} }, signal);
		return reply.result;
	}
	return rpc(socketPath(app, target.dir ?? socketDir()), method, params ?? {}, signal);
}

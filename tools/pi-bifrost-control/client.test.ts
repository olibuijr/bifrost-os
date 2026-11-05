// Run: node --test tools/pi-bifrost-control/client.test.ts
// Serves the control-socket framing from springboard/sdk/src/control.rs on a temp Unix socket.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:net";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { appCall, appMethods, listApps, rpc, RpcError, socketPath, socketDir } from "./client.ts";

/** The socket of app `id` in data root `dir`, its data directory made first: `<dir>/<id>/control.sock`. */
function sockIn(dir: string, id: string): string {
	mkdirSync(join(dir, id), { recursive: true });
	return join(dir, id, "control.sock");
}

type Handler = (req: { method: string; params: unknown }) => { result?: unknown; error?: { code: number; message: string } };

// Like the Rust server: read up to the newline, answer once, close.
function serve(path: string, handler: Handler): Promise<Server> {
	const server = createServer((conn) => {
		let data = "";
		let answered = false;
		conn.on("error", () => {});
		conn.on("data", (c) => {
			data += c.toString("utf8");
			if (answered || !data.includes("\n")) return;
			answered = true;
			const req = JSON.parse(data.slice(0, data.indexOf("\n")));
			const reply = { jsonrpc: "2.0", id: req.id, ...handler({ method: req.method, params: req.params }) };
			conn.end(JSON.stringify(reply) + "\n");
		});
	});
	return new Promise((resolve) => server.listen(path, () => resolve(server)));
}

function appHandler({ method, params }: { method: string; params: unknown }) {
	switch (method) {
		case "app.info":
			return { result: { id: "is.olibuijr.notes.desktop", sdk_version: 1, pid: 4242, data: "/tmp/x" } };
		case "app.methods":
			return { result: ["app.info", "app.methods", "notes.list", "notes.get"] };
		case "notes.list":
			return { result: [{ id: "a", title: "milk", echo: params }] };
		default:
			return { error: { code: -32601, message: "method not found" } };
	}
}

function withDir(fn: (dir: string) => Promise<void>) {
	return async () => {
		const root = mkdtempSync(join(tmpdir(), "bifrost-ctl-"));
		const dir = join(root, "appdata");
		mkdirSync(dir, { recursive: true });
		try {
			await fn(dir);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	};
}

test("socketPath is control.sock in the app's data directory and rejects paths", () => {
	const none = {} as NodeJS.ProcessEnv;
	assert.equal(socketPath("is.olibuijr.notes.desktop", "/d", none), "/d/is.olibuijr.notes.desktop/control.sock");
	for (const bad of ["../etc/passwd", "/tmp/x.sock", "a/b", "", ".hidden", "x".repeat(200)]) {
		assert.throws(() => socketPath(bad, "/d", none), RpcError, bad);
	}
});

test("BIFROST_CONTROL names the socket explicitly", () => {
	assert.equal(socketPath("is.olibuijr.notes.desktop", "/d", { BIFROST_CONTROL: "/x/c.sock" } as NodeJS.ProcessEnv), "/x/c.sock");
	assert.throws(() => socketPath("../x", "/d", { BIFROST_CONTROL: "/x/c.sock" } as NodeJS.ProcessEnv), RpcError, "the id is still checked");
});

test("socketDir honours BIFROST_SHELL_DIR and falls back to /var/lib/springboard", () => {
	assert.equal(socketDir({ BIFROST_SHELL_DIR: "/s" } as NodeJS.ProcessEnv), "/s/appdata");
	assert.equal(socketDir({} as NodeJS.ProcessEnv), "/var/lib/springboard/appdata");
});

test("rpc round-trips a call and returns result", withDir(async (dir) => {
	const sock = sockIn(dir, "is.olibuijr.notes.desktop");
	const server = await serve(sock, appHandler);
	try {
		const info = await rpc(sock, "app.info");
		assert.equal((info as { pid: number }).pid, 4242);
		assert.deepEqual(await appCall("is.olibuijr.notes.desktop", "notes.list", { query: "milk" }, { dir }), [
			{ id: "a", title: "milk", echo: { query: "milk" } },
		]);
	} finally {
		server.close();
	}
}));

test("error replies surface JSON-RPC code and message", withDir(async (dir) => {
	const sock = sockIn(dir, "a");
	const server = await serve(sock, appHandler);
	try {
		await assert.rejects(rpc(sock, "nope.nothing"), (e: unknown) => {
			assert.ok(e instanceof RpcError);
			assert.equal(e.code, -32601);
			assert.match(e.message, /method not found/);
			return true;
		});
	} finally {
		server.close();
	}
}));

test("appMethods lists what the app serves", withDir(async (dir) => {
	const server = await serve(sockIn(dir, "is.olibuijr.notes.desktop"), appHandler);
	try {
		assert.deepEqual(await appMethods("is.olibuijr.notes.desktop", { dir }), ["app.info", "app.methods", "notes.list", "notes.get"]);
	} finally {
		server.close();
	}
}));

test("listApps reports answering apps and stale sockets", withDir(async (dir) => {
	const server = await serve(sockIn(dir, "is.olibuijr.notes.desktop"), appHandler);
	try {
		// A plain file named control.sock is not a live socket: connecting fails.
		writeFileSync(sockIn(dir, "is.olibuijr.dead"), "");
		// A data directory without a socket is an app that is not serving: not listed.
		mkdirSync(join(dir, "is.olibuijr.idle"));
		writeFileSync(join(dir, "notes.txt"), "");
		const apps = await listApps({ dir });
		assert.deepEqual(apps.map((a) => a.id), ["is.olibuijr.dead", "is.olibuijr.notes.desktop"]);
		assert.equal(apps[0].info, undefined);
		assert.match(apps[0].error ?? "", /no control socket/);
		assert.equal(apps[1].info?.pid, 4242);
	} finally {
		server.close();
	}
}));

test("listApps is empty when the directory does not exist", async () => {
	assert.deepEqual(await listApps({ dir: "/nonexistent/bifrost-test/appdata" }), []);
});

test("an unreachable app gives a clear error", withDir(async (dir) => {
	await assert.rejects(appCall("is.olibuijr.gone", "app.info", {}, { dir }), /no control socket/);
}));

test("abort signal cancels a pending call", withDir(async (dir) => {
	const sock = sockIn(dir, "slow");
	// Server that never replies.
	const server = createServer(() => {});
	await new Promise<void>((resolve) => server.listen(sock, () => resolve()));
	try {
		const ac = new AbortController();
		const p = rpc(sock, "app.info", {}, ac.signal);
		setTimeout(() => ac.abort(), 20);
		await assert.rejects(p, /aborted/);
	} finally {
		server.close();
	}
}));

// ---- remote path: the real relay, run through a fake ssh that executes the command locally ----

function withFakeSsh(fn: (appdata: string) => Promise<void>) {
	return async () => {
		const root = mkdtempSync(join(tmpdir(), "bifrost-remote-"));
		const shell = join(root, "shell");
		mkdirSync(join(shell, "appdata"), { recursive: true });
		// ssh <options...> <host> <command>: run the command with sh on this machine.
		const fake = join(root, "fake-ssh");
		writeFileSync(fake, '#!/bin/sh\nfor a; do last="$a"; done\nexec sh -c "$last"\n');
		chmodSync(fake, 0o755);
		// the relay inherits the environment through the fake ssh, so it looks in <shell>/appdata
		const saved = { ssh: process.env.BIFROST_SSH, shell: process.env.BIFROST_SHELL_DIR };
		process.env.BIFROST_SSH = fake;
		process.env.BIFROST_SHELL_DIR = shell;
		try {
			await fn(join(shell, "appdata"));
		} finally {
			process.env.BIFROST_SSH = saved.ssh;
			process.env.BIFROST_SHELL_DIR = saved.shell;
			if (saved.ssh === undefined) delete process.env.BIFROST_SSH;
			if (saved.shell === undefined) delete process.env.BIFROST_SHELL_DIR;
			rmSync(root, { recursive: true, force: true });
		}
	};
}

test("remote: list and call go through the relay", withFakeSsh(async (appdata) => {
	const server = await serve(sockIn(appdata, "is.olibuijr.notes.desktop"), appHandler);
	try {
		const apps = await listApps({ host: "midget" });
		assert.deepEqual(apps.map((a) => a.id), ["is.olibuijr.notes.desktop"]);
		assert.equal(apps[0].info?.pid, 4242);
		assert.deepEqual(await appMethods("is.olibuijr.notes.desktop", { host: "midget" }), ["app.info", "app.methods", "notes.list", "notes.get"]);
		assert.deepEqual(await appCall("is.olibuijr.notes.desktop", "notes.list", { query: "milk" }, { host: "midget" }), [
			{ id: "a", title: "milk", echo: { query: "milk" } },
		]);
	} finally {
		server.close();
	}
}));

test("remote: unknown app and app errors are reported by the relay", withFakeSsh(async (appdata) => {
	const server = await serve(sockIn(appdata, "is.olibuijr.notes.desktop"), appHandler);
	try {
		await assert.rejects(appCall("is.olibuijr.gone", "app.info", {}, { host: "midget" }), /no control socket on this host/);
		await assert.rejects(appCall("is.olibuijr.notes.desktop", "nope.nothing", {}, { host: "midget" }), (e: unknown) => {
			assert.ok(e instanceof RpcError);
			assert.match(e.message, /method not found/);
			return true;
		});
	} finally {
		server.close();
	}
}));

test("remote: no apps gives an empty list", withFakeSsh(async () => {
	assert.deepEqual(await listApps({ host: "midget" }), []);
}));

test("remote: ssh failure surfaces the host and the ssh error", async () => {
	const saved = process.env.BIFROST_SSH;
	process.env.BIFROST_SSH = "/bin/false";
	try {
		await assert.rejects(listApps({ host: "midget" }), /midget: ssh failed \(exit 1\)/);
	} finally {
		if (saved === undefined) delete process.env.BIFROST_SSH;
		else process.env.BIFROST_SSH = saved;
	}
});

test("remote: hosts that look like ssh options are refused", async () => {
	for (const bad of ["-oProxyCommand=x", "a b", "", "x;rm"]) {
		await assert.rejects(listApps({ host: bad }), /invalid host/, bad);
	}
});

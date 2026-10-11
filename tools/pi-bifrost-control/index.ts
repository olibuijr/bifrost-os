// pi extension: lets an agent see and drive running BifrOSt PC apps through their JSON-RPC
// control sockets (tasks/app-control.md). Tools:
//   bifrost_apps          running apps that serve a control socket
//   bifrost_app_methods   methods one app serves
//   bifrost_app_call      one JSON-RPC call on an app
// Each takes an optional `host` (an ssh alias such as midget) to reach a remote machine.
// Command: /bifrost-apps [host] lists the same apps for the user.

import { Type } from "@earendil-works/pi-ai";
import { defineTool, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appCall, appMethods, listApps, socketDir, type Target } from "./client.ts";

const MAX_OUTPUT_CHARS = 50_000;

// Methods that change app data. The app enforces its own rules; this only makes the agent ask first.
const HOST_PARAM = Type.Optional(
	Type.String({ description: "ssh host alias of a remote BifrOSt machine, for example midget. Omit for this machine." }),
);

const WRITE_METHOD = /(^|[._-])(create|add|update|set|save|write|edit|rename|move|delete|remove|complete|import|clear|reset)([._-]|$)/i;

function asText(value: unknown): string {
	const text = JSON.stringify(value, null, 2) ?? "null";
	if (text.length <= MAX_OUTPUT_CHARS) return text;
	return `${text.slice(0, MAX_OUTPUT_CHARS)}\n… truncated (${text.length - MAX_OUTPUT_CHARS} more characters)`;
}

function errorText(e: unknown): string {
	return e instanceof Error ? e.message : String(e);
}

function targetOf(host: string | undefined): Target {
	return host ? { host } : {};
}

function where(host: string | undefined): string {
	return host ? ` on ${host}` : "";
}

export default function bifrostControl(pi: ExtensionAPI) {
	pi.registerTool(
		defineTool({
			name: "bifrost_apps",
			label: "BifrOSt apps",
			description:
				"List running BifrOSt PC apps that serve a JSON-RPC control socket: app id, pid and data directory. " +
				"Stale sockets are reported with their error. Pass host to list a remote machine over ssh (for example midget). " +
				"Use the id with bifrost_app_methods and bifrost_app_call.",
			promptSnippet: "List running BifrOSt PC apps with control sockets",
			parameters: Type.Object({ host: HOST_PARAM }),
			async execute(_toolCallId, params, signal) {
				if (signal?.aborted) throw new Error("aborted");
				const apps = await listApps(targetOf(params.host), signal);
				const dir = socketDir();
				const text = apps.length
					? apps
							.map((a) =>
								a.info
									? `${a.info.id}  pid ${a.info.pid}  data ${a.info.data}`
									: `${a.id}  (not answering: ${a.error})`,
							)
							.join("\n")
					: params.host
						? `No apps with a control socket on ${params.host}.`
						: `No apps with a control socket in ${dir}.`;
				return { content: [{ type: "text", text }], details: { host: params.host ?? null, dir: params.host ? null : dir, apps } };
			},
		}),
	);

	pi.registerTool(
		defineTool({
			name: "bifrost_app_methods",
			label: "BifrOSt app methods",
			description:
				"List the JSON-RPC methods a running BifrOSt app serves (app.info, app.methods and the app's own data methods).",
			promptSnippet: "List the control methods of a running BifrOSt app",
			parameters: Type.Object({
				app: Type.String({ description: "App id, for example is.olibuijr.notes.desktop" }),
				host: HOST_PARAM,
			}),
			async execute(_toolCallId, params, signal) {
				try {
					const methods = await appMethods(params.app, targetOf(params.host), signal);
					return {
						content: [{ type: "text", text: methods.join("\n") || "(no methods)" }],
						details: { app: params.app, host: params.host ?? null, methods },
					};
				} catch (e) {
					throw new Error(errorText(e));
				}
			},
		}),
	);

	pi.registerTool(
		defineTool({
			name: "bifrost_app_call",
			label: "BifrOSt app call",
			description:
				"Call one JSON-RPC method on a running BifrOSt app and return its result as JSON. " +
				"Use bifrost_app_methods first to learn the method names and parameters. " +
				"Methods that change data ask the user to confirm before they run.",
			promptSnippet: "Call a control method on a running BifrOSt app",
			promptGuidelines: [
				"Use bifrost_app_methods to find a method before calling bifrost_app_call.",
				"Prefer read methods (list, get, info, state) unless the user asked for a change.",
			],
			parameters: Type.Object({
				app: Type.String({ description: "App id, for example is.olibuijr.notes.desktop" }),
				method: Type.String({ description: "Method name, for example notes.list" }),
				params: Type.Optional(
					Type.Object({}, { additionalProperties: true, description: "Method parameters as a JSON object" }),
				),
				host: HOST_PARAM,
			}),
			async execute(_toolCallId, params, signal, _onUpdate, ctx) {
				if (WRITE_METHOD.test(params.method)) {
					if (!ctx.hasUI) {
						throw new Error(`${params.method} changes app data and needs user confirmation, which is unavailable in this mode`);
					}
					const ok = await ctx.ui.confirm(
						"BifrOSt app change",
						`${params.app}${where(params.host)}: ${params.method}\n${JSON.stringify(params.params ?? {}, null, 2)}`,
					);
					if (!ok) throw new Error(`${params.method} was not approved by the user`);
				}
				try {
					const result = await appCall(params.app, params.method, params.params ?? {}, targetOf(params.host), signal);
					return {
						content: [{ type: "text", text: asText(result) }],
						details: { app: params.app, host: params.host ?? null, method: params.method, result },
					};
				} catch (e) {
					throw new Error(errorText(e));
				}
			},
		}),
	);

	pi.registerCommand("bifrost-apps", {
		description: "List running BifrOSt PC apps with control sockets: /bifrost-apps [host]",
		handler: async (args, ctx) => {
			const host = args.trim() || undefined;
			const apps = await listApps(targetOf(host));
			if (apps.length === 0) {
				ctx.ui.notify(`No apps with a control socket${where(host)}`, "info");
				return;
			}
			const lines = apps.map((a) => (a.info ? `${a.info.id} (pid ${a.info.pid})` : `${a.id}: not answering`));
			ctx.ui.notify(lines.join("\n"), "info");
		},
	});
}

// TEST-ONLY fixture server. Serves the built UI (dist/) with an in-memory
// imitation of the Vesper API so screens can be rendered before the real
// server is available. It is not part of the application or its bundle.
//
//   npm run build && npm run fixture-server        (PORT defaults to 4173)
//   POST /__fixture/scenario?name=outage|healthy|empty|disconnected|readonly|config-error
//   POST /__fixture/bump    (simulate an external configuration change)
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { fixtureConfig, fixtureKinds, fixtureSnapshot, type Scenario } from "./fixtures.ts";
import type { IconEntry } from "../src/types.ts";

const root = fileURLToPath(new URL("../dist/", import.meta.url));
const port = Number(process.env.PORT ?? 4173);
const TOKEN = "fixture-csrf-token";

let config = fixtureConfig();
let scenario: Scenario | "disconnected" = "outage";
let configError: string | null = null;
let rev = 1;
const state = { acknowledged: new Set<number>(), silenced: {} as Record<string, string | null> };
const uploads: IconEntry[] = [];

const types: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".md": "text/plain; charset=utf-8",
  ".map": "application/json",
};

function send(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

async function body(req: IncomingMessage): Promise<any> {
  const chunks: Buffer[] = [];
  for await (const c of req) chunks.push(c as Buffer);
  const text = Buffer.concat(chunks).toString();
  return text && req.headers["content-type"]?.includes("json") ? JSON.parse(text) : text;
}

function bump() {
  config = { ...config, revision: `r-fixture-${++rev}` };
}

async function api(req: IncomingMessage, res: ServerResponse, url: URL) {
  const p = url.pathname;
  const m = req.method ?? "GET";
  if (m !== "GET") {
    if (req.headers["x-vesper-csrf"] !== TOKEN) return send(res, 403, { error: "Missing or invalid CSRF token." });
    if (req.headers.origin !== `http://${req.headers.host}`) return send(res, 403, { error: "Origin not allowed." });
  }
  if (p === "/api/csrf") return send(res, 200, { token: TOKEN });
  if (p === "/api/snapshot") {
    if (scenario === "disconnected") {
      req.socket.destroy();
      return;
    }
    const snap = fixtureSnapshot(config, scenario, state);
    return send(res, 200, { ...snap, config_error: configError });
  }
  if (p === "/api/config") return send(res, 200, config);
  if (p === "/api/icons" && m === "GET") {
    const catalog = JSON.parse(await readFile(join(root, "icons/catalog.json"), "utf8"));
    return send(res, 200, { icons: [...catalog.icons, ...uploads] });
  }
  if (p === "/api/icons" && m === "POST") {
    await body(req);
    const id = `upload:custom-${uploads.length + 1}.png`;
    uploads.push({ id, name: `Uploaded ${uploads.length + 1}`, url: "/icons/dashboard/linux.png" });
    return send(res, 200, { id, url: "/icons/dashboard/linux.png" });
  }
  if (p === "/api/config/reload") return send(res, 200, { message: "Configuration unchanged." });
  if (p === "/api/draft/test") {
    const { checks } = await body(req);
    const results = Object.fromEntries(
      Object.entries(checks as Record<string, any>).map(([id, c]) => [
        id,
        c.kind === "push"
          ? { healthy: false, latency_ms: 0, message: "Push checks cannot be run actively; send a heartbeat instead." }
          : String(c.url ?? c.host).includes("10.9.")
            ? { healthy: false, latency_ms: 0, message: "Destination 10.9.0.4 is outside allowed_cidrs." }
            : { healthy: true, latency_ms: 41, message: c.kind === "tcp" ? "TCP connect succeeded" : `HTTP ${c.expected_status}` },
      ]),
    );
    return send(res, 200, { results });
  }
  const t = p.match(/^\/api\/notifications\/([^/]+)\/test$/);
  if (t) return send(res, 200, { message: `${t[1]} accepted the test message (acceptance is not a read receipt)` });
  if (p === "/api/notifications/kinds") return send(res, 200, fixtureKinds);
  let r = p.match(/^\/api\/incidents\/(\d+)\/acknowledge$/);
  if (r) {
    const { acknowledged } = await body(req);
    if (acknowledged) state.acknowledged.add(Number(r[1]));
    else state.acknowledged.delete(Number(r[1]));
    return send(res, 200, {});
  }
  r = p.match(/^\/api\/services\/([^/]+)\/([^/]+)\/(recheck|silence)$/);
  if (r) {
    if (r[3] === "silence") state.silenced[`${r[1]}/${r[2]}`] = (await body(req)).until;
    return send(res, 200, {});
  }
  r = p.match(/^\/api\/(services|machines)\/([^/]+)(?:\/([^/]+))?$/);
  if (r && m === "PUT") {
    if (!config.writable) return send(res, 403, { error: "Configuration is read-only." });
    const b = await body(req);
    if (b.revision !== config.revision)
      return send(res, 409, { error: `Configuration revision ${config.revision} does not match ${b.revision}.` });
    const machines = structuredClone(config.machines);
    if (r[1] === "machines") machines[r[2]] = { ...b.machine, services: machines[r[2]]?.services ?? {} };
    else machines[r[2]].services[r[3]] = b.service;
    config = fixtureConfig(machines, `r-fixture-${++rev}`);
    return send(res, 200, { revision: config.revision, message: `Saved to machines/${r[2]}${r[3] ? `/services/${r[3]}` : ""}.yaml` });
  }
  send(res, 404, { error: "Not found." });
}

createServer(async (req, res) => {
  const url = new URL(req.url ?? "/", "http://fixture");
  try {
    if (url.pathname === "/__fixture/scenario") {
      const name = url.searchParams.get("name") ?? "outage";
      config = { ...config, writable: name !== "readonly" };
      configError = name === "config-error" ? "machines/bee/services/halogen.yaml: line 7: duplicate key `checks`" : null;
      scenario = (["readonly", "config-error"].includes(name) ? "outage" : name) as typeof scenario;
      return send(res, 200, { scenario });
    }
    if (url.pathname === "/__fixture/bump") {
      bump();
      return send(res, 200, { revision: config.revision });
    }
    if (url.pathname.startsWith("/api/")) return await api(req, res, url);
    const file = normalize(join(root, decodeURIComponent(url.pathname)));
    const target = file.startsWith(root) && (await stat(file).catch(() => null))?.isFile() ? file : join(root, "index.html");
    res.writeHead(200, { "Content-Type": types[extname(target)] ?? "application/octet-stream" });
    res.end(await readFile(target));
  } catch (e) {
    send(res, 500, { error: String(e) });
  }
}).listen(port, () => console.log(`TEST-ONLY fixture server on http://127.0.0.1:${port}`));

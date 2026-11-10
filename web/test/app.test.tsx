import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api, resetCsrfForTests } from "../src/api";
import { App } from "../src/App";
import { navigate } from "../src/router";
import type { ConfigResponse, Snapshot } from "../src/types";
import { fixtureConfig, fixtureSnapshot } from "./fixtures";

type Handler = (init: RequestInit & { url: string }) => Response | Promise<Response>;
interface Call {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: unknown;
}

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

let calls: Call[];
let routes: Record<string, Handler>;

function installFetch() {
  calls = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit = {}) => {
      const method = init.method ?? "GET";
      const headers = (init.headers ?? {}) as Record<string, string>;
      calls.push({
        method,
        url,
        headers,
        body: typeof init.body === "string" ? JSON.parse(init.body) : init.body,
      });
      const handler = routes[`${method} ${url}`];
      if (!handler) return json(404, { error: `no fixture for ${method} ${url}` });
      return handler({ ...init, url });
    }),
  );
}

const writes = () => calls.filter((c) => c.method !== "GET");

let config: ConfigResponse;
let snapshot: Snapshot;

beforeEach(() => {
  resetCsrfForTests();
  config = fixtureConfig();
  snapshot = fixtureSnapshot(config, "outage", { acknowledged: new Set(), silenced: {} });
  routes = {
    "GET /api/csrf": () => json(200, { token: "tok-1" }),
    "GET /api/snapshot": () => json(200, snapshot),
    "GET /api/config": () => json(200, config),
    "GET /api/icons": () =>
      json(200, { icons: [{ id: "jellyfin", name: "Jellyfin", url: "/icons/dashboard/jellyfin.png" }] }),
  };
  installFetch();
  vi.stubGlobal("confirm", vi.fn(() => true));
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function renderAt(path: string) {
  navigate(path, { replace: true, force: true });
  return render(<App />);
}

describe("API client", () => {
  it("sends the CSRF token on writes and refreshes it once after a 403", async () => {
    let tokens = 0;
    routes["GET /api/csrf"] = () => json(200, { token: `tok-${++tokens}` });
    let attempts = 0;
    routes["POST /api/notifications/telegram/test"] = (init) => {
      attempts++;
      const h = init.headers as Record<string, string>;
      return h["X-Vesper-CSRF"] === "tok-2" ? json(200, { message: "sent" }) : json(403, { error: "bad token" });
    };
    await expect(api.testChannel("telegram")).resolves.toEqual({ message: "sent" });
    expect(attempts).toBe(2);
    expect(tokens).toBe(2);
  });

  it("surfaces the server's safe error text", async () => {
    routes["POST /api/notifications/telegram/test"] = () => json(502, { error: "Telegram rejected the bot token (401)." });
    await expect(api.testChannel("telegram")).rejects.toThrow("Telegram rejected the bot token (401).");
  });
});

describe("overview", () => {
  it("orders machines and services by stable ID, not by health", async () => {
    snapshot = {
      ...snapshot,
      machines: [...snapshot.machines].reverse().map((m) => ({ ...m, services: [...m.services].reverse() })),
    };
    renderAt("/");
    const cards = await screen.findAllByRole("heading", { level: 3 });
    expect(cards.map((h) => h.textContent)).toEqual(["Bee", "Jelly", "Raptor"]);
    const jelly = cards[1].closest("section")!;
    const tiles = within(jelly)
      .getAllByRole("link")
      .slice(1)
      .map((a) => a.getAttribute("href"));
    expect(tiles).toEqual([
      "/services/jelly/home-assistant",
      "/services/jelly/host",
      "/services/jelly/jellyfin",
      "/services/jelly/radarr",
      "/services/jelly/sonarr",
    ]);
  });

  it("shows every machine and service without overview headings, incident rows or a site filter", async () => {
    const machine = snapshot.machines[0];
    machine.description = "Detailed host information belongs in the machine view.";
    renderAt("/");
    expect(await screen.findAllByRole("heading", { level: 3 })).toHaveLength(3);
    const main = within(screen.getByRole("main"));
    expect(main.queryByRole("heading", { name: "Your homelab" })).not.toBeInTheDocument();
    expect(main.queryByRole("heading", { name: "Machines & services" })).not.toBeInTheDocument();
    expect(main.queryByRole("heading", { name: "Incidents" })).not.toBeInTheDocument();
    expect(main.queryByRole("combobox")).not.toBeInTheDocument();
    expect(main.queryByRole("link", { name: /correlated with/ })).not.toBeInTheDocument();
    for (const machine of snapshot.machines) {
      for (const service of machine.services) {
        expect(main.getByRole("link", { name: new RegExp(`^${service.name},`) })).toHaveAttribute(
          "href", `/services/${service.id}`,
        );
      }
    }
    expect(screen.getByRole("link", { name: /^Incidents/ })).toHaveAttribute("href", "/incidents");
    expect(main.queryByText(machine.description)).not.toBeInTheDocument();
    const heading = main.getByRole("heading", { level: 3, name: machine.name });
    const host = within(heading.closest("section")!);
    expect(host.getByText(new RegExp(`${machine.services.length} services ·`))).toBeInTheDocument();
    await userEvent.setup().click(heading.closest("a")!);
    expect(await screen.findByRole("heading", { level: 1, name: machine.name })).toBeInTheDocument();
    expect(screen.getByText(new RegExp(machine.description))).toBeInTheDocument();
  });

  it("turns a lost snapshot connection into stale last-known state, never Down", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderAt("/");
    const tile = await screen.findByRole("link", { name: /^Jellyfin, Down, on Jelly$/ });
    expect(tile).toHaveClass("bad");

    routes["GET /api/snapshot"] = () => Promise.reject(new TypeError("network down"));
    await act(() => vi.advanceTimersByTimeAsync(6000));

    expect(await screen.findByText("Live connection lost")).toBeInTheDocument();
    const stale = screen.getByRole("link", { name: /^Jellyfin, Down \(last known\), on Jelly$/ });
    expect(stale).toHaveClass("stale");
    expect(document.querySelectorAll(".service-tile.bad")).toHaveLength(0);
    // Healthy services are not promoted to a failure either.
    expect(screen.getByRole("link", { name: /^Grafana, Healthy \(last known\)/ })).toHaveClass("stale");

    const incident = snapshot.incidents.find((i) => i.service_id === "jelly/jellyfin")!;
    await act(() => navigate(`/incidents/${incident.id}`));
    expect(await screen.findByRole("heading", { level: 1, name: "Jellyfin incident is open" })).toBeInTheDocument();
    expect(screen.getByText("Open (last known)").closest(".tag")).toHaveClass("stale");
    expect(screen.queryByText(/is having an outage/)).not.toBeInTheDocument();
  });
});

describe("service editor", () => {
  it("saves only the edited field and preserves advanced configuration", async () => {
    const user = userEvent.setup();
    let saved: any = null;
    routes["PUT /api/services/jelly/jellyfin"] = (init) => {
      saved = JSON.parse(init.body as string);
      return json(200, { revision: "r-next", message: "Saved machines/jelly/services/jellyfin.yaml" });
    };
    renderAt("/services/jelly/jellyfin/edit");
    const name = await screen.findByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Jellyfin Media");
    expect(screen.getByText("Unsaved changes")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save & monitor" }));

    await waitFor(() => expect(saved).not.toBeNull());
    const original = config.machines.jelly.services.jellyfin;
    expect(saved).toEqual({ revision: config.revision, service: { ...original, name: "Jellyfin Media", paused: false } });
    const put = writes().find((c) => c.method === "PUT")!;
    expect(put.headers["X-Vesper-CSRF"]).toBe("tok-1");
    expect(await screen.findByText("Saved machines/jelly/services/jellyfin.yaml")).toBeInTheDocument();
  });

  it("keeps edits after a revision conflict and retries on the latest revision", async () => {
    const user = userEvent.setup();
    const bodies: any[] = [];
    routes["PUT /api/services/jelly/jellyfin"] = (init) => {
      const b = JSON.parse(init.body as string);
      bodies.push(b);
      return b.revision === "r-other"
        ? json(200, { revision: "r-3", message: "Saved" })
        : json(409, { error: "Configuration revision changed." });
    };
    renderAt("/services/jelly/jellyfin/edit");
    const name = await screen.findByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Mine");
    // Someone else saves first.
    config = { ...fixtureConfig(), revision: "r-other" };
    await user.click(screen.getByRole("button", { name: "Save paused" }));

    expect(await screen.findByText("Save rejected: configuration changed")).toBeInTheDocument();
    expect(screen.getByLabelText("Name")).toHaveValue("Mine");
    expect(screen.getByText(/This service was not changed elsewhere/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Keep my edits on the latest revision" }));
    await user.click(screen.getByRole("button", { name: "Save paused" }));
    await waitFor(() => expect(bodies).toHaveLength(2));
    expect(bodies[1].revision).toBe("r-other");
    expect(bodies[1].service.name).toBe("Mine");
    expect(bodies[1].service.paused).toBe(true);
  });

  it("draft-tests checks through the draft endpoint only", async () => {
    const user = userEvent.setup();
    routes["POST /api/draft/test"] = (init) => {
      const { checks } = JSON.parse(init.body as string);
      const [id] = Object.keys(checks);
      return json(200, { results: { [id]: { healthy: false, latency_ms: 12, message: "HTTP 503" } } });
    };
    renderAt("/services/jelly/jellyfin/edit");
    await user.click(await screen.findByRole("button", { name: "Test this check" }));
    expect(await screen.findByText("HTTP 503")).toBeInTheDocument();
    expect(writes().map((c) => `${c.method} ${c.url}`)).toEqual(["POST /api/draft/test"]);
    const sent = writes()[0].body as any;
    expect(Object.values(sent.checks)).toEqual([config.machines.jelly.services.jellyfin.checks.web]);
    // A failed test does not block saving.
    expect(screen.getByRole("button", { name: "Save & monitor" })).toBeEnabled();
  });

  it("creates a new machine before its first service", async () => {
    const user = userEvent.setup();
    const order: string[] = [];
    routes["PUT /api/machines/nas"] = (init) => {
      const b = JSON.parse(init.body as string);
      order.push(`machine@${b.revision}`);
      expect(b.machine).toEqual({ name: "NAS", site: "", description: "", host_service: null, services: {} });
      return json(200, { revision: "r-m", message: "Saved machine" });
    };
    routes["PUT /api/services/nas/jellyfin"] = (init) => {
      const b = JSON.parse(init.body as string);
      order.push(`service@${b.revision}`);
      expect(b.service.icon).toBe("jellyfin");
      expect(b.service.checks.http.url).toBe("http://192.168.1.30:8096/health");
      return json(200, { revision: "r-s", message: "Saved service" });
    };
    renderAt("/services/new");
    await user.click(await screen.findByLabelText("New machine"));
    await user.type(screen.getByLabelText("Machine ID"), "nas");
    await user.type(screen.getByLabelText("Machine name"), "NAS");
    await user.type(screen.getByLabelText("Name"), "Jellyfin");
    await user.type(screen.getByLabelText("Service ID"), "jellyfin");
    await user.type(screen.getByLabelText("URL"), "http://192.168.1.30:8096/health");
    await user.click(screen.getByRole("button", { name: "Save & monitor" }));
    await waitFor(() => expect(order).toEqual([`machine@${config.revision}`, "service@r-m"]));
  });

  it("blocks invalid saves and points at the field", async () => {
    const user = userEvent.setup();
    renderAt("/services/new");
    await user.click(await screen.findByRole("button", { name: "Save & monitor" }));
    expect(screen.getByText(/Fix \d+ fields? before saving/)).toBeInTheDocument();
    expect(screen.getByText("Name the service")).toBeInTheDocument();
    expect(writes()).toHaveLength(0);
  });

  it("allows draft and export but not saving when configuration is read-only", async () => {
    config = { ...config, writable: false };
    renderAt("/services/jelly/jellyfin/edit");
    expect(await screen.findByText("Configuration is read-only")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save & monitor" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Save paused" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Test this check" })).toBeEnabled();
    const yaml = screen.getByLabelText("YAML export").textContent!;
    expect(yaml).toContain("env: JELLYFIN_TOKEN");
    expect(yaml).toContain("failure_threshold: 4");
    expect(yaml).not.toContain("null");
  });

  it("asks before leaving with unsaved edits", async () => {
    const user = userEvent.setup();
    const confirm = vi.fn(() => false);
    vi.stubGlobal("confirm", confirm);
    renderAt("/services/jelly/jellyfin/edit");
    await user.type(await screen.findByLabelText("Description"), " edited");
    await user.click(screen.getByRole("link", { name: "Settings" }));
    expect(confirm).toHaveBeenCalled();
    expect(location.pathname).toBe("/services/jelly/jellyfin/edit");
  });
});

describe("settings", () => {
  it("reports the real test result per channel", async () => {
    const user = userEvent.setup();
    routes["POST /api/notifications/telegram/test"] = () => json(502, { error: "Telegram: chat not found (400)." });
    renderAt("/settings");
    await user.click(await screen.findByRole("button", { name: "Send test message to telegram" }));
    expect(await screen.findByText("Telegram: chat not found (400).")).toBeInTheDocument();
    expect(screen.queryByText(/accepted the test message/)).not.toBeInTheDocument();
  });
});

describe("source safety", () => {
  it("never renders HTML strings", () => {
    const files: string[] = [];
    const walk = (d: string) =>
      readdirSync(d).forEach((f) => (statSync(join(d, f)).isDirectory() ? walk(join(d, f)) : files.push(join(d, f))));
    walk(join(__dirname, "../src"));
    for (const f of files) expect(readFileSync(f, "utf8"), f).not.toMatch(/dangerouslySetInnerHTML|innerHTML|outerHTML|insertAdjacentHTML/);
  });
});

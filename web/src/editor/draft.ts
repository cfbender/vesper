import YAML from "yaml";
import type { Check, CheckKind, ConfigResponse, IconEntry, Machine, SecretRef, Service } from "../types";

export const ID_RE = /^[A-Za-z0-9_-]+$/;

export interface HeaderRow {
  key: string;
  name: string;
  ref: SecretRef;
}

export interface DraftCheck {
  /** Local React key; stable across ID edits. */
  key: string;
  id: string;
  isNew: boolean;
  /** The full check object, including fields this editor does not show. */
  check: Check;
  headers: HeaderRow[];
}

export interface Draft {
  isNew: boolean;
  machineMode: "existing" | "new";
  machineId: string;
  newMachine: { name: string; site: string; description: string };
  serviceId: string;
  /** Every service field except checks, preserved verbatim from configuration. */
  service: Omit<Service, "checks">;
  checks: DraftCheck[];
  iconTouched: boolean;
}

let keySeq = 0;
const nextKey = () => `k${++keySeq}`;
export const newHeaderKey = nextKey;

/** Matches `Check::default()` in crates/core/src/model.rs. */
export function defaultCheck(kind: CheckKind = "http"): Check {
  return {
    kind,
    name: "",
    enabled: true,
    url: null,
    host: null,
    port: null,
    expected_status: 200,
    body_contains: null,
    headers: {},
    follow_redirects: false,
    token: null,
    expected_interval_seconds: 300,
    grace_seconds: 60,
    interval_seconds: null,
    timeout_seconds: null,
    failure_threshold: null,
    recovery_threshold: null,
    stale_after_seconds: null,
  };
}

function draftCheck(id: string, check: Check, isNew: boolean): DraftCheck {
  return {
    key: nextKey(),
    id,
    isNew,
    check,
    headers: Object.entries(check.headers ?? {}).map(([name, ref]) => ({ key: nextKey(), name, ref })),
  };
}

export function newCheck(draft: Draft, kind: CheckKind): DraftCheck {
  const taken = new Set(draft.checks.map((c) => c.id));
  let id: string = kind;
  for (let n = 2; taken.has(id); n++) id = `${kind}-${n}`;
  return draftCheck(id, defaultCheck(kind), true);
}

export function newDraft(machineId: string | null, config: ConfigResponse): Draft {
  const machines = Object.keys(config.machines).sort();
  const existing = machineId && config.machines[machineId] ? machineId : (machines[0] ?? "");
  const draft: Draft = {
    isNew: true,
    machineMode: existing ? "existing" : "new",
    machineId: existing,
    newMachine: { name: "", site: "", description: "" },
    serviceId: "",
    service: {
      name: "",
      icon: "server",
      description: "",
      paused: false,
      notifications: true,
      depends_on: [],
      maintenance: [],
    },
    checks: [],
    iconTouched: false,
  };
  draft.checks.push(newCheck(draft, "http"));
  return draft;
}

export function editDraft(machineId: string, serviceId: string, config: ConfigResponse): Draft | null {
  const raw = config.machines[machineId]?.services[serviceId];
  if (!raw) return null;
  const { checks, ...service } = structuredClone(raw);
  return {
    isNew: false,
    machineMode: "existing",
    machineId,
    newMachine: { name: "", site: "", description: "" },
    serviceId,
    service,
    checks: Object.entries(checks ?? {}).map(([id, c]) => draftCheck(id, c, false)),
    iconTouched: true,
  };
}

export function assembleCheck(c: DraftCheck): Check {
  return { ...c.check, headers: Object.fromEntries(c.headers.map((h) => [h.name, h.ref])) };
}

/** The Service sent to the server: raw fields kept, edited fields applied. */
export function draftService(d: Draft): Service {
  return { ...d.service, checks: Object.fromEntries(d.checks.map((c) => [c.id, assembleCheck(c)])) };
}

export function draftMachine(d: Draft): Machine {
  return { ...d.newMachine, host_service: null, services: {} };
}

/** Comparable form used to detect unsaved edits. */
export function draftFingerprint(d: Draft): string {
  return JSON.stringify({
    m: d.machineMode,
    id: d.machineId,
    nm: d.machineMode === "new" ? d.newMachine : null,
    s: d.serviceId,
    svc: draftService(d),
  });
}

export const refValue = (r: SecretRef | null) => (r ? ("env" in r ? r.env : r.file) : "");
export const refKind = (r: SecretRef | null): "env" | "file" => (r && "file" in r ? "file" : "env");

export type Errors = Record<string, string>;

function positiveInt(v: number | null, optional: boolean, max = Number.MAX_SAFE_INTEGER): string | null {
  if (v === null) return optional ? null : "Required";
  if (!Number.isInteger(v) || v < 1 || v > max)
    return max === Number.MAX_SAFE_INTEGER ? "Enter a whole number of at least 1" : `Enter a whole number from 1 to ${max}`;
  return null;
}

export function validateDraft(d: Draft, config: ConfigResponse): Errors {
  const e: Errors = {};
  const machine = d.machineMode === "existing" ? config.machines[d.machineId] : undefined;
  if (d.machineMode === "existing") {
    if (!machine) e.machineId = "Choose a machine";
  } else {
    if (!ID_RE.test(d.machineId)) e.machineId = "Use letters, digits, hyphens or underscores";
    else if (config.machines[d.machineId]) e.machineId = "A machine with this ID already exists";
    if (!d.newMachine.name.trim()) e.machineName = "Name the machine";
  }
  if (!ID_RE.test(d.serviceId)) e.serviceId = "Use letters, digits, hyphens or underscores";
  else if (d.isNew && machine?.services[d.serviceId]) e.serviceId = "This machine already has a service with this ID";
  if (!d.service.name.trim()) e.name = "Name the service";
  const self = `${d.machineId}/${d.serviceId}`;
  if (d.service.depends_on.includes(self)) e.depends_on = "A service cannot depend on itself";
  d.service.maintenance.forEach((w, i) => {
    const s = Date.parse(w.start);
    const t = Date.parse(w.end);
    if (Number.isNaN(s) || Number.isNaN(t)) e[`maint:${i}`] = "Enter a start and end time";
    else if (t <= s) e[`maint:${i}`] = "End must be after start";
  });
  const seen = new Set<string>();
  for (const c of d.checks) {
    const p = `check:${c.key}:`;
    if (!ID_RE.test(c.id)) e[`${p}id`] = "Use letters, digits, hyphens or underscores";
    else if (seen.has(c.id)) e[`${p}id`] = "Check IDs must be unique";
    seen.add(c.id);
    const k = c.check;
    if (k.kind === "http") {
      const url = k.url?.trim() ?? "";
      if (!url) e[`${p}url`] = "Enter a URL";
      else {
        try {
          const u = new URL(url);
          if (u.protocol !== "http:" && u.protocol !== "https:") e[`${p}url`] = "Use http:// or https://";
          else if (u.username || u.password) e[`${p}url`] = "Credentials in URLs are not allowed; use a header secret";
        } catch {
          e[`${p}url`] = "Enter a full URL such as https://host:8096/health";
        }
      }
      const st = k.expected_status;
      if (!Number.isInteger(st) || st < 100 || st > 599) e[`${p}expected_status`] = "Enter an HTTP status from 100 to 599";
      const names = new Set<string>();
      c.headers.forEach((h) => {
        if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(h.name)) e[`${p}header:${h.key}`] = "Enter a valid header name";
        else if (names.has(h.name.toLowerCase())) e[`${p}header:${h.key}`] = "Duplicate header";
        else if (!refValue(h.ref).trim()) e[`${p}header:${h.key}`] = "Enter a secret reference";
        names.add(h.name.toLowerCase());
      });
    } else if (k.kind === "tcp") {
      if (!k.host?.trim()) e[`${p}host`] = "Enter a host or IP address";
      const err = positiveInt(k.port, false, 65535);
      if (err) e[`${p}port`] = err;
    } else {
      const a = positiveInt(k.expected_interval_seconds, false);
      if (a) e[`${p}expected_interval_seconds`] = a;
      if (!Number.isInteger(k.grace_seconds) || k.grace_seconds < 0) e[`${p}grace_seconds`] = "Enter zero or more seconds";
      if (k.token && !refValue(k.token).trim()) e[`${p}token`] = "Enter a secret reference or remove the token";
    }
    for (const f of ["interval_seconds", "timeout_seconds", "failure_threshold", "recovery_threshold", "stale_after_seconds"] as const) {
      const err = positiveInt(k[f], true);
      if (err) e[`${p}${f}`] = err;
    }
  }
  return e;
}

const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

/** Local catalog icons whose ID or name matches the service name or ID. */
export function suggestIcons(icons: IconEntry[], ...terms: string[]): IconEntry[] {
  const words = terms
    .flatMap((t) => [t, ...t.split(/[\s_/-]+/)])
    .map(norm)
    .filter((w) => w.length >= 3);
  if (words.length === 0) return [];
  const scored = icons
    .filter((i) => !i.id.startsWith("upload:"))
    .map((i) => {
      const a = norm(i.id);
      const b = norm(i.name);
      let score = 0;
      for (const w of words) {
        if (a === w || b === w) score = Math.max(score, 3);
        else if (a.startsWith(w) || b.startsWith(w) || w.startsWith(a)) score = Math.max(score, 2);
        else if (a.includes(w) || b.includes(w)) score = Math.max(score, 1);
      }
      return { i, score };
    })
    .filter((x) => x.score > 0)
    .sort((x, y) => y.score - x.score || x.i.name.localeCompare(y.i.name));
  return scored.slice(0, 6).map((x) => x.i);
}

/** Exact catalog match used to pick an icon automatically for new services. */
export function exactIcon(icons: IconEntry[], name: string, id: string): IconEntry | undefined {
  const keys = [norm(name), norm(id)].filter(Boolean);
  return icons.find((i) => !i.id.startsWith("upload:") && (keys.includes(norm(i.id)) || keys.includes(norm(i.name))));
}

export function stripNulls(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(stripNulls);
  if (v && typeof v === "object")
    return Object.fromEntries(
      Object.entries(v)
        .filter(([, x]) => x !== null && x !== undefined)
        .map(([k, x]) => [k, stripNulls(x)]),
    );
  return v;
}

/** Read-only YAML of the draft service keyed by its ID; contains secret references only. */
export function exportYaml(d: Draft): string {
  return YAML.stringify({ [d.serviceId || "service"]: stripNulls(draftService(d)) }, { lineWidth: 0 });
}

export const LOGICAL_PREFIX = (m: string, s: string) => `machines/${m}/services/${s}`;

/** Files that currently define this service or parts of it (split layouts). */
export function serviceSources(config: ConfigResponse, m: string, s: string): { path: string; file: string }[] {
  const prefix = LOGICAL_PREFIX(m, s);
  return Object.entries(config.sources)
    .filter(([path]) => path === prefix || path.startsWith(`${prefix}/`))
    .map(([path, file]) => ({ path, file }))
    .sort((a, b) => a.path.localeCompare(b.path));
}

import type { CheckDefaults, CheckKind, Health } from "./types";

export type Tone = "good" | "warn" | "bad" | "stale" | "neutral";

const HEALTH: Record<Health, { label: string; tone: Tone }> = {
  healthy: { label: "Healthy", tone: "good" },
  suspect: { label: "Suspect", tone: "warn" },
  recovering: { label: "Recovering", tone: "warn" },
  down: { label: "Down", tone: "bad" },
  stale: { label: "Stale", tone: "stale" },
  unknown: { label: "Unknown", tone: "stale" },
  paused: { label: "Paused", tone: "neutral" },
};

/**
 * Health as displayed. When the browser has lost the live snapshot, every
 * status is shown as last-known and unconfirmed — never as a fresh failure.
 */
export function healthDisplay(health: Health, live: boolean): { label: string; tone: Tone } {
  const h = HEALTH[health] ?? HEALTH.unknown;
  if (live) return h;
  return { label: `${h.label} (last known)`, tone: "stale" };
}

export function kindLabel(kind: CheckKind): string {
  return kind === "http" ? "HTTP" : kind === "tcp" ? "TCP" : "Push";
}

export function duration(seconds: number): string {
  if (!Number.isFinite(seconds)) return "—";
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s`;
  if (s < 3600) return s % 60 ? `${Math.floor(s / 60)}m ${s % 60}s` : `${s / 60}m`;
  if (s < 86400) {
    const m = Math.floor((s % 3600) / 60);
    return m ? `${Math.floor(s / 3600)}h ${m}m` : `${s / 3600}h`;
  }
  const h = Math.floor((s % 86400) / 3600);
  return h ? `${Math.floor(s / 86400)}d ${h}h` : `${s / 86400}d`;
}

export function ago(iso: string | null | undefined, now: number): string {
  if (!iso) return "never";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "unknown time";
  const diff = (now - t) / 1000;
  if (diff < -5) return `in ${duration(-diff)}`;
  if (diff < 5) return "just now";
  return `${duration(diff)} ago`;
}

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const dateTimeFmt = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

/** Local wall time; includes the date unless it is today. */
export function localTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toDateString() === new Date().toDateString() ? timeFmt.format(d) : dateTimeFmt.format(d);
}

export function fullTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : dateTimeFmt.format(d);
}

export function policySummary(p: CheckDefaults, kind: CheckKind): string {
  if (kind === "push") return `Stale after ${duration(p.stale_after_seconds)}`;
  return `Every ${duration(p.interval_seconds)} · ${p.failure_threshold} failures to Down · ${p.recovery_threshold} passes to recover`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(n % 1024 ? 1 : 0)} KiB`;
  return `${(n / 1048576).toFixed(n % 1048576 ? 1 : 0)} MiB`;
}

export const byId = <T extends { id: string }>(a: T, b: T) =>
  a.id < b.id ? -1 : a.id > b.id ? 1 : 0;

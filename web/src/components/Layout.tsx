import type { ReactNode } from "react";
import { useLive } from "../data";
import { ago, localTime } from "../format";
import { Link, paths, useRoute, type Route } from "../router";
import type { ChannelView } from "../types";
import { Notice } from "./ui";

function section(route: Route): "overview" | "incidents" | "settings" | null {
  switch (route.name) {
    case "overview":
    case "machine":
    case "service":
    case "edit-service":
    case "new-service":
      return "overview";
    case "incidents":
    case "incident":
      return "incidents";
    case "settings":
      return "settings";
    default:
      return null;
  }
}

export function ConnectionNotice() {
  const { snapshot, error, live, lastSuccess, now } = useLive();
  if (live) return null;
  if (!snapshot) {
    return error ? (
      <Notice tone="stale" title="Can't reach the Vesper server" live>
        <p>{error} Retrying every few seconds. No health is shown until a snapshot arrives.</p>
      </Notice>
    ) : null;
  }
  return (
    <Notice tone="stale" title="Live connection lost" live>
      <p>
        Showing the last snapshot, received {ago(lastSuccess ? new Date(lastSuccess).toISOString() : null, now)} (
        {localTime(snapshot.generated_at)}). Current health is unknown; nothing below is a new
        failure. Reconnecting…
      </p>
    </Notice>
  );
}

export function Layout({ children }: { children: ReactNode }) {
  const route = useRoute();
  const active = section(route);
  const { snapshot, live, lastSuccess, now } = useLive();
  const openIncidents = snapshot?.incidents.filter((i) => !i.resolved_at).length ?? 0;
  const nav = (id: typeof active, to: string, label: ReactNode) => (
    <Link to={to} aria-current={active === id ? "page" : undefined}>
      {label}
    </Link>
  );
  return (
    <div className="app">
      <a className="skip" href="#main">
        Skip to content
      </a>
      <header className="bar">
        <Link to={paths.overview()} className="brand" aria-label="Vesper overview">
          <span aria-hidden="true">◒</span>vesper
        </Link>
        <nav className="nav" aria-label="Main">
          {nav("overview", paths.overview(), "Overview")}
          {nav(
            "incidents",
            paths.incidents(),
            <>
              Incidents
              {openIncidents > 0 && (
                <span className="count" aria-label={`${openIncidents} open`}>
                  {openIncidents}
                </span>
              )}
            </>,
          )}
          {nav("settings", paths.settings(), "Settings")}
        </nav>
        {snapshot?.config_writable && (
          <Link to={paths.newService()} className="btn primary add">
            <span aria-hidden="true">+</span> Add service
          </Link>
        )}
      </header>
      <main id="main" className="content">
        <div className="banners">
          <ConnectionNotice />
          {snapshot?.config_error && (
            <Notice tone="bad" title="Configuration error">
              <p className="evidence">{snapshot.config_error}</p>
              <p>
                The last valid configuration (revision <code>{short(snapshot.config_revision)}</code>)
                is still running. <Link to={paths.settings()}>Configuration details</Link>
              </p>
            </Notice>
          )}
        </div>
        {children}
      </main>
      <footer className="bottom">
        <span>
          {snapshot
            ? live
              ? `Live · updated ${ago(lastSuccess ? new Date(lastSuccess).toISOString() : null, now)}`
              : `Disconnected · snapshot from ${localTime(snapshot.generated_at)}`
            : "Connecting…"}
        </span>
        {snapshot && <span>{notificationSummary(snapshot.notifications)}</span>}
        {snapshot && (
          <span>
            Config <code>{short(snapshot.config_revision)}</code>
            {snapshot.config_writable ? "" : " · read-only"}
          </span>
        )}
      </footer>
    </div>
  );
}

export const short = (rev: string) => (rev.length > 12 ? rev.slice(0, 12) : rev);

function notificationSummary(channels: ChannelView[]): string {
  if (!channels?.length) return "Notifications: not configured";
  if (channels.length === 1) return `${channels[0].kind_name}: ${channels[0].status}`;
  const failing = channels.filter((c) => !c.status.startsWith("ok") && c.status !== "configured").length;
  return failing ? `Notifications: ${failing} of ${channels.length} channels need attention` : `Notifications: ${channels.length} channels ok`;
}

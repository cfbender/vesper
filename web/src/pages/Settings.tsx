import { useCallback, useEffect, useState } from "react";
import { api, errorText } from "../api";
import { short } from "../components/Layout";
import { Notice, PageTitle, ToneTag, useAction } from "../components/ui";
import { useLive } from "../data";
import { bytes, duration, fullTime } from "../format";
import type { ChannelView, ConfigResponse, NotificationField, NotificationKind } from "../types";

export function useConfig() {
  const [config, setConfig] = useState<ConfigResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(
    () =>
      api.config().then(
        (c) => {
          setConfig(c);
          setError(null);
          return c;
        },
        (e) => {
          setError(errorText(e));
          return null;
        },
      ),
    [],
  );
  useEffect(() => {
    load();
  }, [load]);
  return { config, error, load };
}

/** Example YAML for one channel kind: required settings, then optional ones commented out. */
export function channelExample(kind: NotificationKind): string {
  const value = (f: NotificationField) => {
    if (f.secret) return `{ file: ${kind.kind.replace(/_/g, "-")}-${f.key.replace(/_/g, "-")} }`;
    const v = f.example ?? f.default ?? (f.kind === "bool" ? "false" : f.kind === "choice" ? f.choices?.[0] : "…");
    return f.kind === "list" ? `[${v}]` : String(v);
  };
  const lines = [
    "notifications:",
    "  dashboard_url: https://vesper.example.lan",
    "  channels:",
    `    ${kind.kind.replace(/_/g, "-")}:`,
    `      kind: ${kind.kind}`,
    ...kind.fields.filter((f) => f.required).map((f) => `      ${f.key}: ${value(f)}`),
    ...kind.fields.filter((f) => !f.required).map((f) => `      # ${f.key}: ${value(f)}`),
  ];
  return lines.join("\n");
}

function ChannelRow({ channel }: { channel: ChannelView }) {
  const test = useAction<{ message: string }>();
  const ok = channel.status.startsWith("ok") || channel.status === "configured";
  const bad = channel.status.startsWith("error");
  return (
    <li className="channel">
      <div className="channel-head">
        <strong>{channel.id}</strong> <span className="note">{channel.kind_name}</span>{" "}
        <ToneTag tone={bad ? "bad" : ok ? "good" : "warn"}>{bad ? "Error" : ok ? "Ready" : "Delayed"}</ToneTag>
        <button
          className="btn"
          disabled={test.busy}
          aria-label={`Send test message to ${channel.id}`}
          onClick={() => test.run(() => api.testChannel(channel.id))}
        >
          {test.busy ? "Sending…" : "Send test message"}
        </button>
      </div>
      <p className="evidence">{channel.status}</p>
      <div aria-live="polite">
        {test.result && (
          <Notice tone="good" title="Test message accepted">
            <p className="evidence">{test.result.message}</p>
            <p className="note">Acceptance by the service is not a read receipt; check that it arrived.</p>
          </Notice>
        )}
        {test.error && (
          <Notice tone="bad" title="Test message failed">
            <p className="evidence">{test.error}</p>
          </Notice>
        )}
      </div>
    </li>
  );
}

function KindReference() {
  const [kinds, setKinds] = useState<NotificationKind[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState("telegram");
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (open && !kinds)
      api.notificationKinds().then(
        (r) => setKinds(r.kinds),
        (e) => setError(errorText(e)),
      );
  }, [open, kinds]);
  const kind = kinds?.find((k) => k.kind === selected) ?? kinds?.[0];
  return (
    <details className="samples" onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary>Channel kinds{kinds ? ` (${kinds.length})` : ""}</summary>
      {error && <Notice tone="bad" title="Could not load channel kinds">{error}</Notice>}
      {kinds && kind && (
        <>
          <div className="field">
            <label htmlFor="kind-select">Kind</label>
            <select id="kind-select" value={kind.kind} onChange={(e) => setSelected(e.target.value)}>
              {[...kinds]
                .sort((a, b) => a.name.localeCompare(b.name))
                .map((k) => (
                  <option key={k.kind} value={k.kind}>
                    {k.name.toLowerCase() === k.kind ? k.name : `${k.name} (${k.kind})`}
                  </option>
                ))}
            </select>
          </div>
          <p className="note">
            <a href={kind.docs} target="_blank" rel="noreferrer">
              {kind.name} API documentation
            </a>
            {kind.per_incident && " · Sends one alert per incident and resolves it on recovery."}
          </p>
          <pre className="block">{channelExample(kind)}</pre>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Setting</th>
                  <th>Value</th>
                  <th>Description</th>
                </tr>
              </thead>
              <tbody>
                {kind.fields.map((f) => (
                  <tr key={f.key}>
                    <td>
                      <code>{f.key}</code>
                    </td>
                    <td>
                      {f.secret ? "secret reference" : f.kind === "choice" ? f.choices?.join(" · ") : f.kind}
                      {f.required ? ", required" : f.default ? `, default ${f.default}` : ", optional"}
                    </td>
                    <td>{f.help}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </details>
  );
}

function NotificationsCard({ channels }: { channels: ChannelView[] | null }) {
  return (
    <section className="card" aria-labelledby="notify-title">
      <h2 id="notify-title">Notifications</h2>
      {channels && channels.length > 0 ? (
        <ul className="plain-list channels">
          {channels.map((c) => (
            <ChannelRow key={c.id} channel={c} />
          ))}
        </ul>
      ) : (
        <p className="note">
          {channels ? "No notification channels are configured. " : ""}Add channels under{" "}
          <code>notifications.channels</code> in your YAML. Every channel receives every incident
          notification. Credentials are secret references to environment variables or files in the
          secrets directory, never literal values.
        </p>
      )}
      <KindReference />
    </section>
  );
}

export function SettingsPage() {
  const { snapshot, refresh } = useLive();
  const { config, error, load } = useConfig();
  const reload = useAction<Record<string, unknown>>();
  const sources = config ? [...new Set(Object.values(config.sources))].sort() : [];
  const reloadMessage =
    reload.result && typeof reload.result.message === "string" ? reload.result.message : null;
  return (
    <>
      <PageTitle title="Settings" sub="Configuration lives in YAML files. This page shows what is running." />
      <section className="card" aria-labelledby="cfg-title">
        <h2 id="cfg-title">Configuration</h2>
        {error && <Notice tone="bad" title="Could not load configuration">{error}</Notice>}
        <dl className="factgrid">
          <div>
            <dt>Running revision</dt>
            <dd>
              <code>{snapshot ? short(snapshot.config_revision) : "—"}</code>
            </dd>
          </div>
          <div>
            <dt>Editing</dt>
            <dd>
              {config ? (
                config.writable ? (
                  <ToneTag tone="good">Writable</ToneTag>
                ) : (
                  <ToneTag tone="neutral">Read-only: draft, test and export only</ToneTag>
                )
              ) : (
                "—"
              )}
            </dd>
          </div>
          <div>
            <dt>Status</dt>
            <dd>
              {snapshot?.config_error ? (
                <ToneTag tone="bad">Error; last valid revision still active</ToneTag>
              ) : (
                <ToneTag tone="good">Valid</ToneTag>
              )}
            </dd>
          </div>
          <div>
            <dt>Engine started</dt>
            <dd>{fullTime(snapshot?.started_at)}</dd>
          </div>
        </dl>
        {snapshot?.config_error && <pre className="evidence block">{snapshot.config_error}</pre>}
        <div className="actions">
          <button
            className="btn"
            disabled={reload.busy}
            onClick={() =>
              reload.run(async () => {
                const r = await api.reloadConfig();
                await Promise.all([refresh(), load()]);
                return r;
              })
            }
          >
            {reload.busy ? "Reloading…" : "Reload configuration now"}
          </button>
        </div>
        <div aria-live="polite">
          {reload.result && (
            <Notice tone={snapshot?.config_error ? "bad" : "good"} title="Reload finished">
              <p>
                {reloadMessage ?? "The server re-read the configuration root."} Running revision:{" "}
                <code>{snapshot ? short(snapshot.config_revision) : "—"}</code>
                {snapshot?.config_error ? " (files have errors; last valid configuration kept)" : ""}.
              </p>
            </Notice>
          )}
          {reload.error && <Notice tone="bad" title="Reload failed">{reload.error}</Notice>}
        </div>
        {sources.length > 0 && (
          <details className="samples">
            <summary>Source files ({sources.length})</summary>
            <ul className="plain-list mono">
              {sources.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          </details>
        )}
      </section>

      <NotificationsCard channels={snapshot?.notifications ?? null} />

      {config && (
        <section className="card" aria-labelledby="policy-title">
          <h2 id="policy-title">Check defaults</h2>
          <p className="note">Inherited by every check unless it overrides them. Edit in YAML.</p>
          <dl className="factgrid">
            <div>
              <dt>Interval</dt>
              <dd>{duration(config.defaults.interval_seconds)}</dd>
            </div>
            <div>
              <dt>Timeout</dt>
              <dd>{duration(config.defaults.timeout_seconds)}</dd>
            </div>
            <div>
              <dt>Failures to Down</dt>
              <dd>{config.defaults.failure_threshold}</dd>
            </div>
            <div>
              <dt>Passes to recover</dt>
              <dd>{config.defaults.recovery_threshold}</dd>
            </div>
            <div>
              <dt>Stale after</dt>
              <dd>{duration(config.defaults.stale_after_seconds)}</dd>
            </div>
          </dl>
          <h2 id="net-title">Network policy</h2>
          <p className="note">
            Read-only. Checks may only reach allowed networks; loopback, link-local and cloud metadata
            addresses are always denied.
          </p>
          <dl className="factgrid">
            <div>
              <dt>Allowed networks</dt>
              <dd>
                {config.network.allowed_cidrs.length ? (
                  <span className="mono">{config.network.allowed_cidrs.join(", ")}</span>
                ) : (
                  <ToneTag tone="warn">None: every destination is denied</ToneTag>
                )}
              </dd>
            </div>
            <div>
              <dt>Concurrent probes</dt>
              <dd>{config.network.max_concurrent}</dd>
            </div>
            <div>
              <dt>Response limit</dt>
              <dd>{bytes(config.network.max_response_bytes)}</dd>
            </div>
            <div>
              <dt>Private CA</dt>
              <dd>{config.network.private_ca_file ? <code>{config.network.private_ca_file}</code> : "System roots only"}</dd>
            </div>
          </dl>
        </section>
      )}

      <section className="card" aria-labelledby="access-title">
        <h2 id="access-title">Access</h2>
        <p>
          Warning: Vesper has no built-in login. Authentication at your reverse proxy is optional;
          without it, anyone who can reach Vesper has full administrator access. Restrict network
          access or configure authentication. Vesper cannot verify your proxy’s authentication setup.
        </p>
      </section>

      <section className="card" aria-labelledby="credits-title">
        <h2 id="credits-title">Credits</h2>
        <p className="note">
          Service logos: Homarr Labs Dashboard Icons, Apache License 2.0 (
          <a href="/icons/dashboard/LICENSE">license</a>, <a href="/icons/dashboard/NOTICE.md">notice</a>
          ). Product names and logos are trademarks of their respective owners and imply no endorsement.
          Icons are served locally; Vesper makes no CDN requests.
        </p>
      </section>
    </>
  );
}

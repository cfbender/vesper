import { useCallback, useEffect, useState } from "react";
import { api, errorText } from "../api";
import { short } from "../components/Layout";
import { Notice, PageTitle, ToneTag, useAction } from "../components/ui";
import { useLive } from "../data";
import { bytes, duration, fullTime } from "../format";
import type { ConfigResponse } from "../types";

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

const TELEGRAM_EXAMPLE = `telegram:
  bot_token: { env: VESPER_TELEGRAM_TOKEN }
  chat_id: { file: telegram-chat-id }
  dashboard_url: https://vesper.example.lan`;

export function SettingsPage() {
  const { snapshot, refresh } = useLive();
  const { config, error, load } = useConfig();
  const reload = useAction<Record<string, unknown>>();
  const telegram = useAction<{ message: string }>();
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

      <section className="card" aria-labelledby="tg-title">
        <h2 id="tg-title">Telegram</h2>
        <dl className="factgrid">
          <div>
            <dt>Configured</dt>
            <dd>
              {config ? (
                config.telegram_configured ? (
                  <ToneTag tone="good">Yes</ToneTag>
                ) : (
                  <ToneTag tone="warn">Not configured</ToneTag>
                )
              ) : (
                "—"
              )}
            </dd>
          </div>
          <div>
            <dt>Delivery status</dt>
            <dd className="evidence">{snapshot?.telegram || "Unknown"}</dd>
          </div>
        </dl>
        {config && !config.telegram_configured && (
          <>
            <p className="note">
              Add a <code>telegram</code> block to your YAML. Credentials are secret references to
              environment variables or files in the secrets directory, never literal values.
            </p>
            <pre className="block">{TELEGRAM_EXAMPLE}</pre>
          </>
        )}
        <div className="actions">
          <button
            className="btn primary"
            disabled={telegram.busy || !config?.telegram_configured}
            onClick={() => telegram.run(() => api.testTelegram())}
          >
            {telegram.busy ? "Sending…" : "Send test message"}
          </button>
        </div>
        <div aria-live="polite">
          {telegram.result && (
            <Notice tone="good" title="Telegram accepted the test message">
              <p className="evidence">{telegram.result.message}</p>
              <p className="note">Acceptance by Telegram is not a read receipt; check the chat.</p>
            </Notice>
          )}
          {telegram.error && (
            <Notice tone="bad" title="Test message failed">
              <p className="evidence">{telegram.error}</p>
            </Notice>
          )}
        </div>
      </section>

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
          Vesper has no accounts or login. Your authentication proxy protects it, and everyone the proxy
          admits is an administrator.
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

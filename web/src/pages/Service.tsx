import { useState } from "react";
import { api } from "../api";
import {
  Crumbs,
  HealthBadge,
  History,
  Notice,
  PageTitle,
  ServiceIcon,
  takeFlash,
  useAction,
} from "../components/ui";
import { useLive } from "../data";
import { ago, duration, fullTime, kindLabel, localTime, policySummary } from "../format";
import { Link, paths } from "../router";
import type { CheckView } from "../types";

function CheckCard({ check }: { check: CheckView }) {
  const { live, now } = useLive();
  const p = check.policy;
  const overdue =
    check.last_checked !== null && now - Date.parse(check.last_checked) > p.stale_after_seconds * 1000;
  return (
    <section className="card check" aria-labelledby={`check-${check.id}`}>
      <div className="sectionline">
        <h3 id={`check-${check.id}`}>
          {check.name || check.id} <span className="pill">{kindLabel(check.kind)}</span>
        </h3>
        <HealthBadge health={check.health} />
      </div>
      <dl className="factgrid">
        <div>
          <dt>Latest evidence</dt>
          <dd className="evidence">{check.message || "No result yet"}</dd>
        </div>
        <div>
          <dt>{check.kind === "push" ? "Last heartbeat" : "Last checked"}</dt>
          <dd>
            {check.last_checked ? (
              <>
                <time dateTime={check.last_checked} title={fullTime(check.last_checked)}>
                  {localTime(check.last_checked)}
                </time>{" "}
                · {ago(check.last_checked, now)}
                {live && overdue && <span className="warn-text"> · older than {duration(p.stale_after_seconds)}</span>}
              </>
            ) : (
              "Never"
            )}
          </dd>
        </div>
        {check.kind !== "push" && (
          <div>
            <dt>Latency</dt>
            <dd>{check.latency_ms !== null ? `${check.latency_ms} ms` : "—"}</dd>
          </div>
        )}
        <div>
          <dt>Streak</dt>
          <dd>
            {check.failures > 0
              ? `${check.failures} consecutive failure${check.failures === 1 ? "" : "s"} (Down at ${p.failure_threshold})`
              : `${check.successes} consecutive pass${check.successes === 1 ? "" : "es"}`}
          </dd>
        </div>
        <div>
          <dt>Policy</dt>
          <dd>
            {policySummary(p, check.kind)}
            {check.kind !== "push" && ` · timeout ${duration(p.timeout_seconds)} · stale after ${duration(p.stale_after_seconds)}`}
          </dd>
        </div>
        <div>
          <dt>Perspective</dt>
          <dd>
            {check.kind === "push" ? (
              <>
                Producer → Vesper: <code>POST /api/heartbeat/{check.id}</code> with its bearer token
              </>
            ) : (
              "Vesper server → endpoint"
            )}
          </dd>
        </div>
      </dl>
      <History samples={check.history} label={`${check.name || check.id} history`} />
    </section>
  );
}

const SILENCE_OPTIONS = [
  { label: "1 hour", seconds: 3600 },
  { label: "4 hours", seconds: 14400 },
  { label: "24 hours", seconds: 86400 },
  { label: "7 days", seconds: 604800 },
];

export function ServicePage({ machineId, serviceId }: { machineId: string; serviceId: string }) {
  const { snapshot, live, now, refresh } = useLive();
  const recheck = useAction<unknown>();
  const silence = useAction<unknown>();
  const [silenceFor, setSilenceFor] = useState(3600);
  const [flash] = useState(takeFlash);
  const fullId = `${machineId}/${serviceId}`;
  if (!snapshot) return <PageTitle title={serviceId} sub="Waiting for the first snapshot…" />;
  const machine = snapshot.machines.find((m) => m.id === machineId);
  const service = machine?.services.find((s) => s.id === fullId);
  const crumbs = [
    { label: "Overview", to: paths.overview() },
    { label: machine?.name || machineId, to: machine ? paths.machine(machineId) : undefined },
    { label: service?.name || serviceId },
  ];
  if (!machine || !service)
    return (
      <>
        <Crumbs items={crumbs} />
        <PageTitle title="Service not found" />
        {flash && <Notice tone="good" title="Saved" live><p>{flash}</p></Notice>}
        <Notice>
          <p>
            No service <code>{fullId}</code> exists in the running configuration.
            {flash && " It appears here once the server activates the saved revision."}
          </p>
        </Notice>
      </>
    );
  const incidents = snapshot.incidents.filter((i) => i.service_id === fullId);
  const open = incidents.find((i) => !i.resolved_at);
  const silenced = service.silenced_until && Date.parse(service.silenced_until) > now;
  const activeChecks = service.checks.filter((c) => c.kind !== "push");
  const runSilence = (until: string | null) =>
    silence.run(async () => {
      const r = await api.silence(machineId, serviceId, until);
      await refresh();
      return r;
    });
  return (
    <>
      <Crumbs items={crumbs} />
      <PageTitle
        title={service.name}
        icon={<ServiceIcon icon={service.icon} size={44} />}
        sub={service.description || `${machine.name} · ${service.checks.length} checks`}
        aside={<HealthBadge health={service.health} />}
      />
      {flash && (
        <Notice tone="good" title="Saved" live>
          <p>{flash}</p>
        </Notice>
      )}
      {service.paused && (
        <Notice tone="neutral" title="Paused">
          <p>Checks are not running. A paused service is not considered healthy.</p>
        </Notice>
      )}
      {open && (
        <Notice tone={open.acknowledged_at ? "warn" : "bad"} title={`Incident #${open.id} is open`}>
          <p>
            Since {localTime(open.opened_at)} ({ago(open.opened_at, now)}) ·{" "}
            {open.acknowledged_at ? "acknowledged" : "not acknowledged"} · {open.notification_state}.{" "}
            <Link to={paths.incident(open.id)}>Open incident</Link>
          </p>
        </Notice>
      )}
      <dl className="factgrid">
        <div>
          <dt>Service ID</dt>
          <dd>
            <code>{service.id}</code>
          </dd>
        </div>
        <div>
          <dt>Source file</dt>
          <dd>
            <code>{service.source || "—"}</code>
          </dd>
        </div>
        <div>
          <dt>Notifications</dt>
          <dd>
            {silenced
              ? `Silenced until ${localTime(service.silenced_until)}`
              : service.maintenance
                ? "Suppressed: maintenance window active"
                : "Active"}
          </dd>
        </div>
        <div>
          <dt>Snapshot</dt>
          <dd>
            {live ? "Live" : "Disconnected"} · generated {localTime(snapshot.generated_at)}
          </dd>
        </div>
      </dl>
      <div className="actions">
        <button
          className="btn primary"
          disabled={recheck.busy || service.paused || activeChecks.length === 0}
          onClick={() =>
            recheck.run(async () => {
              const r = await api.recheck(machineId, serviceId);
              await refresh();
              return r;
            })
          }
        >
          {recheck.busy ? "Queueing…" : "Recheck now"}
        </button>
        <Link to={paths.editService(machineId, serviceId)} className="btn">
          Edit service
        </Link>
        {silenced ? (
          <button className="btn" disabled={silence.busy} onClick={() => runSilence(null)}>
            Resume notifications
          </button>
        ) : (
          <span className="btn-group">
            <label>
              <span className="sr">Silence duration</span>
              <select value={silenceFor} onChange={(e) => setSilenceFor(Number(e.target.value))}>
                {SILENCE_OPTIONS.map((o) => (
                  <option key={o.seconds} value={o.seconds}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
            <button
              className="btn"
              disabled={silence.busy}
              onClick={() => runSilence(new Date(Date.now() + silenceFor * 1000).toISOString())}
            >
              Silence
            </button>
          </span>
        )}
      </div>
      <div aria-live="polite">
        {recheck.result !== null && (
          <Notice tone="neutral" title="Recheck queued">
            <p>Active checks will run shortly; results appear here as they complete.</p>
          </Notice>
        )}
        {recheck.error && <Notice tone="bad" title="Recheck failed">{recheck.error}</Notice>}
        {silence.error && <Notice tone="bad" title="Silence change failed">{silence.error}</Notice>}
      </div>
      {activeChecks.length === 0 && service.checks.length > 0 && (
        <p className="note">Push checks wait for their producer; Vesper cannot re-run them.</p>
      )}
      <section aria-labelledby="checks-title">
        <h2 id="checks-title">Checks</h2>
        {service.checks.length === 0 && <p className="note">This service has no checks.</p>}
        {service.checks.map((c) => (
          <CheckCard key={c.id} check={c} />
        ))}
      </section>
      {incidents.length > 0 && (
        <section aria-labelledby="svc-incidents">
          <h2 id="svc-incidents">Incident history</h2>
          <ul className="plain-list">
            {incidents.map((i) => (
              <li key={i.id}>
                <Link to={paths.incident(i.id)}>
                  #{i.id} · {localTime(i.opened_at)}
                  {i.resolved_at
                    ? ` – ${localTime(i.resolved_at)} (${duration((Date.parse(i.resolved_at) - Date.parse(i.opened_at)) / 1000)})`
                    : " · open"}
                </Link>
              </li>
            ))}
          </ul>
        </section>
      )}
    </>
  );
}

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { DailyBars, hourStart, ResponseChart, slotsFromHours, type Slot } from "../components/charts";
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
import {
  ago,
  duration,
  fullTime,
  kindLabel,
  latency,
  localDate,
  localTime,
  policySummary,
  uptimeText,
} from "../format";
import { Link, paths } from "../router";
import type { CheckEvent as CheckEventView, CheckStats, CheckView } from "../types";

const RANGES = [
  { key: "recent", label: "Recent", hours: 0, slotHours: 0 },
  { key: "24h", label: "24 hours", hours: 24, slotHours: 1 },
  { key: "7d", label: "7 days", hours: 7 * 24, slotHours: 1 },
  { key: "30d", label: "30 days", hours: 30 * 24, slotHours: 6 },
] as const;

const WINDOW_LABELS: Record<number, string> = {
  3600: "Last hour",
  86400: "24 hours",
  604800: "7 days",
  2592000: "30 days",
  31536000: "1 year",
};

const DAILY_BARS = 30;
const EVENTS_SHOWN = 6;

/** Loads the check's stats, again whenever a new result arrives. */
function useCheckStats(id: string, lastChecked: string | null) {
  const [stats, setStats] = useState<CheckStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    api.checkStats(id).then(
      (s) => !cancelled && (setStats(s), setError(null)),
      (e) => !cancelled && setError(errorText(e)),
    );
    return () => {
      cancelled = true;
    };
  }, [id, lastChecked]);
  return { stats, error };
}

function ResponseTime({ check, stats, now }: { check: CheckView; stats: CheckStats | null; now: number }) {
  const [range, setRange] = useState<(typeof RANGES)[number]["key"]>("recent");
  const r = RANGES.find((x) => x.key === range)!;
  const name = check.name || check.id;
  let chart;
  if (r.hours === 0) {
    const samples = check.history;
    const slots: Slot[] = samples.map((s) => ({
      t: Date.parse(s.at),
      avg: s.healthy ? s.latency_ms : null,
      min: null,
      max: null,
      passes: s.healthy ? 1 : 0,
      failures: s.healthy ? 0 : 1,
    }));
    const from = slots.length ? slots[0].t : now - 3600_000;
    const passing = samples.filter((s) => s.healthy).map((s) => s.latency_ms);
    chart = samples.length ? (
      <ResponseChart
        slots={slots}
        from={from}
        to={Math.max(now, from + 60_000)}
        slotMs={0}
        label={`${name} response time, last ${samples.length} samples: ${
          passing.length
            ? `average ${latency(passing.reduce((a, b) => a + b, 0) / passing.length)}, ${latency(Math.min(...passing))} to ${latency(Math.max(...passing))}`
            : "no passing samples"
        }, ${samples.length - passing.length} failed`}
      />
    ) : (
      <p className="note">No samples recorded yet.</p>
    );
  } else if (!stats) {
    chart = <p className="note">Loading…</p>;
  } else {
    const slotMs = r.slotHours * 3600_000;
    const to = hourStart(now) + 3600_000;
    const from = to - r.hours * 3600_000;
    const slots = slotsFromHours(stats.hours, from, slotMs, r.hours / r.slotHours);
    const w = stats.windows.find((x) => x.seconds === r.hours * 3600);
    chart = slots.some((s) => s.passes + s.failures > 0) ? (
      <ResponseChart
        slots={slots}
        from={from}
        to={to}
        slotMs={slotMs}
        label={`${name} response time over ${r.label}${
          w ? `: average ${latency(w.latency_avg_ms)}, ${latency(w.latency_min_ms)} to ${latency(w.latency_max_ms)}, ${w.failures} failed` : ""
        }`}
      />
    ) : (
      <p className="note">No samples in the last {r.label}.</p>
    );
  }
  return (
    <>
      <div className="sectionline subhead">
        <h4>Response time</h4>
        <div className="segmented compact" role="group" aria-label={`${name} response time range`}>
          {RANGES.map((x) => (
            <button key={x.key} type="button" aria-pressed={x.key === range} onClick={() => setRange(x.key)}>
              {x.label}
            </button>
          ))}
        </div>
      </div>
      {chart}
    </>
  );
}

function DailyUptime({ check, stats, now }: { check: CheckView; stats: CheckStats; now: number }) {
  const today = new Date(now);
  const days = Array.from({ length: DAILY_BARS }, (_, i) => {
    const d = new Date(today.getFullYear(), today.getMonth(), today.getDate() - (DAILY_BARS - 1 - i));
    return { t: d.getTime(), passes: 0, failures: 0 };
  });
  for (const h of stats.hours) {
    const t = Date.parse(h.hour);
    const day = days.findLast((d) => d.t <= t);
    if (day) {
      day.passes += h.passes;
      day.failures += h.failures;
    }
  }
  const withData = days.filter((d) => d.passes + d.failures > 0);
  const bad = withData.filter((d) => d.failures > 0).length;
  return (
    <>
      <div className="sectionline subhead">
        <h4>Daily uptime</h4>
      </div>
      <DailyBars
        days={days}
        label={`${check.name || check.id} daily uptime, last ${DAILY_BARS} days: ${withData.length} days with samples, ${bad} with failures`}
      />
      <div className="legend">
        <span>
          {localDate(days[0].t)} – today
        </span>
        <span>
          {withData.length === 0
            ? "No samples yet"
            : bad === 0
              ? `${withData.length} day${withData.length === 1 ? "" : "s"} without failures`
              : `${bad} of ${withData.length} days with failures`}
        </span>
      </div>
    </>
  );
}

function CheckEvents({ events, now }: { events: CheckEventView[]; now: number }) {
  if (events.length === 0) return <p className="note">No state changes recorded yet.</p>;
  const item = (e: CheckEventView, i: number) => {
    const until = i === 0 ? null : events[i - 1].at;
    return (
      <li key={`${e.at}-${i}`}>
        <time dateTime={e.at} title={fullTime(e.at)}>
          {localTime(e.at)}
        </time>
        <div>
          <HealthBadge health={e.health} live />{" "}
          <span className="muted">
            for {duration(((until ? Date.parse(until) : now) - Date.parse(e.at)) / 1000)}
            {!until && " so far"}
          </span>
          <div className="evidence">{e.message}</div>
        </div>
      </li>
    );
  };
  return (
    <>
      <ol className="timeline events">{events.slice(0, EVENTS_SHOWN).map(item)}</ol>
      {events.length > EVENTS_SHOWN && (
        <details className="samples">
          <summary>
            Show {events.length - EVENTS_SHOWN} older event{events.length - EVENTS_SHOWN === 1 ? "" : "s"}
          </summary>
          <ol className="timeline events" start={EVENTS_SHOWN + 1}>
            {events.slice(EVENTS_SHOWN).map((e, i) => item(e, i + EVENTS_SHOWN))}
          </ol>
        </details>
      )}
    </>
  );
}

function CheckCard({ check }: { check: CheckView }) {
  const { live, now } = useLive();
  const { stats, error } = useCheckStats(check.id, check.last_checked);
  const p = check.policy;
  const push = check.kind === "push";
  const overdue =
    check.last_checked !== null && now - Date.parse(check.last_checked) > p.stale_after_seconds * 1000;
  const day = stats?.windows.find((w) => w.seconds === 86400);
  return (
    <section className="card check" aria-labelledby={`check-${check.id}`}>
      <div className="sectionline">
        <h3 id={`check-${check.id}`}>
          {check.name || check.id} <span className="pill">{kindLabel(check.kind)}</span>
        </h3>
        <HealthBadge health={check.health} />
      </div>
      <dl className="tiles">
        <div className="tile wide">
          <dt>Latest evidence</dt>
          <dd className="evidence">{check.message || "No result yet"}</dd>
        </div>
        {!push && (
          <>
            <div className="tile">
              <dt>Avg response · 24h</dt>
              <dd>{latency(day?.latency_avg_ms)}</dd>
            </div>
            <div className="tile">
              <dt>Response range · 24h</dt>
              <dd>
                {day?.latency_min_ms != null && day.latency_max_ms != null
                  ? `${latency(day.latency_min_ms)} – ${latency(day.latency_max_ms)}`
                  : "—"}
              </dd>
            </div>
          </>
        )}
        <div className="tile">
          <dt>{push ? "Last heartbeat" : "Last checked"}</dt>
          <dd>
            {check.last_checked ? (
              <>
                <time dateTime={check.last_checked} title={fullTime(check.last_checked)}>
                  {ago(check.last_checked, now)}
                </time>
                {!push && check.latency_ms !== null && <span className="muted"> · {latency(check.latency_ms)}</span>}
                {live && overdue && <span className="warn-text"> · older than {duration(p.stale_after_seconds)}</span>}
              </>
            ) : (
              "Never"
            )}
          </dd>
        </div>
      </dl>
      {error && <Notice tone="bad" title="Statistics unavailable">{error}</Notice>}
      <div className="sectionline subhead">
        <h4>Uptime</h4>
      </div>
      <dl className="tiles uptime">
        {(stats?.windows ?? Object.keys(WINDOW_LABELS).map((k) => ({ seconds: Number(k), passes: 0, failures: 0 }))).map((w) => (
          <div className="tile" key={w.seconds}>
            <dt>{WINDOW_LABELS[w.seconds] ?? duration(w.seconds)}</dt>
            <dd>{stats ? uptimeText(w.passes, w.failures) : "…"}</dd>
            {stats && (
              <dd className="tile-sub">
                {w.failures > 0
                  ? `${w.failures.toLocaleString()} of ${(w.passes + w.failures).toLocaleString()} failed`
                  : w.passes === 0
                    ? "no samples"
                    : `${w.passes.toLocaleString()} ${push ? "heartbeat" : "sample"}${w.passes === 1 ? "" : "s"}`}
              </dd>
            )}
          </div>
        ))}
      </dl>
      {!push && <ResponseTime check={check} stats={stats} now={now} />}
      {stats && <DailyUptime check={check} stats={stats} now={now} />}
      <div className="sectionline subhead">
        <h4>Recent samples</h4>
      </div>
      <History samples={check.history} label={`${check.name || check.id} history`} />
      <div className="sectionline subhead">
        <h4>Events</h4>
      </div>
      {stats ? <CheckEvents events={stats.events} now={now} /> : <p className="note">Loading…</p>}
      <dl className="factgrid">
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
            {!push && ` · timeout ${duration(p.timeout_seconds)} · stale after ${duration(p.stale_after_seconds)}`}
          </dd>
        </div>
        <div>
          <dt>Perspective</dt>
          <dd>
            {push ? (
              <>
                Producer → Vesper: <code>POST /api/heartbeat/{check.id}</code> with its bearer token
              </>
            ) : (
              "Vesper server → endpoint"
            )}
          </dd>
        </div>
      </dl>
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
        {snapshot.config_writable && (
          <Link to={paths.editService(machineId, serviceId)} className="btn">
            Edit service
          </Link>
        )}
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

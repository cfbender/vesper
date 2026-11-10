import { HealthBadge, ServiceIcon, StatusDot, ToneTag } from "../components/ui";
import { useLive } from "../data";
import { ago, healthDisplay, localTime, type Tone } from "../format";
import { Link, paths, splitServiceId } from "../router";
import type { IncidentView, MachineView, ServiceView, Snapshot } from "../types";

export function findService(snapshot: Snapshot, serviceId: string) {
  const [m, s] = splitServiceId(serviceId);
  const machine = snapshot.machines.find((x) => x.id === m);
  const service = machine?.services.find((x) => x.id === serviceId || x.id === s);
  return { machine, service };
}

/** Most recent check time across a set of services. */
export function lastChecked(services: ServiceView[]): string | null {
  let best: string | null = null;
  for (const s of services)
    for (const c of s.checks)
      if (c.last_checked && (!best || Date.parse(c.last_checked) > Date.parse(best))) best = c.last_checked;
  return best;
}

export function serviceFlags(s: ServiceView, now: number): string[] {
  const flags: string[] = [];
  if (s.paused) flags.push("Paused");
  if (s.maintenance) flags.push("Maintenance");
  if (s.silenced_until && Date.parse(s.silenced_until) > now) flags.push("Silenced");
  return flags;
}

function ServiceTile({ service, machine }: { service: ServiceView; machine: MachineView }) {
  const { live, now } = useLive();
  const d = healthDisplay(service.health, live);
  const flags = serviceFlags(service, now);
  const [m, s] = splitServiceId(service.id);
  return (
    <Link
      to={paths.service(m, s || service.id)}
      className={`service-tile ${d.tone}`}
      aria-label={`${service.name}, ${d.label}${flags.length ? `, ${flags.join(", ")}` : ""}, on ${machine.name}`}
    >
      <ServiceIcon icon={service.icon} size={28} />
      <span className="service-name">{service.name}</span>
      {flags.length > 0 && <span className="tile-flag">{flags.join(" · ")}</span>}
      <span className="tile-dot">
        <StatusDot tone={d.tone} size={12} />
      </span>
    </Link>
  );
}

function MachineCard({ machine }: { machine: MachineView }) {
  const { live, now } = useLive();
  const checks = machine.services.flatMap((s) => s.checks);
  const failing = checks.filter((c) => c.health === "down" || c.health === "suspect").length;
  const last = lastChecked(machine.services);
  return (
    <section className="host card" aria-labelledby={`machine-${machine.id}`}>
      <Link to={paths.machine(machine.id)} className="host-link">
        <span className="host-head">
          <h3 id={`machine-${machine.id}`}>{machine.name || machine.id}</h3>
          <HealthBadge health={machine.health} />
        </span>
        <span className="host-sub">
          {[machine.site, machine.description].filter(Boolean).join(" · ") || machine.id}
        </span>
      </Link>
      {machine.services.length > 0 ? (
        <div className="service-grid">
          {machine.services.map((s) => (
            <ServiceTile key={s.id} service={s} machine={machine} />
          ))}
        </div>
      ) : (
        <p className="note">No services yet.</p>
      )}
      <p className="foot">
        {machine.services.length} service{machine.services.length === 1 ? "" : "s"} · {checks.length} check
        {checks.length === 1 ? "" : "s"}
        {live && failing > 0 ? ` · ${failing} failing` : ""} ·{" "}
        {live ? `checked ${ago(last, now)}` : `last seen ${localTime(last)}`}
      </p>
    </section>
  );
}

/** Incident status label; while disconnected it is the last known state, never a fresh alarm. */
export function incidentState(i: IncidentView, live: boolean): { label: string; tone: Tone } {
  const label = i.resolved_at ? "Resolved" : i.acknowledged_at ? "Acknowledged" : "Open";
  if (!live) return { label: `${label} (last known)`, tone: "stale" };
  return { label, tone: i.resolved_at ? "good" : i.acknowledged_at ? "warn" : "bad" };
}

export function IncidentRow({ incident }: { incident: IncidentView }) {
  const { snapshot, now, live } = useLive();
  const { machine } = snapshot ? findService(snapshot, incident.service_id) : { machine: undefined };
  const open = !incident.resolved_at;
  const state = incidentState(incident, live);
  return (
    <Link to={paths.incident(incident.id)} className="rowlink incident-row">
      <span className="row-main">
        <strong>{incident.service_name}</strong>
        <small>
          {machine ? `${machine.name} · ` : ""}
          {open
            ? `open since ${localTime(incident.opened_at)} (${ago(incident.opened_at, now)})`
            : `resolved ${localTime(incident.resolved_at)}`}
          {incident.correlated_with ? ` · correlated with ${incident.correlated_with}` : ""}
        </small>
      </span>
      <span className="row-side">
        <ToneTag tone={state.tone}>{state.label}</ToneTag>
        <small>{incident.notification_state}</small>
      </span>
    </Link>
  );
}

function SetupState({ snapshot }: { snapshot: Snapshot }) {
  return (
    <section className="card setup" aria-labelledby="setup-title">
      <h2 id="setup-title">Bring your homelab into view</h2>
      <p>
        No machines are configured yet. Add a first service here, or write YAML files in the
        configuration root and Vesper reloads them automatically.
      </p>
      <ol className="steps">
        <li>Add a service with at least one HTTP, TCP or push check, and test it before saving.</li>
        <li>Configure Telegram in YAML and send a real test message from Settings.</li>
        <li>Come back here to see every service and its current status.</li>
      </ol>
      {!snapshot.config_writable && (
        <p className="note">
          Configuration is read-only. The editor can still draft, test and export YAML for you to place
          in the configuration root.
        </p>
      )}
      <div className="actions">
        <Link to={paths.newService()} className="btn primary">
          Add a service
        </Link>
        <Link to={paths.settings()} className="btn">
          Settings &amp; Telegram
        </Link>
      </div>
    </section>
  );
}

export function Overview() {
  const { snapshot } = useLive();
  if (!snapshot) return <p role="status">Waiting for the first snapshot…</p>;
  return (
    <>
      <h1 className="sr">Overview</h1>
      {snapshot.machines.length === 0 ? (
        <SetupState snapshot={snapshot} />
      ) : (
        <div className="hosts">
          {snapshot.machines.map((m) => (
            <MachineCard key={m.id} machine={m} />
          ))}
        </div>
      )}
    </>
  );
}

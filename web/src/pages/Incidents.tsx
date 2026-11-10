import { useState } from "react";
import { api } from "../api";
import { Crumbs, Notice, PageTitle, ToneTag, useAction } from "../components/ui";
import { useLive } from "../data";
import { ago, duration, fullTime, localTime } from "../format";
import { Link, paths, splitServiceId } from "../router";
import { findService, incidentState, IncidentRow } from "./Overview";

type Filter = "open" | "resolved" | "all";

export function IncidentsPage() {
  const { snapshot, live } = useLive();
  const [filter, setFilter] = useState<Filter>("open");
  if (!snapshot) return <PageTitle title="Incidents" sub="Waiting for the first snapshot…" />;
  const shown = snapshot.incidents.filter((i) =>
    filter === "all" ? true : filter === "open" ? !i.resolved_at : !!i.resolved_at,
  );
  return (
    <>
      <PageTitle
        title="Incidents"
        sub="One incident per service outage. Incident state and notification state are tracked separately."
      />
      <div className="segmented" role="group" aria-label="Show incidents">
        {(["open", "resolved", "all"] as Filter[]).map((f) => (
          <button key={f} aria-pressed={filter === f} onClick={() => setFilter(f)}>
            {f === "open" ? "Open" : f === "resolved" ? "Resolved" : "All"}
          </button>
        ))}
      </div>
      {!live && (
        <p className="note">Disconnected: this is the incident list from the last snapshot.</p>
      )}
      <div className="list">
        {shown.map((i) => (
          <IncidentRow key={i.id} incident={i} />
        ))}
        {shown.length === 0 && (
          <p className="note">
            {filter === "open" ? "No open incidents." : "No incidents to show."} Resolved incidents stay
            in history.
          </p>
        )}
      </div>
    </>
  );
}

const SILENCE_HOUR = 3600 * 1000;

export function IncidentPage({ id }: { id: number }) {
  const { snapshot, now, refresh, live } = useLive();
  const ack = useAction<unknown>();
  const silence = useAction<unknown>();
  if (!snapshot) return <PageTitle title={`Incident #${id}`} sub="Waiting for the first snapshot…" />;
  const incident = snapshot.incidents.find((i) => i.id === id);
  const crumbs = [{ label: "Incidents", to: paths.incidents() }, { label: `#${id}` }];
  if (!incident)
    return (
      <>
        <Crumbs items={crumbs} />
        <PageTitle title="Incident not found" />
        <Notice>
          <p>Incident #{id} is not in the current snapshot.</p>
        </Notice>
      </>
    );
  const { machine, service } = findService(snapshot, incident.service_id);
  const [m, s] = splitServiceId(incident.service_id);
  const open = !incident.resolved_at;
  const silenced = service?.silenced_until && Date.parse(service.silenced_until) > now;
  const end = incident.resolved_at ? Date.parse(incident.resolved_at) : now;
  const correlated = incident.correlated_with ? splitServiceId(incident.correlated_with) : null;
  return (
    <>
      <Crumbs items={crumbs} />
      <PageTitle
        title={`${incident.service_name} ${open ? "is having an outage" : "outage resolved"}`}
        sub={`${machine?.name ?? m} · incident #${incident.id} · opened ${localTime(incident.opened_at)}`}
        aside={<ToneTag tone={incidentState(incident, live).tone}>{incidentState(incident, live).label}</ToneTag>}
      />
      <dl className="factgrid">
        <div>
          <dt>Opened</dt>
          <dd>{fullTime(incident.opened_at)}</dd>
        </div>
        <div>
          <dt>{open ? "Duration so far" : "Resolved"}</dt>
          <dd>
            {open
              ? duration((end - Date.parse(incident.opened_at)) / 1000)
              : `${fullTime(incident.resolved_at)} · lasted ${duration((end - Date.parse(incident.opened_at)) / 1000)}`}
          </dd>
        </div>
        <div>
          <dt>Acknowledged</dt>
          <dd>{incident.acknowledged_at ? fullTime(incident.acknowledged_at) : "No"}</dd>
        </div>
        <div>
          <dt>Notifications</dt>
          <dd>
            {incident.notification_state}
            {silenced && ` · silenced until ${localTime(service?.silenced_until)}`}
          </dd>
        </div>
        {correlated && (
          <div>
            <dt>Correlated with</dt>
            <dd>
              <Link to={paths.service(correlated[0], correlated[1])}>{incident.correlated_with}</Link>{" "}
              <small className="muted">(dependency failure; not a confirmed root cause)</small>
            </dd>
          </div>
        )}
      </dl>
      <section aria-labelledby="timeline-title">
        <h2 id="timeline-title">Timeline</h2>
        <ol className="timeline">
          {incident.events.map((e, i) => (
            <li key={`${e.at}-${i}`}>
              <time dateTime={e.at} title={fullTime(e.at)}>
                {localTime(e.at)}
              </time>
              <span className="evidence">{e.message}</span>
            </li>
          ))}
          {incident.events.length === 0 && <li>No events recorded.</li>}
        </ol>
      </section>
      <div className="actions">
        {open && (
          <button
            className="btn primary"
            disabled={ack.busy}
            onClick={() =>
              ack.run(async () => {
                const r = await api.acknowledge(incident.id, !incident.acknowledged_at);
                await refresh();
                return r;
              })
            }
          >
            {incident.acknowledged_at ? "Undo acknowledge" : "Acknowledge"}
          </button>
        )}
        {open && service && (
          <button
            className="btn"
            disabled={silence.busy}
            onClick={() =>
              silence.run(async () => {
                const r = await api.silence(
                  m,
                  s,
                  silenced ? null : new Date(Date.now() + SILENCE_HOUR).toISOString(),
                );
                await refresh();
                return r;
              })
            }
          >
            {silenced ? "Resume notifications" : "Silence service for 1 hour"}
          </button>
        )}
        {service && (
          <Link to={paths.service(m, s)} className="btn">
            Inspect {incident.service_name}
          </Link>
        )}
      </div>
      <div aria-live="polite">
        {ack.error && <Notice tone="bad" title="Acknowledge failed">{ack.error}</Notice>}
        {silence.error && <Notice tone="bad" title="Silence change failed">{silence.error}</Notice>}
      </div>
      <p className="note">
        Acknowledging records awareness. Silencing pauses notifications. Neither changes health; only
        recovery closes the incident. Last updated {ago(snapshot.generated_at, now)}.
      </p>
    </>
  );
}

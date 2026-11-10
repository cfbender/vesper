import { Crumbs, HealthBadge, Notice, PageTitle, ServiceIcon } from "../components/ui";
import { useLive } from "../data";
import { ago, kindLabel, localTime } from "../format";
import { Link, paths, splitServiceId } from "../router";
import { IncidentRow, lastChecked, serviceFlags } from "./Overview";

export function MachinePage({ machineId }: { machineId: string }) {
  const { snapshot, live, now } = useLive();
  if (!snapshot) return <PageTitle title={machineId} sub="Waiting for the first snapshot…" />;
  const machine = snapshot.machines.find((m) => m.id === machineId);
  if (!machine)
    return (
      <>
        <Crumbs items={[{ label: "Overview", to: paths.overview() }, { label: machineId }]} />
        <PageTitle title="Machine not found" />
        <Notice tone="neutral">
          <p>
            No machine <code>{machineId}</code> exists in the running configuration.
          </p>
        </Notice>
      </>
    );
  const ids = new Set(machine.services.map((s) => s.id));
  const incidents = snapshot.incidents.filter((i) => !i.resolved_at && ids.has(i.service_id));
  const last = lastChecked(machine.services);
  return (
    <>
      <Crumbs items={[{ label: "Overview", to: paths.overview() }, { label: machine.name || machine.id }]} />
      <PageTitle
        title={machine.name || machine.id}
        sub={[machine.site, machine.description].filter(Boolean).join(" · ") || undefined}
        aside={<HealthBadge health={machine.health} />}
      />
      <dl className="factgrid">
        <div>
          <dt>Machine ID</dt>
          <dd>
            <code>{machine.id}</code>
          </dd>
        </div>
        <div>
          <dt>Services</dt>
          <dd>{machine.services.length}</dd>
        </div>
        <div>
          <dt>Freshest result</dt>
          <dd>
            {localTime(last)} {last && `· ${ago(last, now)}`}
          </dd>
        </div>
        <div>
          <dt>Health meaning</dt>
          <dd>Derived from configured checks, not hardware metrics.</dd>
        </div>
      </dl>
      {incidents.length > 0 && (
        <section aria-labelledby="machine-incidents">
          <h2 id="machine-incidents">Open incidents</h2>
          <p className="note">
            Failures on several services at once may be related; correlation is not a confirmed root
            cause.
          </p>
          <div className="list">
            {incidents.map((i) => (
              <IncidentRow key={i.id} incident={i} />
            ))}
          </div>
        </section>
      )}
      <section aria-labelledby="machine-services">
        <div className="sectionline">
          <h2 id="machine-services">Services</h2>
          <Link to={paths.newService(machine.id)} className="btn small">
            Add service here
          </Link>
        </div>
        <div className="list">
          {machine.services.map((s) => {
            const [m, sid] = splitServiceId(s.id);
            const kinds = [...new Set(s.checks.map((c) => kindLabel(c.kind)))].join(", ");
            const flags = serviceFlags(s, now);
            const sl = lastChecked([s]);
            return (
              <Link key={s.id} to={paths.service(m, sid)} className="rowlink">
                <span className="row-icon">
                  <ServiceIcon icon={s.icon} size={28} />
                </span>
                <span className="row-main">
                  <strong>{s.name}</strong>
                  <small>
                    {s.checks.length} check{s.checks.length === 1 ? "" : "s"}
                    {kinds && ` · ${kinds}`}
                    {flags.length > 0 && ` · ${flags.join(", ")}`} ·{" "}
                    {live ? `checked ${ago(sl, now)}` : `last result ${localTime(sl)}`}
                  </small>
                </span>
                <span className="row-side">
                  <HealthBadge health={s.health} />
                  <span aria-hidden="true" className="chev">
                    ›
                  </span>
                </span>
              </Link>
            );
          })}
          {machine.services.length === 0 && <p className="note">No services on this machine yet.</p>}
        </div>
      </section>
    </>
  );
}

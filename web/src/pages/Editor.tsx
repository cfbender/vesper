import { useEffect, useMemo, useState } from "react";
import YAML from "yaml";
import { api, ApiError, errorText } from "../api";
import { short } from "../components/Layout";
import { Crumbs, Notice, PageTitle, setFlash, ToneTag } from "../components/ui";
import { useLive } from "../data";
import { CheckEditor, type TestState } from "../editor/CheckEditor";
import {
  assembleCheck,
  draftFingerprint,
  draftMachine,
  draftService,
  editDraft,
  exactIcon,
  exportYaml,
  newCheck,
  newDraft,
  serviceSources,
  stripNulls,
  validateDraft,
  type Draft,
} from "../editor/draft";
import { TextArea, TextField, Toggle } from "../editor/fields";
import { IconPicker } from "../editor/IconPicker";
import { navigate, paths, useNavigationBlocker } from "../router";
import type { CheckKind, ConfigResponse, MaintenanceWindow, Service } from "../types";
import { useConfig } from "./Settings";

const LEAVE_MESSAGE = "You have unsaved changes to this service. Leave and discard them?";

function toLocalInput(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}
const fromLocalInput = (v: string) => (v ? new Date(v).toISOString().replace(".000Z", "Z") : "");

interface Conflict {
  message: string;
  latest: ConfigResponse | null;
}

export function EditorPage({
  machineId,
  serviceId,
  presetMachine = null,
}: {
  machineId: string | null;
  serviceId: string | null;
  presetMachine?: string | null;
}) {
  const editing = machineId !== null && serviceId !== null;
  const { snapshot, iconList, refresh } = useLive();
  const { config, error: configError, load } = useConfig();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [original, setOriginal] = useState<Service | null>(null);
  const [initialFp, setInitialFp] = useState("");
  const [baseRevision, setBaseRevision] = useState("");
  const [notFound, setNotFound] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<Conflict | null>(null);
  const [tests, setTests] = useState<Record<string, TestState>>({});
  const [copied, setCopied] = useState(false);

  const initFrom = (c: ConfigResponse) => {
    const d = editing ? editDraft(machineId, serviceId, c) : newDraft(presetMachine, c);
    if (!d) {
      setNotFound(true);
      return;
    }
    setDraft(d);
    setOriginal(editing ? (c.machines[machineId]?.services[serviceId] ?? null) : null);
    setInitialFp(draftFingerprint(d));
    setBaseRevision(c.revision);
    setTests({});
    setAttempted(false);
  };

  useEffect(() => {
    if (config && !draft && !notFound) initFrom(config);
    // Initialise once per page; later config loads never replace the draft.
  }, [config]);

  const dirty = draft !== null && draftFingerprint(draft) !== initialFp;
  useNavigationBlocker(dirty && !saving ? LEAVE_MESSAGE : null);

  const errors = useMemo(() => (draft && config ? validateDraft(draft, config) : {}), [draft, config]);
  const errorCount = Object.keys(errors).length;
  const yaml = useMemo(() => (draft ? exportYaml(draft) : ""), [draft]);

  const title = editing ? `Edit ${original?.name || serviceId}` : "Add service";
  const crumbs = editing
    ? [
        { label: "Overview", to: paths.overview() },
        { label: config?.machines[machineId]?.name || machineId, to: paths.machine(machineId) },
        { label: original?.name || serviceId, to: paths.service(machineId, serviceId) },
        { label: "Edit" },
      ]
    : [{ label: "Overview", to: paths.overview() }, { label: "Add service" }];

  if (notFound)
    return (
      <>
        <Crumbs items={crumbs} />
        <PageTitle title="Service not found" />
        <Notice>
          <p>
            <code>
              {machineId}/{serviceId}
            </code>{" "}
            is not in the configuration files.
          </p>
        </Notice>
      </>
    );
  if (!config || !draft)
    return (
      <>
        <Crumbs items={crumbs} />
        <PageTitle title={title} sub={configError ? undefined : "Loading configuration…"} />
        {configError && (
          <Notice tone="bad" title="Could not load configuration">
            <p>{configError}</p>
            <button className="btn" onClick={() => load()}>
              Try again
            </button>
          </Notice>
        )}
      </>
    );

  const update = (fn: (d: Draft) => Draft) =>
    setDraft((d) => {
      if (!d) return d;
      const next = fn(d);
      if (next.isNew && !next.iconTouched) {
        const match = exactIcon(iconList, next.service.name, next.serviceId);
        next.service = { ...next.service, icon: match?.id ?? "server" };
      }
      return next;
    });
  const setService = (patch: Partial<Draft["service"]>) =>
    update((d) => ({ ...d, service: { ...d.service, ...patch } }));
  const setWindows = (fn: (w: MaintenanceWindow[]) => MaintenanceWindow[]) =>
    setService({ maintenance: fn(draft.service.maintenance) });

  const writable = config.writable;
  const sources = serviceSources(config, draft.machineId, draft.serviceId);
  const allServiceIds = Object.entries(config.machines)
    .flatMap(([m, mc]) => Object.keys(mc.services).map((s) => `${m}/${s}`))
    .sort();
  const self = `${draft.machineId}/${draft.serviceId}`;
  const dependencyOptions = [...new Set([...allServiceIds, ...draft.service.depends_on])].filter((id) => id !== self).sort();
  const revisionMoved = snapshot && snapshot.config_revision !== baseRevision && !conflict && !saving;

  const runTests = async (keys: string[]) => {
    const subset = draft.checks.filter((c) => keys.includes(c.key));
    if (subset.length === 0) return;
    const at = new Date().toISOString();
    setTests((t) => ({ ...t, ...Object.fromEntries(subset.map((c) => [c.key, { busy: true }])) }));
    try {
      // Keys are unique and path-safe even while check IDs are being edited.
      const { results } = await api.draftTest(Object.fromEntries(subset.map((c) => [c.key, assembleCheck(c)])));
      setTests((t) => ({
        ...t,
        ...Object.fromEntries(
          subset.map((c) => [
            c.key,
            results[c.key]
              ? { busy: false, at, tested: JSON.stringify(c), result: results[c.key] }
              : { busy: false, at, tested: JSON.stringify(c), error: "The server returned no result for this check." },
          ]),
        ),
      }));
    } catch (e) {
      setTests((t) => ({
        ...t,
        ...Object.fromEntries(subset.map((c) => [c.key, { busy: false, at, tested: JSON.stringify(c), error: errorText(e) }])),
      }));
    }
  };

  const save = async (paused: boolean) => {
    setAttempted(true);
    setSaveError(null);
    if (errorCount > 0) {
      requestAnimationFrame(() =>
        document.querySelector<HTMLElement>('.editor [aria-invalid="true"]')?.focus(),
      );
      return;
    }
    setSaving(true);
    let revision = baseRevision;
    try {
      if (draft.machineMode === "new") {
        const r = await api.saveMachine(draft.machineId, revision, draftMachine(draft));
        revision = r.revision;
        setBaseRevision(revision);
        await load();
        setDraft((d) => d && { ...d, machineMode: "existing" });
      }
      const body = { ...draftService(draft), paused };
      const r = await api.saveService(draft.machineId, draft.serviceId, revision, body);
      setInitialFp(draftFingerprint({ ...draft, machineMode: "existing", service: { ...draft.service, paused } }));
      setFlash(r.message || "Saved.");
      await refresh();
      navigate(paths.service(draft.machineId, draft.serviceId), { force: true });
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        const latest = await api.config().catch(() => null);
        setConflict({ message: e.message, latest });
      } else setSaveError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const latestService =
    conflict?.latest?.machines[draft.machineId]?.services[draft.serviceId] ?? null;
  const serviceChangedRemotely =
    conflict?.latest && JSON.stringify(latestService) !== JSON.stringify(original);

  const download = () => {
    const url = URL.createObjectURL(new Blob([yaml], { type: "application/yaml" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `${draft.serviceId || "service"}.yaml`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const show = attempted;
  return (
    <div className="editor">
      <Crumbs items={crumbs} />
      <PageTitle
        title={title}
        sub={
          editing ? (
            <>
              <code>{self}</code> · editing revision <code>{short(baseRevision)}</code>. Nothing changes
              until you save.
            </>
          ) : (
            "Nothing is monitored until you save."
          )
        }
      />
      <div className="banners">
        {!writable && (
          <Notice tone="neutral" title="Configuration is read-only">
            <p>
              You can draft, test and export YAML, but saving is disabled. Place the exported YAML in the
              configuration root yourself.
            </p>
          </Notice>
        )}
        {revisionMoved && (
          <Notice tone="warn" title="Configuration changed since you opened this editor">
            <p>
              Running revision is now <code>{short(snapshot.config_revision)}</code>. Saving will be rejected
              as a conflict; your edits stay here.
            </p>
          </Notice>
        )}
        {conflict && (
          <Notice tone="warn" title="Save rejected: configuration changed" live>
            <p className="evidence">{conflict.message}</p>
            <p>
              Your edits are still here.{" "}
              {conflict.latest
                ? serviceChangedRemotely
                  ? "This service itself was changed elsewhere; compare before saving again."
                  : "This service was not changed elsewhere; other configuration was."
                : "The latest configuration could not be loaded."}
            </p>
            {conflict.latest && serviceChangedRemotely && (
              <details className="samples">
                <summary>Latest saved version of this service</summary>
                <pre className="block">
                  {latestService ? YAML.stringify(stripNulls(latestService)) : "(The service no longer exists.)"}
                </pre>
              </details>
            )}
            {conflict.latest && (
              <div className="actions">
                <button
                  type="button"
                  className="btn primary"
                  onClick={async () => {
                    await load();
                    setBaseRevision(conflict.latest!.revision);
                    setConflict(null);
                  }}
                >
                  Keep my edits on the latest revision
                </button>
                <button
                  type="button"
                  className="btn"
                  onClick={() => {
                    if (dirty && !window.confirm("Discard your edits and load the latest saved version?")) return;
                    initFrom(conflict.latest!);
                    load();
                    setConflict(null);
                  }}
                >
                  Discard my edits and reload
                </button>
              </div>
            )}
          </Notice>
        )}
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          save(false);
        }}
        noValidate
      >
        <fieldset className="card">
          <legend>Machine</legend>
          {editing ? (
            <p>
              Runs on <strong>{config.machines[draft.machineId]?.name || draft.machineId}</strong>{" "}
              <code>{draft.machineId}</code>
            </p>
          ) : (
            <>
              <div className="segmented" role="radiogroup" aria-label="Machine">
                {(["existing", "new"] as const).map((mode) => (
                  <label key={mode} className="radio">
                    <input
                      type="radio"
                      name="machine-mode"
                      value={mode}
                      checked={draft.machineMode === mode}
                      disabled={mode === "existing" && Object.keys(config.machines).length === 0}
                      onChange={() =>
                        update((d) => ({
                          ...d,
                          machineMode: mode,
                          machineId: mode === "existing" ? (Object.keys(config.machines).sort()[0] ?? "") : "",
                        }))
                      }
                    />
                    {mode === "existing" ? "Existing machine" : "New machine"}
                  </label>
                ))}
              </div>
              {draft.machineMode === "existing" ? (
                <div className="field">
                  <label htmlFor="machine-select">Machine</label>
                  <select
                    id="machine-select"
                    value={draft.machineId}
                    onChange={(e) => update((d) => ({ ...d, machineId: e.target.value }))}
                  >
                    {Object.entries(config.machines)
                      .sort(([a], [b]) => (a < b ? -1 : 1))
                      .map(([id, m]) => (
                        <option key={id} value={id}>
                          {m.name || id} ({id})
                        </option>
                      ))}
                  </select>
                </div>
              ) : (
                <div className="grid2">
                  <TextField
                    label="Machine ID"
                    value={draft.machineId}
                    onChange={(machineId) => update((d) => ({ ...d, machineId }))}
                    placeholder="jelly"
                    hint="Letters, digits, - and _. Cannot change later."
                    error={errors.machineId}
                    showError={show}
                  />
                  <TextField
                    label="Machine name"
                    value={draft.newMachine.name}
                    onChange={(name) => update((d) => ({ ...d, newMachine: { ...d.newMachine, name } }))}
                    placeholder="Jelly"
                    error={errors.machineName}
                    showError={show}
                  />
                  <TextField
                    label="Site"
                    value={draft.newMachine.site}
                    onChange={(site) => update((d) => ({ ...d, newMachine: { ...d.newMachine, site } }))}
                    placeholder="Home"
                  />
                  <TextField
                    label="Description"
                    value={draft.newMachine.description}
                    onChange={(description) =>
                      update((d) => ({ ...d, newMachine: { ...d.newMachine, description } }))
                    }
                    placeholder="Media & automation"
                  />
                </div>
              )}
            </>
          )}
          {draft.machineMode === "existing" && errors.machineId && show && (
            <p className="field-error">{errors.machineId}</p>
          )}
        </fieldset>

        <fieldset className="card">
          <legend>Service</legend>
          <div className="grid2">
            <TextField
              label="Name"
              value={draft.service.name}
              onChange={(name) => setService({ name })}
              placeholder="Jellyfin"
              error={errors.name}
              showError={show}
            />
            <TextField
              label="Service ID"
              value={draft.serviceId}
              onChange={(serviceId) => update((d) => ({ ...d, serviceId }))}
              disabled={editing}
              placeholder="jellyfin"
              hint={editing ? "IDs are fixed so history and links keep working." : "Letters, digits, - and _."}
              error={errors.serviceId}
              showError={show}
            />
          </div>
          <TextArea
            label="Description"
            value={draft.service.description}
            onChange={(description) => setService({ description })}
          />
          <IconPicker
            value={draft.service.icon}
            name={draft.service.name}
            serviceId={draft.serviceId}
            onChange={(icon) => update((d) => ({ ...d, iconTouched: true, service: { ...d.service, icon } }))}
          />
        </fieldset>

        <section className="card" aria-labelledby="checks-heading">
          <div className="sectionline">
            <h2 id="checks-heading">Checks</h2>
            <button
              type="button"
              className="btn"
              disabled={draft.checks.length === 0 || draft.checks.some((c) => tests[c.key]?.busy)}
              onClick={() => runTests(draft.checks.map((c) => c.key))}
            >
              Test all checks
            </button>
          </div>
          <p className="hint">
            Draft tests run once from the Vesper server under the saved network policy. They never open
            incidents or send notifications, and a failed test does not block saving.
          </p>
          {draft.checks.map((dc, i) => (
            <CheckEditor
              key={dc.key}
              dc={dc}
              index={i}
              defaults={config.defaults}
              errors={errors}
              showErrors={show}
              heartbeatPath={`/api/heartbeat/${draft.machineId || "<machine>"}/${draft.serviceId || "<service>"}/${dc.id || "<check>"}`}
              test={tests[dc.key]}
              onChange={(next) => update((d) => ({ ...d, checks: d.checks.map((c) => (c.key === dc.key ? next : c)) }))}
              onRemove={() => update((d) => ({ ...d, checks: d.checks.filter((c) => c.key !== dc.key) }))}
              onTest={() => runTests([dc.key])}
            />
          ))}
          {draft.checks.length === 0 && <p className="note">No checks: this service will always be Unknown.</p>}
          <div className="actions">
            {(["http", "tcp", "push"] as CheckKind[]).map((k) => (
              <button
                key={k}
                type="button"
                className="btn small"
                onClick={() => update((d) => ({ ...d, checks: [...d.checks, newCheck(d, k)] }))}
              >
                Add {k === "http" ? "HTTP" : k === "tcp" ? "TCP" : "push"} check
              </button>
            ))}
          </div>
        </section>

        <fieldset className="card">
          <legend>Notifications</legend>
          <Toggle
            checked={draft.service.notifications}
            onChange={(notifications) => setService({ notifications })}
            label="Send Telegram notifications for this service"
            hint="Incidents are still recorded when notifications are off."
          />
          <fieldset className="subgroup">
            <legend>Depends on</legend>
            <p className="hint">
              When a dependency is confirmed Down, this service's notifications are grouped with it. Checks keep
              running. This is correlation, not proven root cause.
            </p>
            {errors.depends_on && show && <p className="field-error">{errors.depends_on}</p>}
            <div className="checklist">
              {dependencyOptions.map((id) => (
                <Toggle
                  key={id}
                  checked={draft.service.depends_on.includes(id)}
                  onChange={(on) =>
                    setService({
                      depends_on: on
                        ? [...draft.service.depends_on, id].sort()
                        : draft.service.depends_on.filter((x) => x !== id),
                    })
                  }
                  label={<code>{id}</code>}
                />
              ))}
              {dependencyOptions.length === 0 && <p className="note">No other services yet.</p>}
            </div>
          </fieldset>
          <fieldset className="subgroup">
            <legend>Maintenance windows</legend>
            <p className="hint">
              Notifications are suppressed during a window; checks still run. Times are entered in your local
              time and stored in UTC.
            </p>
            {draft.service.maintenance.map((w, i) => (
              <div key={i} className="window-row">
                <TextField
                  label="Start"
                  type="datetime-local"
                  value={toLocalInput(w.start)}
                  onChange={(v) => setWindows((ws) => ws.map((x, j) => (j === i ? { ...x, start: fromLocalInput(v) } : x)))}
                  hint={w.start && !toLocalInput(w.start) ? `Stored value: ${w.start}` : undefined}
                />
                <TextField
                  label="End"
                  type="datetime-local"
                  value={toLocalInput(w.end)}
                  onChange={(v) => setWindows((ws) => ws.map((x, j) => (j === i ? { ...x, end: fromLocalInput(v) } : x)))}
                  error={errors[`maint:${i}`]}
                  showError={show}
                />
                <Toggle
                  checked={w.repeat_weekly}
                  onChange={(repeat_weekly) => setWindows((ws) => ws.map((x, j) => (j === i ? { ...x, repeat_weekly } : x)))}
                  label="Repeat weekly"
                />
                <button type="button" className="btn small" onClick={() => setWindows((ws) => ws.filter((_, j) => j !== i))}>
                  Remove window
                </button>
              </div>
            ))}
            <button
              type="button"
              className="btn small"
              onClick={() => {
                const start = new Date(Date.now() + 3600_000);
                start.setMinutes(0, 0, 0);
                const end = new Date(start.getTime() + 3600_000);
                setWindows((ws) => [
                  ...ws,
                  { start: fromLocalInput(toLocalInput(start.toISOString())), end: fromLocalInput(toLocalInput(end.toISOString())), repeat_weekly: false },
                ]);
              }}
            >
              Add maintenance window
            </button>
          </fieldset>
        </fieldset>

        <section className="card" aria-labelledby="source-heading">
          <h2 id="source-heading">Source &amp; export</h2>
          {sources.length > 0 ? (
            <ul className="plain-list mono">
              {sources.map((s) => (
                <li key={s.path}>
                  {s.file} <small className="muted">({s.path})</small>
                </li>
              ))}
            </ul>
          ) : (
            <p className="note">
              New service: the server writes it to a new or owning file in the configuration root.
            </p>
          )}
          {sources.length > 1 && (
            <p className="hint">
              This service is split across files. Saving edits those files in place; the server refuses with an
              export-only error if it cannot do so safely.
            </p>
          )}
          <details className="samples">
            <summary>YAML export (read-only)</summary>
            <p className="hint">
              The current draft. Secret values never appear here, only references. Exporting writes nothing.
            </p>
            <pre className="block" aria-label="YAML export">{yaml}</pre>
            <div className="actions">
              <button
                type="button"
                className="btn small"
                onClick={() =>
                  navigator.clipboard?.writeText(yaml).then(
                    () => setCopied(true),
                    () => setCopied(false),
                  )
                }
              >
                {copied ? "Copied" : "Copy YAML"}
              </button>
              <button type="button" className="btn small" onClick={download}>
                Download .yaml
              </button>
            </div>
          </details>
        </section>

        <div className="savebar" role="region" aria-label="Save">
          <div className="savebar-status" aria-live="polite">
            {saving ? (
              "Saving…"
            ) : show && errorCount > 0 ? (
              <ToneTag tone="bad">
                Fix {errorCount} field{errorCount === 1 ? "" : "s"} before saving
              </ToneTag>
            ) : dirty ? (
              <ToneTag tone="warn">Unsaved changes</ToneTag>
            ) : (
              <span className="muted">No unsaved changes</span>
            )}
            {saveError && (
              <span className="field-error" role="alert">
                Save failed: {saveError}
              </span>
            )}
          </div>
          <div className="actions">
            <button type="submit" className="btn primary" disabled={!writable || saving}>
              {editing && original?.paused ? "Save & resume monitoring" : "Save & monitor"}
            </button>
            <button type="button" className="btn" disabled={!writable || saving} onClick={() => save(true)}>
              Save paused
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => navigate(editing ? paths.service(machineId, serviceId) : paths.overview())}
            >
              Cancel
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}

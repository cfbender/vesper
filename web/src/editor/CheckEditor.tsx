import { ToneTag } from "../components/ui";
import { kindLabel, localTime } from "../format";
import type { Check, CheckDefaults, CheckKind, ProbeResult } from "../types";
import { newHeaderKey, type DraftCheck, type Errors } from "./draft";
import { NumberField, SecretRefField, TextField, Toggle } from "./fields";

export interface TestState {
  busy: boolean;
  at?: string;
  /** JSON of the check as tested, to flag edits made since. */
  tested?: string;
  result?: ProbeResult;
  error?: string;
}

const OVERRIDES = [
  ["interval_seconds", "Interval (s)"],
  ["timeout_seconds", "Timeout (s)"],
  ["failure_threshold", "Failures to Down"],
  ["recovery_threshold", "Passes to recover"],
  ["stale_after_seconds", "Stale after (s)"],
] as const;

export function CheckEditor({
  dc,
  index,
  defaults,
  errors,
  showErrors,
  heartbeatPath,
  test,
  onChange,
  onRemove,
  onTest,
}: {
  dc: DraftCheck;
  index: number;
  defaults: CheckDefaults;
  errors: Errors;
  showErrors: boolean;
  heartbeatPath: string;
  test?: TestState;
  onChange: (dc: DraftCheck) => void;
  onRemove: () => void;
  onTest: () => void;
}) {
  const c = dc.check;
  const p = `check:${dc.key}:`;
  const set = (patch: Partial<Check>) => onChange({ ...dc, check: { ...c, ...patch } });
  const err = (f: string) => errors[`${p}${f}`];
  const overridden = OVERRIDES.filter(([f]) => c[f] !== null).length;
  const editedSinceTest = test?.tested !== undefined && test.tested !== JSON.stringify(dc);
  const title = c.name || dc.id || `Check ${index + 1}`;
  return (
    <fieldset className="card check-editor">
      <legend>
        {title} <span className="pill">{kindLabel(c.kind)}</span>
      </legend>
      <div className="grid2">
        <TextField
          label="Check ID"
          value={dc.id}
          onChange={(id) => onChange({ ...dc, id })}
          disabled={!dc.isNew}
          hint={dc.isNew ? "Path segment, e.g. web or api" : "Existing IDs are fixed so history is kept"}
          error={err("id")}
          showError={showErrors}
        />
        <TextField label="Display name" value={c.name} onChange={(name) => set({ name })} placeholder="Optional" />
        <div className="field">
          <label htmlFor={`${dc.key}-kind`}>Check type</label>
          <select id={`${dc.key}-kind`} value={c.kind} onChange={(e) => set({ kind: e.target.value as CheckKind })}>
            <option value="http">HTTP(S) request</option>
            <option value="tcp">TCP connect</option>
            <option value="push">Push heartbeat</option>
          </select>
        </div>
        <Toggle
          checked={c.enabled}
          onChange={(enabled) => set({ enabled })}
          label="Enabled"
          hint="Disabled checks are kept in configuration but never run."
        />
      </div>

      {c.kind === "http" && (
        <>
          <TextField
            label="URL"
            type="url"
            value={c.url ?? ""}
            onChange={(url) => set({ url: url || null })}
            placeholder="https://jelly.lan:8096/health"
            error={err("url")}
            showError={showErrors}
          />
          <div className="grid2">
            <NumberField
              label="Expected status"
              value={c.expected_status}
              onChange={(v) => set({ expected_status: v ?? Number.NaN })}
              error={err("expected_status")}
              showError={showErrors}
            />
            <TextField
              label="Body must contain"
              value={c.body_contains ?? ""}
              onChange={(v) => set({ body_contains: v === "" ? null : v })}
              placeholder="Optional text"
              hint="Matched as plain text; responses are never displayed."
            />
          </div>
          <Toggle
            checked={c.follow_redirects}
            onChange={(follow_redirects) => set({ follow_redirects })}
            label="Follow redirects"
            hint="Off by default. Secrets are never forwarded to another origin."
          />
          <fieldset className="subgroup">
            <legend>Request headers</legend>
            <p className="hint">Values are secret references, never literal credentials.</p>
            {dc.headers.map((h, i) => (
              <div key={h.key} className="header-row">
                <TextField
                  label="Header name"
                  value={h.name}
                  onChange={(name) =>
                    onChange({ ...dc, headers: dc.headers.map((x, j) => (j === i ? { ...x, name } : x)) })
                  }
                  placeholder="Authorization"
                  error={err(`header:${h.key}`)}
                  showError={showErrors}
                />
                <SecretRefField
                  label={`${h.name || "Header"} value`}
                  value={h.ref}
                  onChange={(ref) =>
                    onChange({ ...dc, headers: dc.headers.map((x, j) => (j === i ? { ...x, ref } : x)) })
                  }
                />
                <button
                  type="button"
                  className="btn small"
                  onClick={() => onChange({ ...dc, headers: dc.headers.filter((_, j) => j !== i) })}
                >
                  Remove header
                </button>
              </div>
            ))}
            <button
              type="button"
              className="btn small"
              onClick={() =>
                onChange({ ...dc, headers: [...dc.headers, { key: newHeaderKey(), name: "", ref: { env: "" } }] })
              }
            >
              Add header
            </button>
          </fieldset>
        </>
      )}

      {c.kind === "tcp" && (
        <div className="grid2">
          <TextField
            label="Host"
            value={c.host ?? ""}
            onChange={(host) => set({ host: host || null })}
            placeholder="192.168.1.20"
            error={err("host")}
            showError={showErrors}
          />
          <NumberField label="Port" value={c.port} onChange={(port) => set({ port })} error={err("port")} showError={showErrors} />
        </div>
      )}

      {c.kind === "push" && (
        <>
          <div className="grid2">
            <NumberField
              label="Expected every (s)"
              value={c.expected_interval_seconds}
              onChange={(v) => set({ expected_interval_seconds: v ?? Number.NaN })}
              error={err("expected_interval_seconds")}
              showError={showErrors}
            />
            <NumberField
              label="Grace (s)"
              value={c.grace_seconds}
              onChange={(v) => set({ grace_seconds: v ?? Number.NaN })}
              hint="A missed heartbeat plus grace opens an incident."
              error={err("grace_seconds")}
              showError={showErrors}
            />
          </div>
          <SecretRefField
            label="Producer token"
            value={c.token}
            onChange={(token) => set({ token })}
            error={err("token")}
            showError={showErrors}
          />
          <p className="hint">
            The producer sends <code>POST {heartbeatPath}</code> with <code>Authorization: Bearer &lt;token&gt;</code>{" "}
            and <code>{"{\"healthy\": true}"}</code>. Vesper cannot run push checks itself.
          </p>
        </>
      )}

      <details className="subgroup" open={overridden > 0 || undefined}>
        <summary>
          Policy {overridden > 0 ? `(${overridden} override${overridden === 1 ? "" : "s"})` : "(inherits defaults)"}
        </summary>
        <p className="hint">Leave empty to inherit the configured default shown as the placeholder.</p>
        <div className="grid3">
          {OVERRIDES.map(([f, label]) => (
            <NumberField
              key={f}
              label={label}
              value={c[f]}
              onChange={(v) => set({ [f]: v })}
              placeholder={`Inherited: ${defaults[f]}`}
              error={err(f)}
              showError={showErrors}
            />
          ))}
        </div>
      </details>

      <div className="actions">
        <button type="button" className="btn" onClick={onTest} disabled={test?.busy}>
          {test?.busy ? "Testing…" : "Test this check"}
        </button>
        <button type="button" className="btn danger" onClick={onRemove}>
          Remove check
        </button>
      </div>
      <div aria-live="polite">
        {test && !test.busy && (test.result || test.error) && (
          <div className="test-result">
            {test.result ? (
              <ToneTag tone={test.result.healthy ? "good" : "bad"}>
                {test.result.healthy ? "Passed" : "Failed"} · {test.result.latency_ms} ms
              </ToneTag>
            ) : (
              <ToneTag tone="bad">Test request failed</ToneTag>
            )}
            <span className="evidence">{test.result?.message ?? test.error}</span>
            <small className="muted">
              Draft test at {localTime(test.at)}
              {editedSinceTest ? " · edited since" : ""}
            </small>
          </div>
        )}
      </div>
    </fieldset>
  );
}

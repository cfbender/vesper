import { useEffect, useId, useState, type ReactNode } from "react";
import type { SecretRef } from "../types";
import { refKind, refValue } from "./draft";

interface FieldProps {
  label: ReactNode;
  error?: string;
  hint?: ReactNode;
  /** Shown only once the user has tried to save or test. */
  showError?: boolean;
  name?: string;
}

function Shell({
  id,
  label,
  error,
  hint,
  showError = true,
  children,
}: FieldProps & { id: string; children: ReactNode }) {
  return (
    <div className={`field${error && showError ? " invalid" : ""}`}>
      <label htmlFor={id}>{label}</label>
      {children}
      {hint && (
        <p className="hint" id={`${id}-hint`}>
          {hint}
        </p>
      )}
      {error && showError && (
        <p className="field-error" id={`${id}-error`}>
          {error}
        </p>
      )}
    </div>
  );
}

function describedBy(id: string, hint: unknown, error: unknown) {
  return [hint ? `${id}-hint` : "", error ? `${id}-error` : ""].filter(Boolean).join(" ") || undefined;
}

export function TextField({
  value,
  onChange,
  placeholder,
  type = "text",
  disabled,
  autoComplete = "off",
  ...p
}: FieldProps & {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
  disabled?: boolean;
  autoComplete?: string;
}) {
  const id = useId();
  const err = p.showError !== false ? p.error : undefined;
  return (
    <Shell id={id} {...p}>
      <input
        id={id}
        name={p.name}
        type={type}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        autoComplete={autoComplete}
        spellCheck={false}
        aria-invalid={err ? true : undefined}
        aria-describedby={describedBy(id, p.hint, err)}
        onChange={(e) => onChange(e.target.value)}
      />
    </Shell>
  );
}

export function TextArea({ value, onChange, ...p }: FieldProps & { value: string; onChange: (v: string) => void }) {
  const id = useId();
  return (
    <Shell id={id} {...p}>
      <textarea id={id} name={p.name} rows={2} value={value} onChange={(e) => onChange(e.target.value)} aria-describedby={describedBy(id, p.hint, undefined)} />
    </Shell>
  );
}

/** Integer input; empty means `null` (inherit) when optional. */
export function NumberField({
  value,
  onChange,
  placeholder,
  ...p
}: FieldProps & {
  value: number | null;
  onChange: (v: number | null) => void;
  placeholder?: string;
}) {
  const id = useId();
  const [text, setText] = useState(value === null || Number.isNaN(value) ? "" : String(value));
  useEffect(() => {
    if (value !== null && !Number.isNaN(value) && Number(text) !== value) setText(String(value));
    if (value === null && text.trim() !== "") setText("");
    // Only external value changes (such as discarding edits) rewrite the text.
  }, [value]);
  const err = p.showError !== false ? p.error : undefined;
  return (
    <Shell id={id} {...p}>
      <input
        id={id}
        name={p.name}
        inputMode="numeric"
        type="text"
        pattern="[0-9]*"
        value={text}
        placeholder={placeholder}
        aria-invalid={err ? true : undefined}
        aria-describedby={describedBy(id, p.hint, err)}
        onChange={(e) => {
          setText(e.target.value);
          const t = e.target.value.trim();
          onChange(t === "" ? null : /^\d+$/.test(t) ? Number(t) : Number.NaN);
        }}
      />
    </Shell>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  hint,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: ReactNode;
  hint?: ReactNode;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="toggle">
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        aria-describedby={hint ? `${id}-hint` : undefined}
      />
      <label htmlFor={id}>{label}</label>
      {hint && (
        <p className="hint" id={`${id}-hint`}>
          {hint}
        </p>
      )}
    </div>
  );
}

/** Secret references only: an environment variable name or a file in the secrets directory. */
export function SecretRefField({
  label,
  value,
  onChange,
  error,
  showError,
}: {
  label: string;
  value: SecretRef | null;
  onChange: (v: SecretRef) => void;
  error?: string;
  showError?: boolean;
}) {
  const id = useId();
  const kind = refKind(value);
  const v = refValue(value);
  const make = (k: "env" | "file", x: string): SecretRef => (k === "env" ? { env: x } : { file: x });
  const err = showError !== false ? error : undefined;
  return (
    <fieldset className={`secret-ref${err ? " invalid" : ""}`}>
      <legend>{label}</legend>
      <div className="secret-row">
        <label className="sr" htmlFor={`${id}-k`}>
          {label} source
        </label>
        <select id={`${id}-k`} value={kind} onChange={(e) => onChange(make(e.target.value as "env" | "file", v))}>
          <option value="env">Environment variable</option>
          <option value="file">Secrets file</option>
        </select>
        <label className="sr" htmlFor={`${id}-v`}>
          {label} {kind === "env" ? "variable name" : "file name"}
        </label>
        <input
          id={`${id}-v`}
          value={v}
          spellCheck={false}
          autoComplete="off"
          placeholder={kind === "env" ? "VARIABLE_NAME" : "filename"}
          aria-invalid={err ? true : undefined}
          aria-describedby={err ? `${id}-error` : undefined}
          onChange={(e) => onChange(make(kind, e.target.value))}
        />
      </div>
      {err && (
        <p className="field-error" id={`${id}-error`}>
          {err}
        </p>
      )}
    </fieldset>
  );
}

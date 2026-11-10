import { useId, useMemo, useState } from "react";
import { api } from "../api";
import { Notice, ServiceIcon, useAction } from "../components/ui";
import { useLive } from "../data";
import type { IconEntry } from "../types";
import { suggestIcons } from "./draft";

/** Must match the server's upload limits. SVG uploads are rejected by the server. */
export const UPLOAD_TYPES = ["image/png", "image/jpeg", "image/webp"];
export const UPLOAD_MAX_BYTES = 2 * 1024 * 1024;

const GENERIC: IconEntry = { id: "server", name: "Generic server", url: "" };

export function IconPicker({
  value,
  onChange,
  name,
  serviceId,
}: {
  value: string;
  onChange: (id: string) => void;
  name: string;
  serviceId: string;
}) {
  const { iconList, icons, iconError, reloadIcons } = useLive();
  const [query, setQuery] = useState("");
  const [uploadError, setUploadError] = useState<string | null>(null);
  const upload = useAction<{ id: string; url: string }>();
  const id = useId();
  const suggestions = useMemo(() => suggestIcons(iconList, name, serviceId), [iconList, name, serviceId]);
  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return iconList.filter((i) => i.name.toLowerCase().includes(q) || i.id.toLowerCase().includes(q)).slice(0, 36);
  }, [iconList, query]);
  const current = icons.get(value) ?? (value === "server" ? GENERIC : null);

  const choice = (i: IconEntry) => (
    <button
      key={i.id}
      type="button"
      className="icon-choice"
      aria-pressed={value === i.id}
      onClick={() => onChange(i.id)}
    >
      <ServiceIcon icon={i.id} size={28} />
      <span>{i.name}</span>
    </button>
  );

  const onFile = async (file: File | undefined) => {
    setUploadError(null);
    if (!file) return;
    if (!UPLOAD_TYPES.includes(file.type)) {
      setUploadError("Choose a PNG, JPEG or WebP image. SVG uploads are not accepted.");
      return;
    }
    if (file.size > UPLOAD_MAX_BYTES) {
      setUploadError("Images must be 2 MiB or smaller.");
      return;
    }
    const r = await upload.run(() => api.uploadIcon(file));
    if (r) {
      await reloadIcons();
      onChange(r.id);
    }
  };

  return (
    <fieldset className="icon-picker">
      <legend>Icon</legend>
      <div className="icon-current">
        <ServiceIcon icon={value} size={44} />
        <div>
          <strong>{current?.name ?? value}</strong>
          <small className="muted">
            <code>{value}</code>
            {!current && " · not in the local catalog; shown as a generic icon"}
          </small>
        </div>
      </div>
      {iconError && <p className="field-error">Icon catalog unavailable: {iconError}</p>}
      {suggestions.length > 0 && (
        <>
          <p className="hint">Suggested from the local catalog</p>
          <div className="icon-options" role="group" aria-label="Suggested icons">
            {suggestions.map(choice)}
            {choice(GENERIC)}
          </div>
        </>
      )}
      <div className="field">
        <label htmlFor={`${id}-q`}>Search the catalog ({iconList.length} local icons)</label>
        <input
          id={`${id}-q`}
          type="search"
          value={query}
          placeholder="e.g. jellyfin, proxmox, grafana"
          autoComplete="off"
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      {query.trim() && (
        <div className="icon-options" role="group" aria-label="Matching icons">
          {results.map(choice)}
          {results.length === 0 && <p className="note">No local icon matches “{query}”.</p>}
        </div>
      )}
      <div className="field">
        <label htmlFor={`${id}-u`}>Upload your own</label>
        <input
          id={`${id}-u`}
          type="file"
          accept={UPLOAD_TYPES.join(",")}
          disabled={upload.busy}
          aria-describedby={`${id}-u-hint`}
          onChange={(e) => {
            onFile(e.target.files?.[0]);
            e.target.value = "";
          }}
        />
        <p className="hint" id={`${id}-u-hint`}>
          PNG, JPEG or WebP, up to 2 MiB and 2048×2048. Stored locally and re-encoded as a PNG of at most
          256 px. SVG files are not accepted.
        </p>
      </div>
      <div aria-live="polite">
        {upload.busy && <p className="note">Uploading…</p>}
        {(uploadError || upload.error) && (
          <Notice tone="bad" title="Upload failed">
            {uploadError ?? upload.error}
          </Notice>
        )}
        {upload.result && <p className="note">Uploaded as <code>{upload.result.id}</code> and selected.</p>}
      </div>
    </fieldset>
  );
}

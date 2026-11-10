import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { errorText } from "../api";
import { useLive } from "../data";
import { healthDisplay, localTime, type Tone } from "../format";
import { Link } from "../router";
import type { Health, Sample } from "../types";

/** Shape and color both carry status, so it never relies on color alone. */
export function StatusDot({ tone, size = 12 }: { tone: Tone; size?: number }) {
  return (
    <svg
      className={`dot ${tone}`}
      width={size}
      height={size}
      viewBox="0 0 12 12"
      aria-hidden="true"
      focusable="false"
    >
      {tone === "good" && <circle cx="6" cy="6" r="5.5" fill="currentColor" />}
      {tone === "warn" && (
        <>
          <circle cx="6" cy="6" r="4.75" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <path d="M6 1.25a4.75 4.75 0 0 1 0 9.5z" fill="currentColor" />
        </>
      )}
      {tone === "bad" && (
        <>
          <circle cx="6" cy="6" r="5.5" fill="currentColor" />
          <path d="M6 2.8v3.6M6 8.4v.5" stroke="var(--base)" strokeWidth="1.6" strokeLinecap="round" />
        </>
      )}
      {tone === "stale" && (
        <circle cx="6" cy="6" r="4.75" fill="none" stroke="currentColor" strokeWidth="1.5" strokeDasharray="2.4 1.6" />
      )}
      {tone === "neutral" && (
        <>
          <circle cx="6" cy="6" r="4.75" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <path d="M4.6 4v4M7.4 4v4" stroke="currentColor" strokeWidth="1.4" />
        </>
      )}
    </svg>
  );
}

export function HealthBadge({ health, live }: { health: Health; live?: boolean }) {
  const ctx = useLive();
  const d = healthDisplay(health, live ?? ctx.live);
  return (
    <span className={`tag ${d.tone}`}>
      <StatusDot tone={d.tone} size={10} />
      {d.label}
    </span>
  );
}

export function ToneTag({ tone, children }: { tone: Tone; children: ReactNode }) {
  return (
    <span className={`tag ${tone}`}>
      <StatusDot tone={tone} size={10} />
      {children}
    </span>
  );
}

function GenericIcon({ size }: { size: number }) {
  return (
    <svg
      className="service-icon generic"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="3" y="3" width="18" height="7" rx="2" />
      <rect x="3" y="14" width="18" height="7" rx="2" />
      <path d="M7 6.5h.01M7 17.5h.01M12 6.5h5M12 17.5h5" />
    </svg>
  );
}

/** Service icons come only from the local catalog or local uploads. */
export function ServiceIcon({ icon, size = 28 }: { icon: string; size?: number }) {
  const { icons } = useLive();
  const entry = icons.get(icon);
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [entry?.url]);
  const local = entry && entry.url.startsWith("/") && !entry.url.startsWith("//");
  if (!local || failed) return <GenericIcon size={size} />;
  return (
    <img
      className="service-icon"
      src={entry.url}
      width={size}
      height={size}
      alt=""
      aria-hidden="true"
      loading="lazy"
      decoding="async"
      onError={() => setFailed(true)}
    />
  );
}

export function Crumbs({ items }: { items: { label: string; to?: string }[] }) {
  return (
    <nav className="crumbs" aria-label="Breadcrumb">
      <ol>
        {items.map((item, i) => (
          <li key={i}>
            {item.to ? (
              <Link to={item.to}>{item.label}</Link>
            ) : (
              <span aria-current="page">{item.label}</span>
            )}
          </li>
        ))}
      </ol>
    </nav>
  );
}

/** Page title that receives focus after navigation so screen readers announce it. */
export function PageTitle({
  title,
  sub,
  aside,
  icon,
}: {
  title: string;
  sub?: ReactNode;
  aside?: ReactNode;
  icon?: ReactNode;
}) {
  const ref = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    document.title = `${title} · Vesper`;
    if (document.activeElement === document.body || document.activeElement === null)
      ref.current?.focus({ preventScroll: true });
  }, [title]);
  return (
    <div className="heading">
      <div className="heading-main">
        {icon}
        <div>
          <h1 ref={ref} tabIndex={-1}>
            {title}
          </h1>
          {sub && <p className="sub">{sub}</p>}
        </div>
      </div>
      {aside && <div className="heading-aside">{aside}</div>}
    </div>
  );
}

export function Notice({
  tone = "neutral",
  title,
  children,
  live = false,
}: {
  tone?: Tone;
  title?: ReactNode;
  children?: ReactNode;
  live?: boolean;
}) {
  return (
    <div className={`notice ${tone}`} role={live ? (tone === "bad" ? "alert" : "status") : undefined}>
      {title && (
        <p className="notice-title">
          <StatusDot tone={tone} size={10} /> {title}
        </p>
      )}
      {children && <div className="notice-body">{children}</div>}
    </div>
  );
}

/** Tracks one user-triggered server action and its real result. */
export function useAction<T>() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<T | null>(null);
  const run = useCallback(async (fn: () => Promise<T>) => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await fn();
      setResult(r);
      return r;
    } catch (e) {
      setError(errorText(e));
      return null;
    } finally {
      setBusy(false);
    }
  }, []);
  return { run, busy, error, result, clear: () => (setError(null), setResult(null)) };
}

export function History({ samples, label }: { samples: Sample[]; label: string }) {
  const recent = samples.slice(-40);
  const failed = recent.filter((s) => !s.healthy).length;
  if (recent.length === 0) return <p className="note">No samples recorded yet.</p>;
  return (
    <div className="history-block">
      <div
        className="history"
        role="img"
        aria-label={`${label}: ${recent.length} recent samples, ${failed} failed`}
      >
        {recent.map((s, i) => (
          <span
            key={`${s.at}-${i}`}
            className={s.healthy ? "ok" : "failed"}
            title={`${localTime(s.at)} · ${s.healthy ? "passed" : "failed"} · ${s.latency_ms} ms`}
          />
        ))}
      </div>
      <div className="legend">
        <span>
          {localTime(recent[0].at)} – {localTime(recent[recent.length - 1].at)}
        </span>
        <span>
          {recent.length - failed} passed · {failed} failed
        </span>
      </div>
      <details className="samples">
        <summary>Show samples</summary>
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th scope="col">Time</th>
                <th scope="col">Result</th>
                <th scope="col">Latency</th>
                <th scope="col">Evidence</th>
              </tr>
            </thead>
            <tbody>
              {[...recent].reverse().map((s, i) => (
                <tr key={`${s.at}-${i}`}>
                  <td>{localTime(s.at)}</td>
                  <td>
                    <ToneTag tone={s.healthy ? "good" : "bad"}>{s.healthy ? "Passed" : "Failed"}</ToneTag>
                  </td>
                  <td>{s.latency_ms} ms</td>
                  <td className="evidence">{s.message || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  );
}

let flash: string | null = null;
/** One-shot message shown on the next page, e.g. the server's save confirmation. */
export const setFlash = (message: string | null) => {
  flash = message;
};
export const takeFlash = () => {
  const m = flash;
  // Cleared after this render pass so StrictMode's double initialisation sees it too.
  if (m) setTimeout(() => flash === m && (flash = null), 0);
  return m;
};

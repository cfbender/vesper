import { useLayoutEffect, useRef, useState, type PointerEvent } from "react";
import { latency, uptimeText } from "../format";
import type { StatsHour } from "../types";

/** One chart slot: response time summary and outcome counts. `avg` null is a gap. */
export interface Slot {
  t: number;
  avg: number | null;
  min: number | null;
  max: number | null;
  passes: number;
  failures: number;
}

const HOUR = 3600_000;

/** Sums hourly rollups into `count` slots of `slotMs` starting at `from`. */
export function slotsFromHours(hours: StatsHour[], from: number, slotMs: number, count: number): Slot[] {
  const slots: Slot[] = Array.from({ length: count }, (_, i) => ({
    t: from + i * slotMs,
    avg: null,
    min: null,
    max: null,
    passes: 0,
    failures: 0,
  }));
  // Latency averages cover passing samples, so passes weight them.
  const sums = slots.map(() => ({ latency: 0, weight: 0 }));
  for (const h of hours) {
    const i = Math.floor((Date.parse(h.hour) - from) / slotMs);
    const s = slots[i];
    if (!s) continue;
    s.passes += h.passes;
    s.failures += h.failures;
    if (h.latency_avg_ms === null) continue;
    sums[i].latency += h.latency_avg_ms * h.passes;
    sums[i].weight += h.passes;
    if (h.latency_min_ms !== null) s.min = Math.min(s.min ?? Infinity, h.latency_min_ms);
    if (h.latency_max_ms !== null) s.max = Math.max(s.max ?? 0, h.latency_max_ms);
  }
  return slots.map((s, i) => ({ ...s, avg: sums[i].weight > 0 ? sums[i].latency / sums[i].weight : null }));
}

/** Start of the UTC hour containing `t`, which is also the local hour almost everywhere. */
export const hourStart = (t: number) => Math.floor(t / HOUR) * HOUR;

function niceStep(raw: number) {
  const p = 10 ** Math.floor(Math.log10(raw));
  const f = raw / p;
  return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * p;
}

function useWidth(fallback: number) {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(fallback);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(240, Math.round(e.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

const shortTime = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const shortDate = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
const dateHour = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });

/**
 * Response time line with a min–max band, failure ticks along the base and a
 * pointer readout. The summary is in the accessible label; nothing depends on hover.
 */
export function ResponseChart({
  slots,
  from,
  to,
  slotMs,
  label,
}: {
  slots: Slot[];
  from: number;
  to: number;
  /** Width of one slot; 0 for individual samples. */
  slotMs: number;
  label: string;
}) {
  const [ref, width] = useWidth(640);
  const [hover, setHover] = useState<number | null>(null);
  const height = 190;
  const pad = { l: 52, r: 10, t: 10, b: 24 };
  const plotW = width - pad.l - pad.r;
  const plotH = height - pad.t - pad.b;
  const peak = Math.max(1, ...slots.map((s) => s.max ?? s.avg ?? 0));
  const step = niceStep(peak / 3);
  const top = Math.ceil(peak / step) * step;
  const span = Math.max(1, to - from);
  const x = (t: number) => pad.l + ((t - from) / span) * plotW;
  const mid = (s: Slot) => x(s.t + slotMs / 2);
  const y = (v: number) => pad.t + plotH - (v / top) * plotH;

  const runs: Slot[][] = [];
  let run: Slot[] = [];
  for (const s of slots) {
    if (s.avg === null) {
      if (run.length) runs.push(run);
      run = [];
    } else run.push(s);
  }
  if (run.length) runs.push(run);
  const line = runs
    .map((r) => r.map((s, i) => `${i ? "L" : "M"}${mid(s).toFixed(1)},${y(s.avg!).toFixed(1)}`).join(""))
    .join("");
  const band = runs
    .filter((r) => r.length > 1 && r.every((s) => s.min !== null && s.max !== null))
    .map(
      (r) =>
        r.map((s, i) => `${i ? "L" : "M"}${mid(s).toFixed(1)},${y(s.max!).toFixed(1)}`).join("") +
        [...r].reverse().map((s) => `L${mid(s).toFixed(1)},${y(s.min!).toFixed(1)}`).join("") +
        "Z",
    )
    .join("");
  const ticks = Array.from({ length: Math.round(top / step) + 1 }, (_, i) => i * step);
  const xTicks = Math.max(2, Math.min(6, Math.floor(plotW / 110)));
  const fmt = span > 2 * 86400_000 ? shortDate : shortTime;
  const failW = slotMs ? Math.max(2, (slotMs / span) * plotW - 1) : 3;

  const onMove = (e: PointerEvent<SVGSVGElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const px = ((e.clientX - rect.left) / rect.width) * width;
    let best: number | null = null;
    slots.forEach((s, i) => {
      if (s.passes + s.failures === 0) return;
      if (best === null || Math.abs(mid(s) - px) < Math.abs(mid(slots[best]) - px)) best = i;
    });
    setHover(best);
  };
  const h = hover !== null ? slots[hover] : null;

  return (
    <div className="chart" ref={ref}>
      <p className="chart-readout" aria-hidden="true">
        {h ? (
          <>
            <b>{dateHour.format(h.t)}</b>
            {h.avg !== null && <> · avg {latency(h.avg)}</>}
            {h.min !== null && h.max !== null && slotMs > 0 && (
              <>
                {" "}
                ({latency(h.min)}–{latency(h.max)})
              </>
            )}
            {h.failures > 0 && <span className="bad"> · {h.failures} failed</span>}
          </>
        ) : (
          "\u00a0"
        )}
      </p>
      <svg
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={label}
        onPointerMove={onMove}
        onPointerLeave={() => setHover(null)}
      >
        {ticks.map((v) => (
          <g key={v} className="chart-grid">
            <line x1={pad.l} x2={width - pad.r} y1={y(v)} y2={y(v)} />
            <text x={pad.l - 8} y={y(v)} dy="0.32em" textAnchor="end">
              {latency(v)}
            </text>
          </g>
        ))}
        {Array.from({ length: xTicks }, (_, i) => from + (span * (i + 0.5)) / xTicks).map((t) => (
          <text key={t} className="chart-axis" x={x(t)} y={height - 6} textAnchor="middle">
            {fmt.format(t)}
          </text>
        ))}
        {band && <path className="chart-band" d={band} />}
        {!slotMs &&
          runs
            .filter((r) => r.length > 1)
            .map((r) => (
              <path
                key={`a${r[0].t}`}
                className="chart-band"
                d={`M${mid(r[0]).toFixed(1)},${y(0)}${r.map((s) => `L${mid(s).toFixed(1)},${y(s.avg!).toFixed(1)}`).join("")}L${mid(r[r.length - 1]).toFixed(1)},${y(0)}Z`}
              />
            ))}
        {line && <path className="chart-line" d={line} />}
        {runs
          .filter((r) => r.length === 1)
          .map(([s]) => (
            <circle key={s.t} className="chart-dot" cx={mid(s)} cy={y(s.avg!)} r={2.5} />
          ))}
        {slots
          .filter((s) => s.failures > 0)
          .map((s) => (
            <rect
              key={`f${s.t}`}
              className="chart-fail"
              x={(slotMs ? x(s.t) : x(s.t) - failW / 2).toFixed(1)}
              y={pad.t + plotH - 6}
              width={failW.toFixed(1)}
              height={6}
              rx={1}
            />
          ))}
        {h && (
          <line className="chart-cursor" x1={mid(h)} x2={mid(h)} y1={pad.t} y2={pad.t + plotH} />
        )}
      </svg>
    </div>
  );
}

/** One bar per local day: full pass, some failures, or no samples. */
export function DailyBars({ days, label }: { days: { t: number; passes: number; failures: number }[]; label: string }) {
  return (
    <div className="history daily" role="img" aria-label={label}>
      {days.map((d) => {
        const total = d.passes + d.failures;
        const tone = total === 0 ? "none" : d.failures === 0 ? "ok" : d.passes / total >= 0.99 ? "partial" : "failed";
        return (
          <span
            key={d.t}
            className={tone}
            title={`${shortDate.format(d.t)} · ${total === 0 ? "no samples" : `${uptimeText(d.passes, d.failures)} · ${d.failures} failed of ${total}`}`}
          />
        );
      })}
    </div>
  );
}

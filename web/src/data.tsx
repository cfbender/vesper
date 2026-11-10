import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { api, errorText } from "./api";
import { byId } from "./format";
import type { IconEntry, Snapshot } from "./types";

export const POLL_MS = 5000;
/** Without a successful poll for this long, the snapshot is shown as stale. */
export const STALE_MS = 15000;

interface Live {
  snapshot: Snapshot | null;
  /** Last poll failure; cleared by the next success. */
  error: string | null;
  lastSuccess: number | null;
  /** True only while the most recent poll succeeded recently. */
  live: boolean;
  now: number;
  refresh: () => Promise<void>;
  icons: Map<string, IconEntry>;
  iconList: IconEntry[];
  iconError: string | null;
  reloadIcons: () => Promise<void>;
}

const LiveContext = createContext<Live | null>(null);

/** Stable ID ordering everywhere: machines, services and checks by ID; incidents newest ID first. */
export function orderSnapshot(s: Snapshot): Snapshot {
  return {
    ...s,
    machines: [...s.machines].sort(byId).map((m) => ({
      ...m,
      services: [...m.services].sort(byId).map((sv) => ({
        ...sv,
        checks: [...sv.checks].sort(byId),
      })),
    })),
    incidents: [...s.incidents].sort((a, b) => b.id - a.id),
  };
}

export function LiveProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastSuccess, setLastSuccess] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [iconList, setIconList] = useState<IconEntry[]>([]);
  const [iconError, setIconError] = useState<string | null>(null);
  const inflight = useRef<Promise<void> | null>(null);

  const refresh = useCallback(() => {
    inflight.current ??= api
      .snapshot()
      .then(
        (s) => {
          setSnapshot(orderSnapshot(s));
          setError(null);
          setLastSuccess(Date.now());
        },
        (e) => setError(errorText(e)),
      )
      .finally(() => {
        inflight.current = null;
      });
    return inflight.current;
  }, []);

  const reloadIcons = useCallback(
    () =>
      api.icons().then(
        (r) => {
          setIconList([...r.icons].sort((a, b) => a.name.localeCompare(b.name)));
          setIconError(null);
        },
        (e) => setIconError(errorText(e)),
      ),
    [],
  );

  useEffect(() => {
    let timer: number;
    let stopped = false;
    const loop = async () => {
      await refresh();
      if (!stopped) timer = window.setTimeout(loop, POLL_MS);
    };
    loop();
    const tick = window.setInterval(() => setNow(Date.now()), 1000);
    const onVisible = () => document.visibilityState === "visible" && refresh();
    document.addEventListener("visibilitychange", onVisible);
    reloadIcons();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
      window.clearInterval(tick);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh, reloadIcons]);

  const value = useMemo<Live>(
    () => ({
      snapshot,
      error,
      lastSuccess,
      live: snapshot !== null && error === null && lastSuccess !== null && now - lastSuccess < STALE_MS,
      now,
      refresh,
      icons: new Map(iconList.map((i) => [i.id, i])),
      iconList,
      iconError,
      reloadIcons,
    }),
    [snapshot, error, lastSuccess, now, refresh, iconList, iconError, reloadIcons],
  );
  return <LiveContext.Provider value={value}>{children}</LiveContext.Provider>;
}

export function useLive(): Live {
  const v = useContext(LiveContext);
  if (!v) throw new Error("useLive outside LiveProvider");
  return v;
}

import {
  useEffect,
  useSyncExternalStore,
  type AnchorHTMLAttributes,
  type MouseEvent,
} from "react";

export type Route =
  | { name: "overview" }
  | { name: "machine"; machine: string }
  | { name: "service"; machine: string; service: string }
  | { name: "new-service"; machine: string | null }
  | { name: "edit-service"; machine: string; service: string }
  | { name: "incidents" }
  | { name: "incident"; id: number }
  | { name: "settings" }
  | { name: "not-found" };

export function parseRoute(pathname: string, search: string): Route {
  const parts = pathname.split("/").filter(Boolean).map(decodeURIComponent);
  const [a, b, c, d] = parts;
  if (parts.length === 0) return { name: "overview" };
  if (a === "machines" && parts.length === 2) return { name: "machine", machine: b };
  if (a === "services" && b === "new" && parts.length === 2)
    return { name: "new-service", machine: new URLSearchParams(search).get("machine") };
  if (a === "services" && parts.length === 3) return { name: "service", machine: b, service: c };
  if (a === "services" && parts.length === 4 && d === "edit")
    return { name: "edit-service", machine: b, service: c };
  if (a === "incidents" && parts.length === 1) return { name: "incidents" };
  if (a === "incidents" && parts.length === 2 && /^\d+$/.test(b))
    return { name: "incident", id: Number(b) };
  if (a === "settings" && parts.length === 1) return { name: "settings" };
  return { name: "not-found" };
}

const seg = encodeURIComponent;
export const paths = {
  overview: () => "/",
  machine: (m: string) => `/machines/${seg(m)}`,
  service: (m: string, s: string) => `/services/${seg(m)}/${seg(s)}`,
  newService: (m?: string) => (m ? `/services/new?machine=${seg(m)}` : "/services/new"),
  editService: (m: string, s: string) => `/services/${seg(m)}/${seg(s)}/edit`,
  incidents: () => "/incidents",
  incident: (id: number) => `/incidents/${id}`,
  settings: () => "/settings",
};

/** Splits a full service ID (`machine/service`) into its parts. */
export function splitServiceId(id: string): [string, string] {
  const at = id.indexOf("/");
  return at < 0 ? [id, ""] : [id.slice(0, at), id.slice(at + 1)];
}

const listeners = new Set<() => void>();
let blocker: string | null = null;
let current = location.pathname + location.search;

function notify() {
  current = location.pathname + location.search;
  listeners.forEach((l) => l());
}

window.addEventListener("popstate", () => {
  const next = location.pathname + location.search;
  if (blocker && next !== current && !window.confirm(blocker)) {
    history.pushState(null, "", current);
    return;
  }
  notify();
});

/** Navigates within the app; returns false when the user kept unsaved edits. */
export function navigate(to: string, { replace = false, force = false } = {}): boolean {
  if (!force && blocker && to !== current && !window.confirm(blocker)) return false;
  if (replace) history.replaceState(null, "", to);
  else history.pushState(null, "", to);
  window.scrollTo(0, 0);
  notify();
  return true;
}

export function useLocationKey(): string {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => current,
  );
}

export function useRoute(): Route {
  const key = useLocationKey();
  const [pathname, search = ""] = key.split("?");
  return parseRoute(pathname, search ? `?${search}` : "");
}

/** While `message` is set, leaving the page asks for confirmation. */
export function useNavigationBlocker(message: string | null) {
  useEffect(() => {
    if (!message) return;
    blocker = message;
    const onUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", onUnload);
    return () => {
      if (blocker === message) blocker = null;
      window.removeEventListener("beforeunload", onUnload);
    };
  }, [message]);
}

export function Link({
  to,
  onClick,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement> & { to: string }) {
  const handle = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    )
      return;
    event.preventDefault();
    navigate(to);
  };
  return <a href={to} onClick={handle} {...rest} />;
}

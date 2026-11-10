import type {
  Check,
  ConfigResponse,
  IconEntry,
  Machine,
  ProbeResult,
  SaveResponse,
  Service,
  Snapshot,
} from "./types";

/** A failed API call. `message` is the server's safe `{error}` text when present. */
export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

let csrfToken: Promise<string> | null = null;

function fetchCsrf(): Promise<string> {
  csrfToken ??= request<{ token: string }>("GET", "/api/csrf").then(
    (body) => body.token,
    (error) => {
      csrfToken = null;
      throw error;
    },
  );
  return csrfToken;
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
  csrf?: string,
): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json" };
  let payload: BodyInit | undefined;
  if (body instanceof FormData) payload = body;
  else if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    payload = JSON.stringify(body);
  }
  if (csrf) headers["X-Vesper-CSRF"] = csrf;
  let response: Response;
  try {
    // Same-origin requests: the browser supplies Origin on writes and the
    // authentication proxy's cookies.
    response = await fetch(path, {
      method,
      headers,
      body: payload,
      credentials: "same-origin",
      mode: "same-origin",
      cache: "no-store",
    });
  } catch {
    throw new ApiError("Could not reach the Vesper server.", 0);
  }
  const text = await response.text();
  let json: unknown = undefined;
  if (text) {
    try {
      json = JSON.parse(text);
    } catch {
      json = undefined;
    }
  }
  if (!response.ok) {
    const error =
      json && typeof json === "object" && "error" in json && typeof json.error === "string"
        ? json.error
        : `Request failed with HTTP ${response.status}.`;
    throw new ApiError(error, response.status);
  }
  if (json === undefined) {
    throw new ApiError("The server returned an unreadable response.", response.status);
  }
  return json as T;
}

/** Management writes carry the CSRF token; a rejected token is refreshed once. */
async function mutate<T>(method: string, path: string, body?: unknown): Promise<T> {
  const token = await fetchCsrf();
  try {
    return await request<T>(method, path, body, token);
  } catch (error) {
    if (!(error instanceof ApiError) || error.status !== 403) throw error;
    csrfToken = null;
    return request<T>(method, path, body, await fetchCsrf());
  }
}

const seg = encodeURIComponent;

export const api = {
  snapshot: () => request<Snapshot>("GET", "/api/snapshot"),
  config: () => request<ConfigResponse>("GET", "/api/config"),
  icons: () => request<{ icons: IconEntry[] }>("GET", "/api/icons"),
  reloadConfig: () => mutate<Record<string, unknown>>("POST", "/api/config/reload"),
  saveService: (machine: string, service: string, revision: string, body: Service) =>
    mutate<SaveResponse>("PUT", `/api/services/${seg(machine)}/${seg(service)}`, {
      revision,
      service: body,
    }),
  saveMachine: (machine: string, revision: string, body: Machine) =>
    mutate<SaveResponse>("PUT", `/api/machines/${seg(machine)}`, { revision, machine: body }),
  draftTest: (checks: Record<string, Check>) =>
    mutate<{ results: Record<string, ProbeResult> }>("POST", "/api/draft/test", { checks }),
  recheck: (machine: string, service: string) =>
    mutate<unknown>("POST", `/api/services/${seg(machine)}/${seg(service)}/recheck`),
  silence: (machine: string, service: string, until: string | null) =>
    mutate<unknown>("POST", `/api/services/${seg(machine)}/${seg(service)}/silence`, { until }),
  acknowledge: (incident: number, acknowledged: boolean) =>
    mutate<unknown>("POST", `/api/incidents/${incident}/acknowledge`, { acknowledged }),
  testTelegram: () => mutate<{ message: string }>("POST", "/api/telegram/test"),
  uploadIcon: (file: File) => {
    const form = new FormData();
    form.append("file", file);
    return mutate<{ id: string; url: string }>("POST", "/api/icons", form);
  },
};

export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : "Something went wrong.";
}

/** Test hook: forget the cached CSRF token. */
export function resetCsrfForTests() {
  csrfToken = null;
}

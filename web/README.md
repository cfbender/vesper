# Vesper web

React/TypeScript interface bundled into the Vesper server. It talks only to the
real HTTP API described in `docs/architecture.md`; there is no mock or simulated
data in the application bundle.

```sh
npm ci
npm run build       # typecheck + production bundle in dist/
npm test            # vitest
npm run dev         # Vite dev server; proxies /api to $VESPER_API (default http://127.0.0.1:8080)
```

The server serves `dist/` and falls back to `index.html` for the UI routes
(`/`, `/machines/:m`, `/services/:m/:s`, `/services/new`,
`/services/:m/:s/edit`, `/incidents`, `/incidents/:id`, `/settings`).

## Icons

`public/icons/catalog.json` is `{ "icons": [{ "id", "name", "url" }] }` with
local `/icons/dashboard/<id>.png` URLs. The PNGs are a curated subset of Homarr
Labs Dashboard Icons (Apache-2.0); see `public/icons/dashboard/NOTICE.md` and
`LICENSE`. Unknown icon IDs (including the default `server`) render as a
built-in generic glyph. Uploads accept PNG, JPEG or WebP up to 2 MiB.

## Test-only fixtures

`test/fixtures.ts` and `test/fixture-server.ts` are illustrative test data and
an in-memory imitation of the API, used by unit tests and for rendering
screenshots before a real server is available. They are never imported by
`src/`. Run `npm run build && npm run fixture-server` (port 4173).

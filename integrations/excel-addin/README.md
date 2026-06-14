# FinX Excel / Office Add-in

Office.js custom functions that pull data from the FinX platform REST API
straight into an Excel grid:

| Function | Purpose |
| --- | --- |
| `FINX.GET(route, params, options?)` | Fetch a catalog route — `GET {baseUrl}/api/v1/{route}?{params}` — and spill the response as a 2-D grid. The `OBB.GET`-equivalent over the FinX REST contract. |
| `FINX.BYOD(baseUrl, route, params, options?)` | Bring-your-own-data: the same call against an explicit base URL, bypassing the saved config so a sheet can target an arbitrary FinX-compatible backend. |
| `FINX.ROUTES()` | List the catalog routes the configured backend exposes (calls `meta/routes`), one per row. |

This is the **deliberate non-Rust surface** of the FinX platform (eco wave
G009). Per the ecosystem gap map (`docs/roadmap/openbb-ecosystem-gap.md`), an
Excel add-in is inherently an Office.js / TypeScript client over the existing
REST API, so it lives here as a self-contained TypeScript package rather than a
Rust crate. It consumes the public REST contract only — it adds no new server
capability and is invisible to the Cargo workspace (no `Cargo.toml`).

## How it works

All real logic lives in `src/lib/` as pure, framework-free modules:

| Module | Responsibility |
| --- | --- |
| `url.ts` | Route normalization, query-param encoding, full URL building. |
| `client.ts` | REST fetch orchestration over an injectable `fetch`. |
| `flatten.ts` | `ResultEnvelope.results` → 2-D grid, field projection, row/column caps, header toggle, transpose. |
| `options.ts` | Parse the worksheet `options` argument into structured slicing. |
| `errors.ts` | Map thrown errors / non-2xx responses to a readable cell string. |
| `config.ts` | Validate the saved base URL / API key. |

The Office.js custom-function wrappers in `src/functions/functions.ts` are thin:
they resolve config, parse the loose worksheet args, delegate to `src/lib/`, and
map any failure to a cell value. Because the logic is in `src/lib/`, the unit
tests in `test/` exercise it with no Excel/Office runtime.

### Slicing options

`options` accepts an object or a `key=value;...` string:

```
=FINX.GET("equity/price/historical","symbol=AAPL","fields=date,close;rows=30")
=FINX.GET("equity/quote","symbol=AAPL","transpose=true")
```

Supported keys: `fields` (comma list), `rows`, `columns`/`cols`, `header`
(`true`/`false`), `transpose` (`true`/`false`).

## Configure

Open the **FinX Settings** task pane (ribbon button) and set:

- **Base URL** — e.g. `https://api.example.com` (the host serving the FinX REST
  API; must start with `http(s)://`).
- **API key** *(optional)* — sent as `Authorization: Bearer <key>`.

Values persist in the shared `OfficeRuntime.storage` the custom functions read.

## Build

```bash
npm install
npm run typecheck   # tsc --noEmit
npm run lint        # eslint
npm test            # vitest (pure-lib unit tests)
npm run build       # esbuild → dist/functions.js, dist/taskpane.js
```

## Host & sideload

The add-in is static: host the built `dist/` bundles plus the HTML pages
(`src/functions/functions.html`, `src/taskpane/taskpane.html`) and the
`functions.json` metadata behind an HTTPS origin, then point the placeholder
URLs in `manifest.xml` at that origin (replace every `https://localhost:3000`).
Also replace the placeholder `<Id>` GUID with your own.

Sideload `manifest.xml`:

- **Windows (Excel desktop):** put `manifest.xml` in a trusted network share and
  add the share under *File → Options → Trust Center → Trusted Add-in Catalogs*,
  or use the [office-addin-debugging](https://learn.microsoft.com/office/dev/add-ins/testing/sideload-office-add-ins-for-testing)
  tooling.
- **Mac (Excel desktop):** copy `manifest.xml` into
  `~/Library/Containers/com.microsoft.Excel/Data/Documents/wef`.
- **Excel on the web:** *Insert → Add-ins → Upload My Add-in* and choose
  `manifest.xml`.

Once sideloaded, the `FINX.*` functions are available in any cell and the
**FinX Settings** task pane appears on the Home ribbon tab.

## Clean-room note

This package was built only against the public Office.js custom-functions API
and the FinX platform's own REST contract. No third-party add-in source was
copied, and no `OBB.`-named functions are defined (that namespace belongs to
OpenBB). The `FINX` / `@finx` branding is the platform's own.

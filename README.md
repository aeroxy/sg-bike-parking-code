# SG Bike Parking QR Finder

Find the nearest bike-share parking QR sticker in Singapore — and help map the
ones nobody has found yet.

**Live app: <https://aero.oops.wtf/p/sg-bike-qr>**

## The problem

Every shared bicycle in Singapore has to be parked at a designated spot, and
ending a rental means scanning a physical QR sticker at that spot. The stickers
are small, unlabelled on any map, and often not where you expect — so you end up
wandering around looking for one while the meter runs.

LTA publishes bicycle *rack* locations, but nothing maps the individual QR
sticker codes (`ABC-1234`) to coordinates. This app crowdsources that missing
layer: report a code once, and everyone can find it afterwards.

## How it works

- **Browse** — the map shows known QR stickers near you, or near any place you
  search for. No sign-in needed to look around.
- **Report** — stand next to a sticker, scan it with your camera (or type the
  code), and submit. Your GPS position is attached to the report.
- **Self-correcting** — every report of the same code is stored separately, and
  the pin you see is the **median** of all of them. One person's bad GPS fix
  gets outvoted as more people report the same sticker; the pin gets more
  accurate over time rather than being overwritten by whoever reported last.

Codes are validated as three letters plus four characters (`ABC-1234`). The
dash is inserted for you — just type or scan.

## This is an Aero app

[Aero](https://aero.oops.wtf) is an app-building and hosting platform where each
app is a single self-contained HTML page published at `/p/{name}`. This project
is one such page — [`index.html`](index.html), about a thousand lines with all
CSS and JavaScript inlined.

There is **no backend in this repository**, and no build step, bundler, or
framework. Aero provides the server side:

- **aeroDB** — the page's own database, reached from the browser via
  `<script src="/aero-db.js">`. No API keys, no config, no SDK to install:
  requests go same-origin and ride the visitor's Aero session cookie (or an
  automatic anonymous guest cookie). Access rules are configured per table on
  the platform, not in this code.
- **Geo queries** — aeroDB's `near` query returns rows within a radius, sorted
  nearest-first with distances already computed, so the app never has to fetch
  the whole table and filter client-side.

Because the data client uses same-origin requests, **the page only reaches
aeroDB when served from Aero itself**. Opening `index.html` from disk or hosting
it elsewhere won't connect — see Local development below.

The visual design follows Aero's house style: warm paper, near-black ink, one
orange accent for things you can click and blue for things that are true.

### Data model

Two tables, both configured on the Aero side:

| Table | Contents | Read | Write | Geo |
|---|---|---|---|---|
| `qr_scans` | One row per individual report: code, lat/lng, GPS accuracy, timestamp | public | own | — |
| `qr_locations` | One row per code (row id *is* the code): median position, report count, last-reported time | public | any | **yes** |

`qr_locations` must be created with geo enabled **before its first write** —
rows written earlier never get a geohash and stay invisible to `near` queries
forever, with no backfill.

## Third-party pieces

Everything else is loaded from a CDN or called at runtime; nothing is vendored:

- [Leaflet](https://leafletjs.com/) for the map
- [OpenStreetMap](https://www.openstreetmap.org/copyright) raster tiles
- [Nominatim](https://nominatim.org/) for place-name search — debounced, capped
  at one in-flight request, and cached per session to stay within their usage
  policy
- The browser's native `BarcodeDetector` for QR scanning — no scanning library.
  It isn't available on iOS Safari, which is why manual entry is always
  present rather than a fallback you have to find.

## Local development

Open `index.html` in a browser and it runs against an **in-memory mock
database** seeded with a few fake pins near Raffles Place — enough to exercise
the map, markers, popups, search, validation and the median maths without
publishing anything. A banner makes it obvious that nothing is being saved.

The mock mirrors the aeroDB client surface, including `near` queries with
distances, so the same application code runs in both modes.

What can't be tested this way: real data, and the camera scanner (which needs a
device with a camera and `BarcodeDetector` — Chrome on Android).

## Publishing

```
cp .env.example .env      # then paste a token from https://aero.oops.wtf/connect
./deploy.sh --check       # verify size and digest, publish nothing
./deploy.sh               # publish
```

[`deploy.sh`](deploy.sh) talks to Aero's MCP server, but the document itself
never travels as an MCP argument. It mints a one-shot upload token with
`create_page_upload` and then `PUT`s the bytes over HTTP.

That split matters because `index.html` is ~42 KB. MCP arguments are JSON an
agent authors, so publishing inline means re-emitting the whole document token
by token, with no integrity check at the end — a character lost along the way
publishes live and reports success. `create_page_upload` commits the file's
sha256 **before any bytes move**, so a corrupted body is rejected with 422 and
nothing is published. Uploads otherwise behave exactly like `update_page`:
same validation, same side effects, same entry in the page's version history.

Transfer tokens are **owner-only**. If you're a contributor rather than the
page's owner, `deploy.sh` won't work for you — send a pull request instead.

### Checking for drift

`--check` also answers "is the deployed page still what's in git?" — it reports
the live page's digest alongside the local one and never transfers the
document:

```json
{"sha256":"eb60b46f…","current_sha256":"eb60b46f…","changed":false,"would_publish":true}
```

`changed: false` means the live page is already identical to your working tree.

### Table access

Table access is **not** exposed over MCP today — after the first publish,
configure both tables once from the Aero web UI (Apps → the page → Database), or from the browser
console while signed in:

```js
await fetch('/api/db/sg-bike-qr/tables/qr_scans', {
  method: 'PUT', credentials: 'same-origin',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ read_access: 'public', write_access: 'own' }),
}).then(r => r.json());

await fetch('/api/db/sg-bike-qr/tables/qr_locations', {
  method: 'PUT', credentials: 'same-origin',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ read_access: 'public', write_access: 'any', geo: true }),
}).then(r => r.json());
```

## Known limitations

- **No moderation.** Reports are sanity-checked client-side only — GPS accuracy
  better than 100 m, coordinates inside Singapore, and a valid code format.
  There's no way to flag or remove a bad pin yet; the median makes bad reports
  less damaging, not impossible.
- **Dense areas may return partial results.** A `near` query in a busy spot can
  hit its scan budget and silently drop codes; zoom in for the full list.
- **No offline support.** Tiles, search and data all need a connection.

## Contributing

The whole app is one file. Edit [`index.html`](index.html), open it in a browser
to check your change against the mock database, and publish.

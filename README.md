# SG Bike Parking QR Finder

Find the nearest bike-share parking QR sticker in Singapore — and help map the ones nobody has found yet.

**Live app: <https://sg-bike-qr.vercel.app>**

## The problem

Every shared bicycle in Singapore has to be parked at a designated spot, and ending a rental means scanning a physical QR sticker at that spot. The stickers are small, unlabelled on any map, and often not where you expect — so you end up wandering around looking for one while the meter runs.

LTA publishes bicycle *rack* locations, but nothing maps the individual QR sticker codes (`ABC-1234`) to coordinates. This app crowdsources that missing layer: report a code once, and everyone can find it afterwards.

## How it works

- **Browse** — the map shows known QR stickers near you, or near any place you search for. Seeing codes needs a free sign-in with [Aero](https://aero.oops.wtf).
- **Riding with** — pick the operator you're riding with, and the map leaves out stickers that can't end your ride.
- **Report** — stand next to a sticker, scan it with your camera (or type the code), and submit. Your GPS position is attached to the report.
- **Self-correcting** — every report of the same code is stored separately, and the pin you see is the **median** of all of them. One person's bad GPS fix gets outvoted as more people report the same sticker; the pin gets more accurate over time rather than being overwritten by whoever reported last.
- **Chip in** — the footer opens a PayNow QR any Singapore bank app can scan, for any amount, with a card option for everyone else.

### Whose sticker is it

LTA's QR code parking system puts a sticker at every bicycle parking zone, and any operator's bike can end a ride there. Some operators also print stickers of their own for zones only their bikes may use. The code's shape tells them apart:

| Sticker | Code | Example | Rides that can end there |
|---|---|---|---|
| LTA | 3 letters + 4 letters or digits | `ABC-1234` | any operator |
| HelloRide | 4 letters + 3 digits | `GRAB-001` | HelloRide only |

Without the dash the two can be confused (`GRAB001` is also `GRA-B001`), so the report form asks which sticker it is, then places the dash itself. Anywheel may have stickers of its own too; nobody has seen one yet. Supporting a new kind is one row in `FORMATS` in [`src/codes.rs`](src/codes.rs): the page builds its sticker switch and code checks from that list. A scanned QR code's text is kept with its report, so the first one of a new kind shows what it looks like.

## How it's built

```
public/index.html      the whole front end: one page, CSS and JavaScript inlined
api/server.rs          the API as one Vercel Function (Rust, axum)
src/                   the API itself: codes.rs (sticker formats), geo.rs, auth.rs, lib.rs (routes)
scripts/paynow-qr.mjs  makes the chip-in PayNow QR at build time
aero-app/              the page at the app's old Aero address, which sends visitors here
scripts/export.py      the one-time export from Aero's Firestore, at the move
```

- **Data** lives in MongoDB Atlas. The API is the only client; the browser never sees a connection string.
- **Sign-in** is [Login with Aero](https://aero.oops.wtf/connect): Aero's consent screen sends the browser back to `/auth/aero` with an encrypted token (PASETO `v4.local`) holding the visitor's email. The page keeps it as an opaque string and sends it as a bearer token; the API decrypts it on each call. The page takes the token out of the address bar before anything else runs, analytics included.
- **Geo queries** use MongoDB's `2dsphere` index: `$geoNear` returns the stickers within the map's radius, nearest first, with their distances.
- **The PayNow QR** is a static file, `public/paynow.png`, which [`scripts/paynow-qr.mjs`](scripts/paynow-qr.mjs) makes at build time from `PAYNOW_NUMBER`. The number lives in the deployment's environment rather than in this public repository (the PNG is gitignored), and the page shows the code alone, not the number. It is a static PayNow code that leaves the amount to the payer and marks the transfer `DONATION`. Without the variable no QR is made, and the sheet offers the card link only.

### Data model

| Collection | Contents |
|---|---|
| `qr_scans` | One document per report: `code`, `lat`, `lng`, GPS `accuracy`, `source` (`scan` or `manual`), `raw` (a scanned QR code's text), `clientTimestamp`, `createdAt`, `_owner` (the reporter's email) |
| `qr_locations` | One document per code, `_id` being the code: the median `lat`/`lng` and a GeoJSON `location` for the index, `operator` (`lta` or `helloride`), `numReports`, `lastReportAt`, `createdAt`, `updatedAt`, `_owner` (the first reporter) |

The API writes `qr_locations` itself, recomputing a pin each time its code is reported; nobody can set a pin directly. No response carries `_owner`.

### API

| Method | Path | Sign-in | |
|---|---|---|---|
| `GET` | `/api/formats` | — | the sticker formats |
| `GET` | `/api/near?lat&lng&radius_m[&riding]` | yes | pins within the radius (250 m–50 km), nearest first, at most 50; `riding` is `any`, `helloride` or `anywheel` |
| `POST` | `/api/scans` | yes | a report: `{code, lat, lng, accuracy, source, raw?, clientTimestamp?}`; answers with the code's pin as it now stands |
| `GET` | `/api/sign-in?state=` | — | sends the browser to Aero's consent screen |

## Third-party pieces

Loaded from a CDN or called at runtime; nothing is vendored:

- [Leaflet](https://leafletjs.com/) for the map
- [OpenStreetMap](https://www.openstreetmap.org/copyright) raster tiles
- [Nominatim](https://nominatim.org/) for place-name search — debounced, capped at one in-flight request, and cached per session to stay within their usage policy
- The browser's native `BarcodeDetector` for QR scanning — no scanning library. It isn't available on iOS Safari, which is why manual entry is always present rather than a fallback you have to find.
- Google Analytics, the same property Aero reports into
- [`qrcode`](https://www.npmjs.com/package/qrcode), at build time only, for the PayNow QR

## Local development

Open `public/index.html` straight from disk and it runs on built-in sample data near Raffles Place: enough to exercise the map, pins, popups, search, the sticker switch, validation and the median maths without a server. A banner makes it obvious that nothing is being saved.

The page and the API together, against the database in `.env`:

```
cp .env.example .env       # then fill it in
bun install
bun scripts/paynow-qr.mjs  # the chip-in QR, from PAYNOW_NUMBER in .env
cargo run --bin dev        # http://127.0.0.1:8787
cargo test
```

That database is the real one, so mind what you report. Signing in ends at the deployed callback, so the local page can't sign in by itself.

## Deploying

1. Import this repository as a Vercel project named `sg-bike-qr`. `vercel.json` does the rest: Vercel runs `scripts/paynow-qr.mjs` as the build command, serves `public/`, builds `api/server.rs` as a function in Singapore (`sin1`, next to the Atlas cluster), and rewrites `/api/*` to it.
2. Set `MONGO_DB`, `AERO_CLIENT_ID`, `AERO_LOGIN_KEY` and, optionally, `PAYNOW_NUMBER` in the project's environment variables (see [`.env.example`](.env.example)). The Atlas user should be able to read and write this app's database and nothing else; the API creates its indexes on start.
3. Push to `main`.

Login with Aero is registered with the callback `https://sg-bike-qr.vercel.app/auth/aero`. A registration can't be edited, so another address means registering again for a new client id and key.

### The old Aero address

The app used to live at `https://aero.oops.wtf/p/sg-bike-qr`, on Aero's page database. [`aero-app/index.html`](aero-app/index.html) now sits there and sends visitors here; [`aero-app/deploy.sh`](aero-app/deploy.sh) publishes it (or any other file) with the `AERO_TOKEN` from `.env`:

```
aero-app/deploy.sh --check   # verify size and digest, publish nothing
aero-app/deploy.sh           # publish
```

### Moving the data

The reports and pins came over from Aero's Firestore once, at the move: [`scripts/export.py`](scripts/export.py) prints both tables (run it wherever gcloud is signed in with access, such as Cloud Shell), and `cargo run --bin import -- export.ndjson` checks the export, writes it into MongoDB and reads it all back to compare. The import never deletes, and it refuses to run once the new app has taken reports, since it would move their pins back. The export holds reporters' emails, so it doesn't belong in this repository.

## Known limitations

- **No moderation.** Reports are checked for a GPS fix within 100 m, a position inside Singapore and a valid code, but there's no way to flag or remove a bad pin yet; the median makes bad reports less damaging, not impossible.
- **Anywheel-only stickers** can't be reported until someone has seen one.
- **No offline support.** Tiles, search and data all need a connection.

## Contributing

The front end is one file, [`public/index.html`](public/index.html), and the API is a small Rust crate. Open the page from disk to check a change against the sample data, run `cargo test`, and send a pull request.

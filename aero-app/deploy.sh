#!/usr/bin/env bash
# Publish a page to the app's old Aero address, /p/sg-bike-qr. Since the move
# to Vercel that is index.html here: a page that sends visitors on to
# https://sg-bike-qr.vercel.app. Any other file can go there instead, such as
# the old app from git history, to undo the move.
#
# The document moves over HTTP, not as an MCP argument. create_page_upload
# commits the file's sha256 at mint time, before any bytes move, so the server
# can prove what arrived is what we measured: on mismatch it returns 422 and
# publishes nothing. An inline update_page has no such check — a document
# corrupted in transit goes live and reports success.
#
#   ./deploy.sh                 publish index.html
#   ./deploy.sh FILE            publish FILE
#   ./deploy.sh --check [FILE]  verify size and digest against the server, publish nothing
#
set -euo pipefail

DRY=""
FILE="index.html"
for arg in "$@"; do
  if [ "$arg" = "--check" ]; then DRY=1; else FILE="$(cd "$(dirname "$arg")" && pwd)/$(basename "$arg")"; fi
done

cd "$(dirname "$0")"

# AERO_TOKEN comes from the repository's .env (gitignored), or from the environment.
if [ -f ../.env ]; then
  set -a
  . ../.env
  set +a
fi

PAGE="${AERO_PAGE:-sg-bike-qr}"
MCP_URL="${AERO_MCP_URL:-https://aero.oops.wtf/mcp}"
TOKEN="${AERO_TOKEN:-}"

if [ -z "$TOKEN" ]; then
  echo "no aero token — put AERO_TOKEN=aero_... in .env (see .env.example)" >&2
  echo "mint one at https://aero.oops.wtf/connect" >&2
  exit 1
fi

python3 - "$MCP_URL" "$TOKEN" "$PAGE" "$DRY" "$FILE" <<'PY'
import hashlib, json, pathlib, ssl, sys, urllib.error, urllib.request

mcp_url, token, page, dry, file = sys.argv[1:6]

body = pathlib.Path(file).read_bytes()
sha, size = hashlib.sha256(body).hexdigest(), len(body)
print(f"==> {pathlib.Path(file).name} — {size} bytes, sha256 {sha[:12]}")

# The local network does TLS interception; verification would fail here the
# same way `curl` needs -k.
ctx = ssl._create_unverified_context()


def mcp(name, arguments):
    req = urllib.request.Request(
        mcp_url,
        method="POST",
        data=json.dumps({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        },
    )
    with urllib.request.urlopen(req, context=ctx) as res:
        payload = json.loads(res.read().decode())
    result = payload.get("result", {})
    text = "".join(c.get("text", "") for c in result.get("content", []))
    if result.get("isError") or "error" in payload:
        sys.exit(f"    {name} FAILED: {text or payload.get('error')}")
    return json.loads(text)


mint = mcp("create_page_upload", {"page": page, "sha256": sha, "bytes": size})
print(f"==> upload token minted, expires {mint['expires_at']}")

# A dry run does not spend the token, so --check costs nothing but a round trip.
url = mint["upload_url"] + ("?dry_run=1" if dry else "")
req = urllib.request.Request(
    url,
    method="PUT",
    data=body,
    headers={
        "Authorization": f"Bearer {mint['token']}",
        "Content-Type": "text/html",
    },
)
try:
    with urllib.request.urlopen(req, context=ctx) as res:
        print(f"    {res.read().decode().strip()[:300]}")
except urllib.error.HTTPError as e:
    sys.exit(f"    upload FAILED {e.code}: {e.read().decode().strip()[:300]}")
PY

if [ -n "$DRY" ]; then
  echo "==> checked only — nothing published"
else
  echo "==> done — https://aero.oops.wtf/p/$PAGE"
fi

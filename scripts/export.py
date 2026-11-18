"""Prints a page's Aero database tables (qr_scans, qr_locations) from
Firestore, one document per line, for `cargo run --bin import`.

Read-only. Runs anywhere gcloud is signed in as someone who can read the
Firestore database, such as Cloud Shell, and reads each document once (about
1.3k reads of the free tier's 50k a day, at the time of the move):

    python3 export.py PROJECT DATABASE PAGE_ID > export.ndjson

The output holds reporters' emails (Aero stamps each row's `_owner`), so it
belongs nowhere public: not in this repository.
"""

import json
import subprocess
import sys
import urllib.error
import urllib.request

TABLES = ("qr_scans", "qr_locations")
PAGE_SIZE = 300


def main(project, database, page_id):
    token = subprocess.check_output(["gcloud", "auth", "print-access-token"], text=True).strip()
    base = f"https://firestore.googleapis.com/v1/projects/{project}/databases/{database}/documents/pages/{page_id}"
    for table in TABLES:
        count, page_token = 0, ""
        while True:
            url = f"{base}/{table}?pageSize={PAGE_SIZE}" + (f"&pageToken={page_token}" if page_token else "")
            request = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
            try:
                with urllib.request.urlopen(request) as response:
                    page = json.load(response)
            except urllib.error.HTTPError as err:
                sys.exit(f"{table}: HTTP {err.code}: {err.read().decode()[:300]}")
            for document in page.get("documents", []):
                print(json.dumps({
                    "collection": table,
                    "id": document["name"].rsplit("/", 1)[1],
                    "createTime": document["createTime"],
                    "updateTime": document["updateTime"],
                    "fields": document.get("fields", {}),
                }))
                count += 1
            page_token = page.get("nextPageToken", "")
            if not page_token:
                break
        print(f"{table}: {count} documents", file=sys.stderr)


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])

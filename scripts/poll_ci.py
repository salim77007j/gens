#!/usr/bin/env python3
"""Poll GitHub Actions for the latest run of a branch until it completes.

Usage: GH_TOKEN=... python3 scripts/poll_ci.py <branch> [timeout_secs]
Exit code 0 = run SUCCESS, 1 = FAILURE/cancelled, 2 = timeout.
"""
import json
import os
import sys
import time
import urllib.request

REPO = "salim77007j/gens"


def api(url: str) -> dict:
    req = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
        "User-Agent": "gens-engine-ci",
        "Accept": "application/vnd.github+json",
    })
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def main() -> int:
    branch = sys.argv[1]
    timeout = int(sys.argv[2]) if len(sys.argv) > 2 else 1800
    deadline = time.time() + timeout
    run_id = None
    while time.time() < deadline:
        if run_id is None:
            runs = api(
                f"https://api.github.com/repos/{REPO}/actions/runs"
                f"?branch={branch}&per_page=3"
            )["workflow_runs"]
            if runs:
                run_id = runs[0]["id"]
                print(f"watching run #{runs[0]['run_number']} ({run_id}): {runs[0]['display_title']}", flush=True)
            else:
                print("no runs yet; retrying", flush=True)
        else:
            r = api(f"https://api.github.com/repos/{REPO}/actions/runs/{run_id}")
            if r["status"] == "completed":
                print(f"completed: {r['conclusion']} (run #{r['run_number']})", flush=True)
                return 0 if r["conclusion"] == "success" else 1
        time.sleep(30)
    print("timeout", flush=True)
    return 2


if __name__ == "__main__":
    sys.exit(main())

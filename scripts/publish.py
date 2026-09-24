#!/usr/bin/env python3
"""Publish every workspace crate to crates.io in dependency order.

crates.io rate-limits *new* crate names (a burst of 5, then roughly one
every 10 minutes). This script can be re-run safely: it skips versions that
are already published, and when crates.io answers 429 it sleeps until the
time given in the error message and retries.

Usage: python3 scripts/publish.py [--dry-run]
"""

import email.utils
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

DRY_RUN = "--dry-run" in sys.argv
RETRY_RE = re.compile(r"try again after (.+? GMT)")


def workspace_packages():
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"]
        )
    )
    # `publish: []` means `publish = false`; None means publishable anywhere.
    return {
        p["name"]: p
        for p in meta["packages"]
        if p.get("publish") is None or "crates-io" in p["publish"]
    }


def topo_order(packages):
    order, seen = [], set()

    def visit(name):
        if name in seen:
            return
        seen.add(name)
        for dep in packages[name]["dependencies"]:
            if dep["name"] in packages and dep["kind"] != "dev":
                visit(dep["name"])
        order.append(name)

    for name in sorted(packages):
        visit(name)
    return order


def index_path(name):
    n = name.lower()
    if len(n) <= 2:
        return f"{len(n)}/{n}"
    if len(n) == 3:
        return f"3/{n[0]}/{n}"
    return f"{n[:2]}/{n[2:4]}/{n}"


def is_published(name, version):
    url = f"https://index.crates.io/{index_path(name)}"
    req = urllib.request.Request(url, headers={"User-Agent": "concerto-publish"})
    try:
        with urllib.request.urlopen(req) as resp:
            body = resp.read().decode()
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False
        raise
    return any(json.loads(line)["vers"] == version for line in body.splitlines())


def publish(name):
    cmd = ["cargo", "publish", "-p", name]
    if DRY_RUN:
        cmd.append("--dry-run")
    while True:
        print(f"==> {' '.join(cmd)}", flush=True)
        result = subprocess.run(cmd, stderr=subprocess.PIPE, text=True)
        sys.stderr.write(result.stderr)
        if result.returncode == 0:
            return
        match = RETRY_RE.search(result.stderr)
        if not match:
            sys.exit(f"publishing {name} failed")
        retry_at = email.utils.parsedate_to_datetime(match.group(1)).timestamp()
        wait = max(0, retry_at - time.time()) + 5
        print(f"Rate limited; sleeping {wait / 60:.1f} min", flush=True)
        time.sleep(wait)


def main():
    packages = workspace_packages()
    for name in topo_order(packages):
        version = packages[name]["version"]
        if is_published(name, version):
            print(f"--- {name} {version} already published, skipping")
            continue
        publish(name)


if __name__ == "__main__":
    main()

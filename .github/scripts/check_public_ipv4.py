#!/usr/bin/env python3
"""Fail if tracked text files contain a public IPv4 literal.

Allow-list (issue #81 / roadmap #7):
  - 0.0.0.0/8 (unspecified / this-network)
  - 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 (private)
  - 127.0.0.0/8 (loopback)
  - 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 (RFC 5737 docs)
  - dotted quads whose first octet is a single digit 1-9 (client FileVersions
    such as 6.1.1.3), matching Graal_analysis's version heuristic

Scans `git ls-files` text paths. Exit 1 on any disallowed match.
"""

from __future__ import annotations

import re
import subprocess
import sys

IPV4_RE = re.compile(r"\b(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\b")


def allowed(a: int, b: int, c: int, d: int) -> bool:
    if not all(0 <= x <= 255 for x in (a, b, c, d)):
        return True  # not a real IPv4
    # Version-like quads (Graal FileVersion / build labels).
    if 1 <= a <= 9:
        return True
    if a == 0:
        return True
    if a == 10 or a == 127:
        return True
    if a == 172 and 16 <= b <= 31:
        return True
    if a == 192 and b == 168:
        return True
    # RFC 5737 documentation ranges
    if a == 192 and b == 0 and c == 2:
        return True
    if a == 198 and b == 51 and c == 100:
        return True
    if a == 203 and b == 0 and c == 113:
        return True
    return False


def main() -> int:
    listed = subprocess.check_output(
        ["git", "ls-files", "-z"], text=False
    ).split(b"\0")
    paths = [p.decode("utf-8", "surrogateescape") for p in listed if p]
    leaks: list[str] = []
    for path in paths:
        try:
            data = open(path, "rb").read()
        except OSError:
            continue
        if b"\0" in data:
            continue
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            continue
        for i, line in enumerate(text.splitlines(), 1):
            for m in IPV4_RE.finditer(line):
                octets = tuple(int(g) for g in m.groups())
                if allowed(*octets):
                    continue
                leaks.append(f"{path}:{i}:{m.group(0)}")
    if leaks:
        print("public IPv4 literal(s) outside the allow-list:", file=sys.stderr)
        for leak in leaks:
            print(leak, file=sys.stderr)
        print(
            "use loopback, private, or RFC 5737 (192.0.2.x / 198.51.100.x / "
            "203.0.113.x); see seihai-core#81",
            file=sys.stderr,
        )
        return 1
    print("ok: no public IPv4 literals outside the allow-list")
    return 0


if __name__ == "__main__":
    sys.exit(main())

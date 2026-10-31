#!/usr/bin/env python3
"""PreToolUse guard (SPEC §4.6 item 9, AGENTS.md §1): agents never build or run anything that can reach the laser.

Blocks Bash commands that build with the `hw` feature or `--all-features`, set OPENLASER_HW_PRESENT, use Luiz's
`.target/hw` lane, or run `openlaser-hw` without `--sim` (its `--help` is allowed). Exit code 2 refuses the call.
"""
import json
import re
import sys

try:
    cmd = json.load(sys.stdin).get("tool_input", {}).get("command", "") or ""
except Exception:
    sys.exit(0)

RULES = [
    (r"--features[ =]['\"]?[^ ]*\bhw\b", "builds with the `hw` feature"),
    (r"--all-features", "uses --all-features (it enables `hw`)"),
    (r"OPENLASER_HW_PRESENT\s*=", "sets OPENLASER_HW_PRESENT"),
    (r"\.target/hw\b", "uses Luiz's hardware build lane"),
]
for pattern, why in RULES:
    if re.search(pattern, cmd):
        print(f"Blocked by .claude/hooks/no_hw.py: this command {why}. Agents never build or run hardware code "
              "(AGENTS.md §1); Luiz runs hardware sessions himself (HARDWARE.md).", file=sys.stderr)
        sys.exit(2)

for m in re.finditer(r"openlaser-hw\b([^;&|\n]*)", cmd):
    args = m.group(1)
    if "--sim" not in args and "--help" not in args and not re.search(r"\b(cargo|test|build|check|clippy|grep|rg|cat|sed|ls)\b", cmd.split("openlaser-hw")[0][-40:]):
        print("Blocked by .claude/hooks/no_hw.py: `openlaser-hw` may only run with --sim (or --help) from an agent. "
              "Real hardware sessions are Luiz's (HARDWARE.md).", file=sys.stderr)
        sys.exit(2)
sys.exit(0)

"""Regenerate (or check) the OpenLaser golden-oracle fixtures from the Python prototype.

NO HARDWARE. Run with the prototype's venv python:

  ~/AI/OpenLaser/OpenMopa/.venv/bin/python tests/golden/generator/generate.py           # regenerate
  ~/AI/OpenLaser/OpenMopa/.venv/bin/python tests/golden/generator/generate.py --check   # verify, write nothing

What it does:
  1. Makes real USB unreachable in this process (usb_guard) and in every worker.
  2. Exports the two oracle commits of the prototype (OM) with `git archive`
     into a temporary directory (the OM repo and working copy are only read):
       v0 = 6aed8ff  OM main, the bytes proven on the real laser;
       v1 = HEAD     the safety-fixes branch (B1-B12), candidate until HIL-P.
  3. Runs worker.py once per tier and oracle, and once per T1 scenario per oracle
     TWICE (the two runs must be byte-identical: determinism check), each in a
     fresh process with a sanitised environment (HOME and TMPDIR point into the
     temporary directory, OPENMOPA_MOCK=1, a throwaway board-lock path).
  4. T0/T2/T3/T4 must come out identical for v0 and v1 (those modules did not
     change); they are stored once. T1 is stored per oracle, with a v0-vs-v1 diff.
  5. Writes tests/golden/{t0_numeric,t1_lmc,t2_gcode,t3_hatch,t4_import}/ and
     manifest.json. README.md, inputs/ and generator/ are left alone (inputs/
     is refreshed only with --refresh-inputs).
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import usb_guard  # noqa: E402

usb_guard.install()
SELF_TEST = usb_guard.self_test()

import argparse  # noqa: E402
import difflib  # noqa: E402
import io  # noqa: E402
import json  # noqa: E402
import platform  # noqa: E402
import shutil  # noqa: E402
import struct  # noqa: E402
import subprocess  # noqa: E402
import tarfile  # noqa: E402
import tempfile  # noqa: E402
from concurrent.futures import ThreadPoolExecutor  # noqa: E402
from importlib import metadata  # noqa: E402
from pathlib import Path  # noqa: E402

import common  # noqa: E402

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parent
WORKER = HERE / "worker.py"
TIER_DIRS = {"t0": "t0_numeric", "t1": "t1_lmc", "t2": "t2_gcode", "t3": "t3_hatch", "t4": "t4_import"}
DEFAULT_OM = Path.home() / "AI" / "OpenLaser" / "OpenMopa"
DEFAULT_DXF = Path.home() / "Documents" / "Playground" / "tests" / "DXF"
ORACLE_NOTES = {
    "v0": "OM main: the byte stream proven on the real JPT M7 / LMC board (research 05 'oracle-v0')",
    "v1": "OM fix/safety-b1-b12 (patched prototype, B1-B12); 'oracle-v1' candidate until the HIL-P hardware session passes",
}


def git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True).stdout.strip()


def export_tree(repo: Path, sha: str, dest: Path) -> None:
    raw = subprocess.run(["git", "-C", str(repo), "archive", "--format=tar", sha], check=True, capture_output=True).stdout
    dest.mkdir(parents=True)
    with tarfile.open(fileobj=io.BytesIO(raw)) as tar:
        tar.extractall(dest)  # our own repository's archive


def refresh_inputs(om_repo: Path, dxf_dir: Path, inputs: Path) -> None:
    inputs.mkdir(parents=True, exist_ok=True)
    (inputs / "dxf").mkdir(exist_ok=True)
    copies = {
        "workspace.json": om_repo / "workspace.json",
        "layer_settings.json": om_repo / "layer_settings.json",
        "openmopa-machine.ini": om_repo / "profiles" / "openmopa-machine.ini",
        "calibration.json": om_repo / "calibration.json",
        "dxf/square.dxf": dxf_dir / "square.dxf",
        "dxf/businessCard.dxf": dxf_dir / "businessCard.dxf",
    }
    record = {}
    for rel, src in copies.items():
        shutil.copyfile(src, inputs / rel)
        record[rel] = {"sha256": common.sha256_file(inputs / rel), "bytes": (inputs / rel).stat().st_size,
                       "copied_from": str(src).replace(str(Path.home()), "~")}
    common.write_json(inputs / "inputs.json", {
        "about": "Inputs the generator reads. workspace.json, layer_settings.json and calibration.json are untracked "
                 "files from the OM working copy; the .ini is tracked in OM (the generator reads each oracle's own copy); "
                 "the DXFs are Luiz's test drawings. Refresh with generate.py --refresh-inputs.",
        "files": record,
    })


def worker_env(tmp: Path) -> dict:
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": str(tmp / "home"),
        "TMPDIR": str(tmp / "tmp"),
        "LC_ALL": "C",
        "PYTHONHASHSEED": "0",
        "PYTHONUTF8": "1",
        "PYTHONIOENCODING": "utf-8",
        "OPENMOPA_MOCK": "1",
        "OPENLASER_BOARD_LOCK": str(tmp / "board.lock"),
    }


def run_worker(python: str, tree: Path, inputs: Path, out: Path, env: dict, *command: str) -> str:
    cmd = [python, "-s", "-B", str(WORKER), "--om-tree", str(tree), "--inputs", str(inputs), "--out", str(out), *command]
    proc = subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=env["TMPDIR"])
    if proc.returncode != 0:
        raise RuntimeError(f"worker {' '.join(command)} on {tree.name} failed:\n{proc.stdout}\n{proc.stderr}")
    return proc.stdout


# ---------------------------------------------------------------------------
# T1 v0-vs-v1 diff
# ---------------------------------------------------------------------------
def decode_packet(data: bytes, n_cmds: int, names: dict) -> list[str]:
    out = []
    for slot in range(n_cmds):
        words = struct.unpack_from("<6H", data, slot * 12)
        out.append(f"{names.get(words[0], hex(words[0]))}({','.join(str(w) for w in words[1:])})")
    return out


def wire_tokens(events: list[dict]) -> list[str]:
    tokens = []
    for e in events:
        if e["kind"] == "single":
            tokens.append(f"single {e['op']} {e['hex']}")
        elif e["kind"] == "read":
            tokens.append(f"read {e['status']}")
        elif e["kind"] == "list":
            tokens.append("list")
        else:
            tokens.append(e["kind"])
    return tokens


def command_stream(events: list[dict], packets: dict, names: dict) -> list[str]:
    stream = []
    for e in events:
        if e["kind"] == "list":
            stream.extend(decode_packet(packets[e["sha256"]], e["n_cmds"], names))
            stream.append("--packet boundary--")
    return stream


def hunks(a: list[str], b: list[str], limit: int = 12) -> list[dict]:
    out = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        out.append({"tag": tag, "v0_at": i1, "v1_at": j1, "v0_count": i2 - i1, "v1_count": j2 - j1,
                    "v0": a[i1:min(i2, i1 + limit)], "v1": b[j1:min(j2, j1 + limit)]})
    return out


def t1_diff(stage: dict, scenarios: list[str], packets: dict, names: dict) -> dict:
    result = {}
    for scenario in scenarios:
        e0 = common.read_json(stage["v0"] / "t1_lmc" / f"{scenario}.json")["events"]
        e1 = common.read_json(stage["v1"] / "t1_lmc" / f"{scenario}.json")["events"]
        w = hunks(wire_tokens(e0), wire_tokens(e1))
        c = hunks(command_stream(e0, packets, names), command_stream(e1, packets, names))
        list_shas0 = [e["sha256"] for e in e0 if e["kind"] == "list"]
        list_shas1 = [e["sha256"] for e in e1 if e["kind"] == "list"]
        result[scenario] = {
            "identical": e0 == e1,
            "packets_identical": list_shas0 == list_shas1,
            "v0_packets": len(list_shas0),
            "v1_packets": len(list_shas1),
            "wire_hunks": w[:20],
            "wire_hunks_total": len(w),
            "list_command_hunks": c[:20],
            "list_command_hunks_total": len(c),
        }
    return result


# ---------------------------------------------------------------------------
def build(args, out_root: Path) -> dict:
    om_repo = args.om_repo.resolve()
    inputs = GOLDEN / "inputs"
    if args.refresh_inputs or not (inputs / "inputs.json").exists():
        refresh_inputs(om_repo, args.dxf_dir, inputs)
    shas = {"v0": git(om_repo, "rev-parse", args.v0_rev), "v1": git(om_repo, "rev-parse", args.v1_rev)}
    refs = {"v0": args.v0_rev, "v1": args.v1_rev}
    try:
        branch = git(om_repo, "rev-parse", "--abbrev-ref", args.v1_rev)
    except subprocess.CalledProcessError:
        branch = args.v1_rev
    head_branch = git(om_repo, "rev-parse", "--abbrev-ref", "HEAD")
    gp_version = metadata.version("galvoplotter")
    from galvo.consts import list_command_lookup

    base_meta = {
        "schema": common.SCHEMA,
        "generator": common.GENERATOR_PATH,
        "generator_version": common.GENERATOR_VERSION,
        "galvoplotter_version": gp_version,
        "pyusb_version": metadata.version("pyusb"),
        "python_version": platform.python_version(),
    }

    def meta(tier: str, oracles: list[str], **extra) -> dict:
        return {**base_meta, "tier": tier, "oracle": "+".join(oracles),
                "om_git_sha": {o: shas[o] for o in oracles}, **extra}

    tmp = Path(tempfile.mkdtemp(prefix="openlaser-golden-"))
    for sub in ("home", "tmp"):
        (tmp / sub).mkdir()
    env = worker_env(tmp)
    trees = {}
    for oracle, sha in shas.items():
        trees[oracle] = tmp / f"om-{oracle}-{sha[:7]}"
        export_tree(om_repo, sha, trees[oracle])
    stage = {o: tmp / f"stage-{o}" for o in shas}
    stage_rerun = {o: tmp / f"stage-{o}-rerun" for o in shas}
    python = args.python

    tiers = [t for t in args.tiers.split(",") if t]
    jobs = []
    for oracle in shas:
        for tier in tiers:
            if tier != "t1":
                jobs.append((oracle, stage[oracle], (tier,)))
    scenario_lists = {}
    if "t1" in tiers:
        for oracle in shas:
            scenario_lists[oracle] = json.loads(run_worker(python, trees[oracle], inputs, stage[oracle], env, "list-t1"))
        for oracle, names in scenario_lists.items():
            for scenario in names:
                jobs.append((oracle, stage[oracle], ("t1", scenario)))
                jobs.append((oracle, stage_rerun[oracle], ("t1", scenario)))
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(run_worker, python, trees[o], inputs, out, env, *cmd) for o, out, cmd in jobs]
        for f in futures:
            f.result()

    written: dict[str, dict] = {}
    summary: dict = {"oracles": {o: {"sha": shas[o], "ref": refs[o], "note": ORACLE_NOTES[o]} for o in shas},
                     "om_head_branch": head_branch, "v1_ref_branch": branch, "tiers": {}}

    def emit_json(rel: str, body: dict) -> None:
        data = common.write_json(out_root / rel, body)
        written[rel] = {"sha256": common.sha256_bytes(data), "bytes": len(data)}

    def emit_file(rel: str, data: bytes) -> None:
        path = out_root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        written[rel] = {"sha256": common.sha256_bytes(data), "bytes": len(data)}

    # T0, T2, T3, T4: must be identical across oracles; stored once.
    for tier in [t for t in tiers if t != "t1"]:
        d = TIER_DIRS[tier]
        files0 = sorted(p.relative_to(stage["v0"]) for p in (stage["v0"] / d).rglob("*") if p.is_file())
        files1 = sorted(p.relative_to(stage["v1"]) for p in (stage["v1"] / d).rglob("*") if p.is_file())
        identical = files0 == files1 and all((stage["v0"] / f).read_bytes() == (stage["v1"] / f).read_bytes() for f in files0)
        variants = [("", ["v0", "v1"])] if identical else [("v0/", ["v0"]), ("v1/", ["v1"])]
        tier_summary = {"identical_across_oracles": identical, "files": {}}
        for prefix, oracles in variants:
            src = stage[oracles[0]]
            for f in files0 if oracles[0] == "v0" else files1:
                rel = str(f) if not prefix else f"{d}/{prefix}{f.relative_to(d)}"
                if f.suffix == ".json":
                    body = common.read_json(src / f)
                    emit_json(rel, {"meta": meta(tier.upper(), oracles, identical_across_oracles=identical), **body})
                    tier_summary["files"][rel] = _json_counts(body)
                else:
                    emit_file(rel, (src / f).read_bytes())
        summary["tiers"][tier] = tier_summary

    # T1: per oracle, determinism-checked, packets content-addressed and shared.
    if "t1" in tiers:
        packets: dict[str, bytes] = {}
        t1_summary: dict = {"scenarios": {}}
        for oracle in shas:
            for scenario in scenario_lists[oracle]:
                a = (stage[oracle] / "t1_lmc" / f"{scenario}.json").read_bytes()
                b = (stage_rerun[oracle] / "t1_lmc" / f"{scenario}.json").read_bytes()
                if a != b:
                    raise RuntimeError(f"T1 {oracle}/{scenario} is not deterministic across two runs")
            for p in (stage[oracle] / "packets").glob("*.hex"):
                packets[p.stem] = common.hex_lines_to_packet(p.read_text(encoding="ascii"))
        for digest, data in sorted(packets.items()):
            if common.sha256_bytes(data) != digest:
                raise RuntimeError(f"packet {digest} does not hash to its name")
            emit_file(f"t1_lmc/packets/{digest}.hex", common.packet_to_hex_lines(data).encode("ascii"))
        input_names = sorted({p.name for o in shas for p in (stage[o] / "t1_lmc" / "inputs").glob("*.json")})
        for name in input_names:
            bodies = {o: common.read_json(stage[o] / "t1_lmc" / "inputs" / name)
                      for o in shas if (stage[o] / "t1_lmc" / "inputs" / name).exists()}
            first = next(iter(bodies.values()))
            if any(b != first for b in bodies.values()):
                raise RuntimeError(f"T1 input {name} differs between oracles")
            emit_json(f"t1_lmc/inputs/{name}", {"meta": meta("T1", sorted(bodies)), **first})
        for oracle in shas:
            for scenario in scenario_lists[oracle]:
                body = common.read_json(stage[oracle] / "t1_lmc" / f"{scenario}.json")
                emit_json(f"t1_lmc/{oracle}/{scenario}.json", {"meta": meta("T1", [oracle], oracle_note=ORACLE_NOTES[oracle]), **body})
                t1_summary["scenarios"][f"{oracle}/{scenario}"] = {**body["counts"], "error": body["error"]}
                if scenario == "init_abort":
                    seg = body["segments"]
                    for part in ("init", "abort"):
                        lo, hi = seg[part]
                        emit_json(f"t1_lmc/{oracle}/{part}.json", {
                            "meta": meta("T1", [oracle], oracle_note=ORACLE_NOTES[oracle]),
                            "scenario": part,
                            "description": f"the {part} part of init_abort (events [{lo}, {hi}) of that stream)",
                            "derived_from": f"t1_lmc/{oracle}/init_abort.json",
                            "events": body["events"][lo:hi],
                        })
        common_scenarios = [s for s in scenario_lists["v0"] if s in scenario_lists["v1"]]
        emit_json("t1_lmc/v0_vs_v1.json", {
            "meta": meta("T1", ["v0", "v1"]),
            "about": "Machine-made diff of every T1 scenario, v0 -> v1. wire_hunks compare singles/reads with list packets "
                     "collapsed to 'list'; list_command_hunks compare the decoded list-command streams (all packets concatenated). "
                     "Hunks are truncated to 12 items per side; counts are exact.",
            "scenarios": t1_diff(stage, common_scenarios, packets, list_command_lookup),
        })
        t1_summary["distinct_packets"] = len(packets)
        summary["tiers"]["t1"] = t1_summary

    clean = usb_guard.assert_clean()
    for oracle in shas:
        for p in (stage[oracle] / "usb_isolation").glob("*.json"):
            record = common.read_json(p)
            if record.get("usb_libraries_loaded") != 0 or record.get("blocked_attempts_outside_self_test") != 0:
                raise RuntimeError(f"USB isolation failed in {p}")
    summary["usb_isolation"] = {
        "orchestrator": {"self_test_blocked": SELF_TEST, **clean},
        "workers_checked": sum(len(list((stage[o] / "usb_isolation").glob("*.json"))) +
                               len(list((stage_rerun[o] / "usb_isolation").glob("*.json"))) for o in shas),
        "worker_result": "every worker: self-test blocked 6 USB paths; 0 USB libraries loaded; 0 blocked attempts outside the self-test",
    }

    # The signed rust-v1 goldens (D13) are not generated here: carry their entries over from the committed manifest.
    committed_manifest = common.read_json(GOLDEN / "manifest.json") if (GOLDEN / "manifest.json").exists() else {}
    for k, v in committed_manifest.get("files", {}).items():
        if k.startswith(RUST_V1 + "/"):
            written[k] = v
    totals = {"files": len(written), "bytes": sum(v["bytes"] for v in written.values())}
    for tier, d in TIER_DIRS.items():
        tb = [v["bytes"] for k, v in written.items() if k.startswith(d + "/")]
        totals[d] = {"files": len(tb), "bytes": sum(tb)}
    if RUST_V1 in committed_manifest.get("totals", {}):
        totals[RUST_V1] = committed_manifest["totals"][RUST_V1]
    summary["totals"] = totals
    manifest = {
        "meta": {**base_meta, "om_git_sha": shas, "om_refs": refs},
        "inputs": common.read_json(inputs / "inputs.json")["files"],
        "totals": totals,
        "files": dict(sorted(written.items())),
    }
    data = common.write_json(out_root / "manifest.json", manifest)
    summary["manifest_sha256"] = common.sha256_bytes(data)
    if not args.keep_temp:
        shutil.rmtree(tmp, ignore_errors=True)
    else:
        summary["temp_dir"] = str(tmp)
    return summary


RUST_V1 = "rust-v1"


def _json_counts(body: dict) -> dict:
    out = {}
    if "rows" in body:
        out["rows"] = len(body["rows"])
    if "groups" in body:
        out["rows"] = sum(len(g["rows"]) for g in body["groups"])
    if "errors" in body:
        out["error_rows"] = len(body["errors"])
    if "cases" in body:
        out["cases"] = len(body["cases"])
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--om-repo", type=Path, default=DEFAULT_OM)
    ap.add_argument("--dxf-dir", type=Path, default=DEFAULT_DXF)
    ap.add_argument("--v0-rev", default="6aed8ff")
    ap.add_argument("--v1-rev", default="HEAD")
    ap.add_argument("--tiers", default="t0,t1,t2,t3,t4")
    ap.add_argument("--python", default=sys.executable, help="the OM venv python (default: this interpreter)")
    ap.add_argument("--jobs", type=int, default=max(2, (os.cpu_count() or 4)))
    ap.add_argument("--refresh-inputs", action="store_true", help="re-copy inputs/ from the OM working copy and the DXF folder")
    ap.add_argument("--check", action="store_true", help="regenerate into a temp dir and compare with the committed fixtures")
    ap.add_argument("--keep-temp", action="store_true")
    ap.add_argument("--summary", type=Path, help="also write the run summary JSON here")
    args = ap.parse_args()

    if args.check:
        out_root = Path(tempfile.mkdtemp(prefix="openlaser-golden-check-"))
        summary = build(args, out_root)
        fresh = common.read_json(out_root / "manifest.json")["files"]
        committed = common.read_json(GOLDEN / "manifest.json")["files"]
        bad = sorted(k for k in set(fresh) | set(committed) if fresh.get(k) != committed.get(k))
        shutil.rmtree(out_root, ignore_errors=True)
        if bad:
            print(f"golden check FAILED: {len(bad)} file(s) differ, e.g. {bad[:10]}")
            return 1
        print(f"golden check OK: {len(fresh)} files identical")
        return 0

    out_root = Path(tempfile.mkdtemp(prefix="openlaser-golden-out-"))
    summary = build(args, out_root)
    for d in list(TIER_DIRS.values()) + ["manifest.json"]:
        target = GOLDEN / d
        if target.is_dir():
            shutil.rmtree(target)
        elif target.exists():
            target.unlink()
        src = out_root / d
        if src.exists():
            shutil.move(str(src), str(target))
    shutil.rmtree(out_root, ignore_errors=True)
    text = json.dumps(summary, indent=1)
    if args.summary:
        args.summary.write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

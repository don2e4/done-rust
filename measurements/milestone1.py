#!/usr/bin/env python3
"""Small Linux native probes. Full cross-platform/Electron bench belongs to M4."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import selectors
import signal
import statistics
import subprocess
import sys
import tempfile
import time


def pss_bytes(text):
    for line in text.splitlines():
        fields = line.split()
        if fields and fields[0] == "Pss:":
            if len(fields) != 3 or fields[2] != "kB":
                raise ValueError("unexpected PSS units")
            return int(fields[1]) * 1024
    raise ValueError("Pss is absent; RSS is not a substitute")


def smaps_pss_bytes(text):
    # Sum exact "Pss:" fields only; "SwapPss:" lines must not be counted.
    return sum(pss_bytes(line) for line in text.splitlines()
               if line.split()[:1] == ["Pss:"])


def process_tree_pss(pid):
    pending, seen, total = [pid], set(), 0
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        root = Path("/proc") / str(current)
        try:
            rollup = root / "smaps_rollup"
            if rollup.exists():
                total += pss_bytes(rollup.read_text())
            else:
                total += smaps_pss_bytes((root / "smaps").read_text())
            tasks = list((root / "task").iterdir())
        except (FileNotFoundError, ProcessLookupError):
            if current == pid:
                raise RuntimeError("probe exited during PSS collection")
            continue
        # The kernel lists children per thread, so read every thread's list.
        for task in tasks:
            try:
                pending.extend(int(child) for child in (task / "children").read_text().split())
            except (FileNotFoundError, ProcessLookupError):
                if task.exists():
                    raise RuntimeError("kernel lacks /proc/<pid>/task/<tid>/children "
                                       "(CONFIG_PROC_CHILDREN); cannot walk the process tree")
    return total


def run_probe(binary, memory, env):
    with tempfile.TemporaryFile(mode="w+t") as stderr:
        start = time.monotonic_ns()
        proc = subprocess.Popen([str(binary.resolve())], stdout=subprocess.PIPE,
                                stderr=stderr, env=env, start_new_session=True)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(proc.stdout, selectors.EVENT_READ)
                if not selector.select(5):
                    raise RuntimeError(f"{binary}: no first-frame marker within five seconds")
                line = proc.stdout.readline().decode().strip().split()
            if len(line) != 5 or line[0] != "DONE_FRAME":
                stderr.seek(0)
                raise RuntimeError(f"{binary}: invalid/missing frame marker: {stderr.read(2000).strip()}")
            frame_ms = (int(line[1]) * 1000 - start) / 1_000_000
            if frame_ms < 0:
                raise RuntimeError("probe and runner monotonic clocks disagree")
            samples = []
            if memory:
                time.sleep(2)
                for index in range(5):
                    if proc.poll() is not None:
                        raise RuntimeError("probe exited before idle sampling")
                    samples.append(process_tree_pss(proc.pid))
                    if index < 4:
                        time.sleep(1)
            return {"first_frame_ms": frame_ms, "renderer": line[2], "scale": int(line[3]),
                    "display_type": line[4], "pss_samples_bytes": samples,
                    "idle_pss_bytes": statistics.median(samples) if samples else None}
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
            proc.stdout.close()


def summary(values):
    ordered = sorted(values)
    return {"median": statistics.median(values),
            "p95": ordered[math.ceil(len(ordered) * 0.95) - 1],
            "maximum": max(values), "count": len(values)}


def self_check():
    assert pss_bytes("Rss: 999 kB\nPss: 7 kB\nPss_Anon: 2 kB\n") == 7168
    for text in ("Rss: 7 kB\n", "Pss: 7 MB\n"):
        try:
            pss_bytes(text)
        except ValueError:
            pass
        else:
            raise AssertionError("must reject missing PSS or different units")
    assert smaps_pss_bytes("Size: 4 kB\nPss: 3 kB\nSwapPss: 2 kB\nSize: 4 kB\nPss: 1 kB\nSwapPss: 9 kB\n") == 4096
    assert summary(list(range(1, 21))) == {"median": 10.5, "p95": 19, "maximum": 20, "count": 20}
    print("Measurement parser and summary checks passed.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--done", type=Path, default=Path("target/probe/examples/hello"))
    parser.add_argument("--raw", type=Path, default=Path("target/probe/examples/raw_hello"))
    parser.add_argument("--shipping", type=Path, default=Path("target/release/examples/hello"))
    parser.add_argument("--renderer", help="explicit GSK_RENDERER; omitted uses the session default")
    parser.add_argument("--output", type=Path, default=Path("measurements/local/linux.json"))
    args = parser.parse_args()
    if args.self_check:
        self_check()
        return 0
    if platform.system() != "Linux":
        parser.error("milestone 1 probes use Linux PSS")
    for binary in (args.done, args.raw, args.shipping):
        if not binary.is_file():
            parser.error(f"build the probe first: missing {binary}")
    shipping_size = args.shipping.stat().st_size
    if shipping_size > 2_000_000:
        print(f"FAIL: shipping binary is {shipping_size:,} bytes; hard limit is 2,000,000")
        return 2
    env = dict(os.environ, DONE_PROBE="1")
    if args.renderer:
        env["GSK_RENDERER"] = args.renderer
    # Alternate the first process each pair; never compare concurrent probes.
    runs = {"done": [], "raw": []}
    for index in range(20):
        for name in (("raw", "done") if index % 2 == 0 else ("done", "raw")):
            runs[name].append(run_probe(getattr(args, name), index < 5, env))
        print(f"Pair {index + 1}/20 complete", flush=True)
        if index == 4:
            done_idle = [run["idle_pss_bytes"] for run in runs["done"]]
            raw_idle = [run["idle_pss_bytes"] for run in runs["raw"]]
            overhead = statistics.median([done - raw for done, raw in zip(done_idle, raw_idle)])
            if overhead > 5_000_000 or (statistics.median(done_idle) > 30_000_000 and statistics.median(raw_idle) <= 30_000_000):
                break  # Stop collecting further runs once a memory gate fails.
    idle = {name: [run["idle_pss_bytes"] for run in values[:5]] for name, values in runs.items()}
    overhead = [done - raw for done, raw in zip(idle["done"], idle["raw"])]
    done_memory = statistics.median(idle["done"])
    raw_memory = statistics.median(idle["raw"])
    delta = statistics.median(overhead)
    toolkit_exception = raw_memory > 30_000_000
    renderers = {run["renderer"] for values in runs.values() for run in values}
    scales = {run["scale"] for values in runs.values() for run in values}
    displays = {run["display_type"] for values in runs.values() for run in values}
    renderer_matches = len(renderers) == 1 and "unknown" not in renderers and len(scales) == 1 and len(displays) == 1
    passed = renderer_matches and shipping_size <= 2_000_000 and delta <= 5_000_000 and (done_memory <= 30_000_000 or toolkit_exception)
    source_paths = [Path("Cargo.toml"), Path("Cargo.lock"), *sorted(Path("crates").rglob("*.rs")), *sorted(Path("crates").rglob("Cargo.toml"))]
    cpu = next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), platform.machine())
    memory_kib = next(int(line.split()[1]) for line in Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemTotal:"))
    result = {
        "environment": {
            "platform": platform.platform(), "cpu": cpu, "ram_bytes": memory_kib * 1024,
            "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            "cargo": subprocess.check_output(["cargo", "--version"], text=True).strip(),
            "gtk4": subprocess.check_output(["pkg-config", "--modversion", "gtk4"], text=True).strip(),
            "session": {key: env.get(key) for key in ("DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "GSK_RENDERER")},
            "actual_renderers": sorted(renderers), "command": sys.argv,
            "scale_factors": sorted(scales), "display_types": sorted(displays),
        },
        "source_sha256": {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in source_paths},
        "dependency_lockfile": Path("Cargo.lock").read_text(),
        "binaries": {name: {"path": str(path.resolve()), "bytes": path.stat().st_size,
                            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                     for name, path in (("shipping", args.shipping), ("done_probe", args.done), ("raw_probe", args.raw))},
        "runs": runs, "paired_overhead_bytes": overhead,
        "idle_pss_bytes": {name: summary(values) for name, values in idle.items()},
        "idle_sample_maxima_bytes": {name: max(sample for run in values[:5] for sample in run["pss_samples_bytes"]) for name, values in runs.items()},
        "warm_process_startup_ms": {name: summary([run["first_frame_ms"] for run in values]) for name, values in runs.items()},
        "frame_marker": "GTK after-paint; validate against rendering; compositor presentation is not measured",
        "cold_start": "unverified: no cache reset; these are fresh processes with warm OS caches",
        "toolkit_baseline_memory_exception": toolkit_exception,
        "renderer_matches": renderer_matches,
        "verified_size_and_memory_gates_pass": passed,
        "milestone_complete": False,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Shipping binary: {shipping_size:,} bytes")
    print(f"Idle PSS: done {done_memory / 1e6:.3f} MB; raw {raw_memory / 1e6:.3f} MB; paired overhead {delta / 1e6:.3f} MB")
    print(f"Size/memory gates: {'PASS' if passed else 'FAIL'}; cold startup remains UNVERIFIED")
    print(f"Raw results: {args.output}")
    return 0 if passed else 2


if __name__ == "__main__":
    raise SystemExit(main())

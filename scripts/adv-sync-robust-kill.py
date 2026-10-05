#!/usr/bin/env python3
"""SIGKILL the sync client (target/debug/examples/sync_dir) at random moments.

Two "devices" (vault folders A and B, each with its own state folder) sync
through a real cairn-server process. Between runs the script makes random
edits, creates, deletes and renames on both vaults; each sync_dir run is
killed with SIGKILL after a random delay (or allowed to finish). Optionally
the server is killed with SIGKILL and restarted too (--kill-server).

At the end both devices sync cleanly a few times and the script checks:
  * the two vaults are identical (excluding .trash)
  * every line ever written is in A, B, a .trash folder or the archive of
    user deletions
  * no leftover temp files, valid state.json files
  * number of conflict copies and duplicated contents (reported)

Usage: scripts/adv-sync-robust-kill.py [--iterations 80] [--seed 1] [--kill-server] [--keep]
KILL_MAX=0.1 (seconds) is the largest delay before a SIGKILL.
Exit status 1 if a check fails. Everything lives in a mktemp folder under /tmp.
"""
import argparse
import json
import os
import random
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER = os.path.join(ROOT, "target/debug/cairn-server")
SYNC = os.path.join(ROOT, "target/debug/examples/sync_dir")
KILL_MAX = float(os.environ.get("KILL_MAX", "0.1"))
TOKEN = "kill-test-token-0123456789"
PASS = "correct horse battery staple"


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Server:
    def __init__(self, data, port):
        self.data, self.port, self.proc = data, port, None

    def start(self):
        env = dict(os.environ, CAIRN_TOKENS=TOKEN, CAIRN_DATA=self.data, CAIRN_ADDR=f"127.0.0.1:{self.port}", RUST_LOG="warn")
        self.log = open(os.path.join(self.data, "..", "server.log"), "a")
        self.proc = subprocess.Popen([SERVER], env=env, stdout=self.log, stderr=self.log)
        for _ in range(500):
            try:
                socket.create_connection(("127.0.0.1", self.port), 0.2).close()
                return
            except OSError:
                time.sleep(0.02)
        raise RuntimeError("server did not start")

    def kill(self):
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(signal.SIGKILL)
            self.proc.wait()


def walk(root, skip_trash=True):
    out = {}
    for d, dirs, files in os.walk(root):
        rel_d = os.path.relpath(d, root)
        dirs[:] = [x for x in dirs if not x.startswith(".")] if skip_trash else [x for x in dirs if x not in (".cairn",)]
        for f in files:
            if f.startswith("."):
                continue
            p = os.path.join(d, f)
            rel = os.path.normpath(os.path.join(rel_d, f))
            with open(p, "rb") as fh:
                out[rel] = fh.read().decode("utf-8", "replace")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--iterations", type=int, default=80)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--kill-server", action="store_true", help="also SIGKILL and restart the server at random")
    ap.add_argument("--keep", action="store_true", help="keep the temp folder")
    args = ap.parse_args()
    for b in (SERVER, SYNC):
        if not os.path.exists(b):
            sys.exit(f"missing {b}")
    rnd = random.Random(args.seed)
    tmp = tempfile.mkdtemp(prefix="adv-sync-robust-kill.", dir="/tmp")
    os.makedirs(os.path.join(tmp, "data"))
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    srv = Server(os.path.join(tmp, "data"), port)
    srv.start()
    devs = {
        "A": (os.path.join(tmp, "a"), os.path.join(tmp, "sa"), "laptop"),
        "B": (os.path.join(tmp, "b"), os.path.join(tmp, "sb"), "phone"),
    }
    for v, _, _ in devs.values():
        os.makedirs(v)
    archive = os.path.join(tmp, "archive")
    os.makedirs(archive)
    written = []
    stats = {"killed": 0, "finished": 0, "failed": 0, "server_kills": 0}

    def run_sync(name, kill_after=None):
        v, s, dev = devs[name]
        p = subprocess.Popen([SYNC, v, s, url, TOKEN, "notes", dev, PASS], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if kill_after is not None:
            time.sleep(kill_after)
            if p.poll() is None:
                p.send_signal(signal.SIGKILL)
                p.wait()
                stats["killed"] += 1
                return None
        out, err = p.communicate()
        if p.returncode == 0:
            stats["finished"] += 1
            return json.loads(out.decode() or "{}")
        stats["failed"] += 1
        return err.decode().strip()

    # seed A with notes, initial syncs complete
    a = devs["A"][0]
    for i in range(150):
        d = os.path.join(a, f"f{i % 5}")
        os.makedirs(d, exist_ok=True)
        line = f"seed {i}\n"
        with open(os.path.join(d, f"n{i}.md"), "w") as fh:
            fh.write(line + ("filler text " * 40) + "\n")
        written.append(line)
    print("initial A:", run_sync("A"))
    print("initial B:", run_sync("B"))

    def mutate(name, step):
        v = devs[name][0]
        files = sorted(walk(v).keys())
        for _ in range(rnd.randint(1, 25)):
            op = rnd.random()
            if op < 0.45 and files:
                p = os.path.join(v, rnd.choice(files))
                if os.path.exists(p):
                    line = f"edit {step} {name} {rnd.random():.6f}\n"
                    with open(p, "a") as fh:
                        fh.write(line)
                    written.append(line)
            elif op < 0.65:
                rel = f"f{rnd.randint(0, 6)}/c{rnd.randint(0, 400)}.md"
                p = os.path.join(v, rel)
                if not os.path.exists(p):
                    os.makedirs(os.path.dirname(p), exist_ok=True)
                    line = f"create {step} {name} {rnd.random():.6f}\n"
                    with open(p, "w") as fh:
                        fh.write(line)
                    written.append(line)
            elif op < 0.75 and files:
                rel = rnd.choice(files)
                p = os.path.join(v, rel)
                if os.path.exists(p):
                    shutil.copy(p, os.path.join(archive, f"{step}-{name}-{len(os.listdir(archive))}.txt"))
                    os.remove(p)
            elif op < 0.9 and files:
                rel = rnd.choice(files)
                p = os.path.join(v, rel)
                to = os.path.join(v, f"r{rnd.randint(0, 3)}/m{rnd.randint(0, 300)}.md")
                if os.path.exists(p) and not os.path.exists(to):
                    os.makedirs(os.path.dirname(to), exist_ok=True)
                    os.rename(p, to)
            else:
                # binary attachment
                rel = f"att/a{rnd.randint(0, 5)}.bin"
                p = os.path.join(v, rel)
                os.makedirs(os.path.dirname(p), exist_ok=True)
                line = f"bin {step} {name} {rnd.random():.6f}\n"
                with open(p, "a") as fh:
                    fh.write(line)
                written.append(line)
            files = sorted(walk(v).keys())

    t0 = time.time()
    for step in range(args.iterations):
        name = rnd.choice(["A", "B"])
        mutate(name, step)
        if args.kill_server and rnd.random() < 0.1:
            # kill the server while the client syncs
            v, s, dev = devs[name]
            p = subprocess.Popen([SYNC, v, s, url, TOKEN, "notes", dev, PASS], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            time.sleep(rnd.uniform(0.0, 0.3))
            srv.kill()
            stats["server_kills"] += 1
            p.communicate()
            srv.start()
            continue
        if rnd.random() < 0.75:
            run_sync(name, kill_after=rnd.uniform(0.002, KILL_MAX))
        else:
            run_sync(name)
    # settle
    for _ in range(3):
        for name in ("A", "B"):
            r = run_sync(name)
            if not isinstance(r, dict):
                print(f"final sync {name} failed: {r}")
    fa, fb = walk(devs["A"][0]), walk(devs["B"][0])
    problems = []
    if fa != fb:
        only_a = sorted(set(fa.items()) - set(fb.items()))
        only_b = sorted(set(fb.items()) - set(fa.items()))
        problems.append(f"vaults differ: only on A {[x[0] for x in only_a][:10]}, only on B {[x[0] for x in only_b][:10]} ({len(only_a)}/{len(only_b)})")
    every = ""
    for v, _, _ in devs.values():
        every += "".join(walk(v).values())
        every += "".join(walk(os.path.join(v, ".trash")).values()) if os.path.isdir(os.path.join(v, ".trash")) else ""
    every += "".join(open(os.path.join(archive, f), errors="replace").read() for f in os.listdir(archive))
    lost = [l for l in written if l not in every]
    if lost:
        problems.append(f"{len(lost)} lines lost, e.g. {lost[:5]}")
    for name, (v, s, _) in devs.items():
        try:
            json.load(open(os.path.join(s, "state.json")))
        except Exception as e:
            problems.append(f"{name} state.json unreadable: {e}")
        leftovers = [os.path.join(d, f) for d, _, fs in os.walk(v) for f in fs if ".cairn-tmp-" in f]
        if leftovers:
            problems.append(f"{name}: {len(leftovers)} leftover temp files, e.g. {leftovers[:3]}")
        if os.path.exists(os.path.join(s, "state.tmp")):
            problems.append(f"{name}: leftover state.tmp")
    conflicts = [p for p in fa if "(conflict " in p]
    by_content = {}
    for p, c in fa.items():
        by_content.setdefault(c, []).append(p)
    dupes = {c[:40]: ps for c, ps in by_content.items() if len(ps) > 1}
    print(json.dumps({"seed": args.seed, "iterations": args.iterations, **stats, "files": len(fa), "conflict_copies": len(conflicts),
                      "identical_content_groups": len(dupes), "lines_written": len(written), "seconds": round(time.time() - t0, 1)}))
    if conflicts:
        print("conflict copies:", conflicts[:10])
    if dupes:
        print("identical contents under several names:", list(dupes.values())[:10])
    srv.kill()
    if args.keep:
        print("kept", tmp)
    else:
        shutil.rmtree(tmp, ignore_errors=True)
    if problems:
        print("FAIL:\n  " + "\n  ".join(problems))
        sys.exit(1)
    print("OK")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Time a full-vault sync from scratch with the release server and the
release sync_dir example (performance tests, see e2e/adv_perf.test.mjs).

  python3 scripts/adv-perf-sync.py <source vault> <work dir> [port]

Device A uploads a copy of <source vault>, device B downloads into an empty
folder, then both run a sync with no changes. For every step it prints wall
time and the peak RSS of that sync_dir process (wait4 rusage), and the
server's peak RSS so far (VmHWM). Prints one JSON object at the end.
Build first:
  cargo build --release -p cairn-server
  cargo build --release -p cairn-sync --example sync_dir
"""
import json, os, secrets, shutil, signal, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER = os.path.join(ROOT, "target/release/cairn-server")
SYNC_DIR = os.path.join(ROOT, "target/release/examples/sync_dir")

src, work = sys.argv[1], sys.argv[2]
port = int(sys.argv[3]) if len(sys.argv) > 3 else 18787
shutil.rmtree(work, ignore_errors=True)
os.makedirs(work)
token = "perf-" + secrets.token_hex(16)  # throwaway test token for this local server
url = f"http://127.0.0.1:{port}"


def status(pid, key):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith(key + ":"):
                    return int(line.split()[1])  # kB
    except OSError:
        return None


def cpu_s(pid):
    try:
        f = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
        return round((int(f[11]) + int(f[12])) / os.sysconf("SC_CLK_TCK"), 2)
    except OSError:
        return None


def count_files(d):
    n = 0
    for root, dirs, files in os.walk(d):
        dirs[:] = [x for x in dirs if x != ".cairn"]
        n += sum(1 for f in files if f.endswith(".md"))
    return n


# a copy so the generated vault stays untouched
vault_a = os.path.join(work, "a")
shutil.copytree(src, vault_a, ignore=shutil.ignore_patterns(".cairn"))
vault_b = os.path.join(work, "b")
os.makedirs(vault_b)
data = os.path.join(work, "server-data")
os.makedirs(data)

env = dict(os.environ, CAIRN_TOKENS=token, CAIRN_DATA=data, CAIRN_ADDR=f"127.0.0.1:{port}", RUST_LOG="warn")
server = subprocess.Popen([SERVER], env=env, stdout=open(os.path.join(work, "server.log"), "w"), stderr=subprocess.STDOUT)
for _ in range(100):
    try:
        urllib.request.urlopen(url + "/health", timeout=0.5)
        break
    except Exception:
        time.sleep(0.1)

steps = []


def run_rusage(name, vault, state):
    args = [SYNC_DIR, vault, state, url, token, "perf-vault", name.split()[0], "perf passphrase"]
    outp = os.path.join(work, name.replace(" ", "_") + ".out")
    errp = os.path.join(work, name.replace(" ", "_") + ".err")
    t = time.perf_counter()
    pid = os.fork()
    if pid == 0:
        fo = os.open(outp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC)
        fe = os.open(errp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC)
        os.dup2(fo, 1)
        os.dup2(fe, 2)
        os.execv(args[0], args)
    _, st, ru = os.wait4(pid, 0)
    wall = time.perf_counter() - t
    out = open(outp).read()
    err = open(errp).read()
    report = None
    try:
        report = json.loads(out.strip().splitlines()[-1])
    except Exception:
        pass
    step = {
        "step": name,
        "exit": os.waitstatus_to_exitcode(st),
        "wall_s": round(wall, 3),
        "user_s": round(ru.ru_utime, 3),
        "sys_s": round(ru.ru_stime, 3),
        "peak_rss_mb": round(ru.ru_maxrss / 1024, 1),
        "server_peak_rss_mb": round((status(server.pid, "VmHWM") or 0) / 1024, 1),
        "server_rss_mb": round((status(server.pid, "VmRSS") or 0) / 1024, 1),
        "server_cpu_s_total": cpu_s(server.pid),
        "report": {k: (len(v) if isinstance(v, list) else v) for k, v in (report or {}).items()} if isinstance(report, dict) else report,
        "stderr_tail": err[-300:],
    }
    steps.append(step)
    print(json.dumps(step), flush=True)


try:
    run_rusage("A upload", vault_a, os.path.join(work, "state-a"))
    run_rusage("B download", vault_b, os.path.join(work, "state-b"))
    run_rusage("A nochange", vault_a, os.path.join(work, "state-a"))
    run_rusage("B nochange", vault_b, os.path.join(work, "state-b"))
finally:
    server.send_signal(signal.SIGTERM)
    try:
        server.wait(5)
    except subprocess.TimeoutExpired:
        server.kill()

db = sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(data) for f in fs)
result = {
    "source": src,
    "notes_a": count_files(vault_a),
    "notes_b": count_files(vault_b),
    "server_data_mb": round(db / 1e6, 1),
    "steps": steps,
}
print(json.dumps(result))

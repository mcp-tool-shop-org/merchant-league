#!/usr/bin/env bash
# Offline build and smoke gate for the held Club Mode (`league-tui`) container image.
# Runs in CI (`--network=none`, no login, no registry push) and optionally from `scripts/verify.sh`.
set -euo pipefail
cd "$(dirname "$0")/.."

IMAGE_TAG="${MERCHANT_LEAGUE_SMOKE_IMAGE:-merchant-league:ci-smoke}"
SAVES_CTR="ml-saves-smoke-$$"
START_EPOCH=$(date +%s)

cleanup() {
  docker rm -f "$SAVES_CTR" >/dev/null 2>&1 || true
  docker rmi -f "$IMAGE_TAG" >/dev/null 2>&1 || true
}
trap cleanup EXIT

step() { printf '\n[smoke-container] == %s\n' "$1"; }

step "1. Verify .dockerignore allowlist excludes repo extras and secrets"
python3 - <<'PY'
import os

root = "."
expected_top = {"Cargo.toml", "Cargo.lock", "LICENSE", "crates", "vendor"}
excluded_roots = [
    ".git",
    "target",
    "site",
    "saves",
    ".league",
    "docs",
    "atlas",
    "scripts",
    ".github",
    "README.md",
    "CHANGELOG.md",
    "SECURITY.md",
    "SHIP_GATE.md",
    "rust-toolchain.toml",
]
groups = {}
total_files = 0
total_bytes = 0
for top in sorted(expected_top):
    if not os.path.exists(top):
        raise SystemExit(f"missing expected context entry: {top}")
    if os.path.isfile(top):
        sz = os.path.getsize(top)
        groups[top] = (1, sz)
        total_files += 1
        total_bytes += sz
    else:
        for dp, dns, fns in os.walk(top):
            dns[:] = [d for d in dns if d not in (".git", "target", "saves", ".league")]
            for fn in fns:
                if (
                    fn.startswith(".env")
                    or fn == ".gitignore"
                    or fn.endswith((".save.json", ".seal", ".replay.json", ".seed", ".log"))
                ):
                    continue
                fp = os.path.join(dp, fn)
                rel = os.path.relpath(fp, root)
                parts = rel.split(os.sep)
                grp = "/".join(parts[:2]) if len(parts) >= 2 else parts[0]
                sz = os.path.getsize(fp)
                c, b = groups.get(grp, (0, 0))
                groups[grp] = (c + 1, b + sz)
                total_files += 1
                total_bytes += sz

for grp in sorted(groups):
    c, b = groups[grp]
    print(f"context {grp:28s}: {c:4d} files, {b:10d} bytes")
print(f"context total allowed        : {total_files} files, {total_bytes} bytes")
print(f"excluded top-level paths     : {', '.join(excluded_roots)}")
PY

step "2. Build image locally (${IMAGE_TAG}; no push)"
docker build --progress=plain -t "$IMAGE_TAG" .

step "3. Inspect image config & size"
IMAGE_SIZE=$(docker image inspect "$IMAGE_TAG" --format '{{.Size}}')
IMAGE_USER=$(docker image inspect "$IMAGE_TAG" --format '{{.Config.User}}')
IMAGE_WORKDIR=$(docker image inspect "$IMAGE_TAG" --format '{{.Config.WorkingDir}}')
IMAGE_ENTRYPOINT=$(docker image inspect "$IMAGE_TAG" --format '{{json .Config.Entrypoint}}')
echo "image.Size=${IMAGE_SIZE}"
echo "image.Config.User=${IMAGE_USER}"
echo "image.Config.WorkingDir=${IMAGE_WORKDIR}"
echo "image.Config.Entrypoint=${IMAGE_ENTRYPOINT}"
[ "$IMAGE_USER" = "65532:65532" ]
[ "$IMAGE_WORKDIR" = "/saves" ]
[ "$IMAGE_ENTRYPOINT" = '["/usr/local/bin/league-tui"]' ]

step "4. Run offline CLI & non-TTY smoke checks (--network=none)"
run_check() {
  local expected="$1"
  shift
  local out_file
  out_file=$(mktemp)
  set +e
  "$@" </dev/null >"$out_file" 2>&1
  local rc=$?
  set -e
  echo "CMD: $*  => exit=${rc} (expected ${expected})"
  sed 's/^/  | /' "$out_file"
  LAST_OUTPUT=$(cat "$out_file")
  rm -f "$out_file"
  if [ "$expected" = "nonzero" ]; then
    [ "$rc" -ne 0 ] || { echo "FAIL: expected non-zero exit code"; exit 1; }
  else
    [ "$rc" -eq "$expected" ] || { echo "FAIL: expected exit ${expected}, got ${rc}"; exit 1; }
  fi
}

run_check 0 docker run --rm --network=none "$IMAGE_TAG" --help
printf '%s' "$LAST_OUTPUT" | grep -q "Usage: league-tui"

run_check 0 docker run --rm --network=none "$IMAGE_TAG" --version
printf '%s' "$LAST_OUTPUT" | grep -q "^league-tui 0\.0\.0$"

run_check 1 docker run --rm --network=none "$IMAGE_TAG"
printf '%s' "$LAST_OUTPUT" | grep -q "TTY_REQUIRED"
printf '%s' "$LAST_OUTPUT" | grep -q "docker run --rm -it"

run_check nonzero docker run --rm --network=none --entrypoint /bin/sh "$IMAGE_TAG" -c true
run_check nonzero docker run --rm --network=none --entrypoint /bin/bash "$IMAGE_TAG" -c true

step "5. Prove /saves inside the image is writable by UID 65532 (offline PTY run, no -v mount)"
IMAGE_TAG="$IMAGE_TAG" SAVES_CTR="$SAVES_CTR" python3 - <<'PY'
import fcntl
import io
import json
import os
import pty
import re
import select
import struct
import subprocess
import tarfile
import termios
import time

image_tag = os.environ["IMAGE_TAG"]
ctr = os.environ["SAVES_CTR"]
cmd = [
    "docker",
    "run",
    "--name",
    ctr,
    "-it",
    "--network=none",
    image_tag,
    "--log-file",
    "/saves/smoke.log",
]
print("CMD:", " ".join(cmd))

master_fd, slave_fd = pty.openpty()
attrs = termios.tcgetattr(slave_fd)
attrs[3] &= ~termios.ECHO
termios.tcsetattr(slave_fd, termios.TCSANOW, attrs)
fcntl.ioctl(slave_fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))

proc = subprocess.Popen(
    cmd,
    stdin=slave_fd,
    stdout=slave_fd,
    stderr=slave_fd,
    start_new_session=True,
    close_fds=True,
)
os.close(slave_fd)

ansi_re = re.compile(r"\x1b\[[0-9;:?]*[A-Za-z]")
raw_pending = b""
buf = ""
phase = "wait_lobby"
lobby_cut = 0
curtain_seen_at = 0.0
last_q_sent = 0.0
deadline = time.monotonic() + 30.0

try:
    while time.monotonic() < deadline:
        rlist, _, _ = select.select([master_fd], [], [], 0.15)
        if master_fd in rlist:
            try:
                chunk = os.read(master_fd, 65536)
            except OSError:
                chunk = b""
            if chunk:
                raw_pending += chunk
                while b"\x1b[6n" in raw_pending:
                    _, _, raw_pending = raw_pending.partition(b"\x1b[6n")
                    os.write(master_fd, b"\x1b[1;1R")
                if len(raw_pending) > 8:
                    raw_pending = raw_pending[-8:]
                buf += ansi_re.sub("", chunk.decode("utf-8", errors="replace"))

        if phase == "wait_lobby" and "Seat Count:" in buf:
            lobby_cut = len(buf)
            os.write(master_fd, b"\r")
            phase = "wait_curtain"
        elif phase == "wait_curtain" and "Privacy Curtain" in buf[lobby_cut:]:
            curtain_seen_at = time.monotonic()
            phase = "dead_time_wait"
        elif phase == "dead_time_wait" and (time.monotonic() - curtain_seen_at) >= 0.7:
            os.write(master_fd, b"q")
            last_q_sent = time.monotonic()
            phase = "quitting"
        elif phase == "quitting" and proc.poll() is None and (time.monotonic() - last_q_sent) >= 0.4:
            try:
                os.write(master_fd, b"q")
            except OSError:
                pass
            last_q_sent = time.monotonic()

        if proc.poll() is not None:
            break

    rc = proc.wait(timeout=5)
finally:
    try:
        os.close(master_fd)
    except OSError:
        pass

print(f"PTY container run => exit={rc} (expected 0), phase={phase}")
if rc != 0 or phase != "quitting":
    print(f"PTY tail={buf[-1000:]!r}")
if rc != 0:
    raise SystemExit(f"PTY run failed (rc={rc}, phase={phase})")
if phase != "quitting":
    raise SystemExit(f"PTY run did not reach Privacy Curtain cleanly (phase={phase})")

diff_out = subprocess.check_output(["docker", "diff", ctr], text=True)
print("docker diff output:")
diff_lines = [ln.strip() for ln in diff_out.splitlines() if ln.strip()]
for ln in diff_lines:
    print(f"  {ln}")
for ln in diff_lines:
    path_part = ln.split(" ", 1)[1]
    if path_part != "/saves" and not path_part.startswith("/saves/"):
        raise SystemExit(f"unexpected filesystem change outside /saves: {ln}")
for req in ("A /saves/club-1.save.json", "A /saves/club-1.seal", "A /saves/smoke.log"):
    if req not in diff_lines:
        raise SystemExit(f"missing expected /saves write in docker diff: {req}")

cp_bytes = subprocess.check_output(["docker", "cp", f"{ctr}:/saves", "-"])
seen = {}
with tarfile.open(fileobj=io.BytesIO(cp_bytes), mode="r|*") as tf:
    for m in tf:
        norm_name = m.name.lstrip("./").rstrip("/")
        if not norm_name:
            continue
        print(f"docker cp member: {norm_name:30s} uid={m.uid} gid={m.gid} mode={oct(m.mode)} size={m.size}")
        raw = tf.extractfile(m).read() if m.isfile() else b""
        seen[norm_name] = (m.uid, m.gid, m.mode, m.size, raw)

for req_name in ("saves", "saves/club-1.save.json", "saves/club-1.seal", "saves/smoke.log"):
    if req_name not in seen:
        raise SystemExit(f"missing {req_name} in docker cp output")
    uid, gid, _, size, raw = seen[req_name]
    if (uid, gid) != (65532, 65532):
        raise SystemExit(f"{req_name} owned by {uid}:{gid}, expected 65532:65532")
    if req_name != "saves" and size == 0:
        raise SystemExit(f"{req_name} unexpectedly empty")
    if req_name == "saves/club-1.save.json":
        parsed = json.loads(raw.decode("utf-8"))
        if parsed.get("table_id") != 1:
            raise SystemExit(f"unexpected save table_id: {parsed.get('table_id')}")

subprocess.run(["docker", "rm", "-f", ctr], check=True)
PY

step "6. Inspect complete flattened image file list and assert runtime contents"
IMAGE_TAG="$IMAGE_TAG" IMAGE_SIZE="$IMAGE_SIZE" python3 - <<'PY'
import io
import json
import os
import posixpath
import subprocess
import tarfile

image_tag = os.environ["IMAGE_TAG"]
image_size = os.environ["IMAGE_SIZE"]

proc = subprocess.Popen(["docker", "image", "save", image_tag], stdout=subprocess.PIPE)
save_bytes, _ = proc.communicate()
if proc.returncode != 0:
    raise SystemExit(f"docker image save failed: {proc.returncode}")

with tarfile.open(fileobj=io.BytesIO(save_bytes), mode="r:*") as tf:
    members = {m.name.lstrip("./"): m for m in tf.getmembers()}

    def read_archive_entry(rel_path: str) -> bytes:
        cur = rel_path.lstrip("./")
        for _ in range(8):
            m = members[cur]
            if m.issym():
                parent = posixpath.dirname(cur)
                cur = posixpath.normpath(posixpath.join(parent, m.linkname)).lstrip("./")
                continue
            if m.islnk():
                cur = m.linkname.lstrip("./")
                continue
            f = tf.extractfile(m)
            if f is None:
                raise SystemExit(f"cannot extract archive entry {rel_path} -> {cur}")
            return f.read()
        raise SystemExit(f"symlink loop in docker save archive: {rel_path}")

    manifest = json.loads(read_archive_entry("manifest.json").decode("utf-8"))[0]
    config = json.loads(read_archive_entry(manifest["Config"]).decode("utf-8"))
    layers = [l.lstrip("./") for l in manifest["Layers"]]
    non_empty_history = [h for h in config.get("history", []) if not h.get("empty_layer", False)]

    fs = {}
    stage2_added = []
    for idx, layer_path in enumerate(layers):
        cmd_note = (
            non_empty_history[idx].get("created_by", "")
            if idx < len(non_empty_history)
            else ""
        )
        is_stage2_layer = (
            any(tok in cmd_note for tok in ("league-tui", "merchant-league/LICENSE", "/saves"))
            if non_empty_history
            else idx >= max(0, len(layers) - 4)
        )
        with tarfile.open(fileobj=io.BytesIO(read_archive_entry(layer_path)), mode="r:*") as ltf:
            for m in ltf:
                rel = m.name.lstrip("./").rstrip("/")
                if not rel:
                    continue
                parent, _, base = rel.rpartition("/")
                if base == ".wh..wh..opq":
                    prefix = f"{parent}/" if parent else ""
                    for k in [k for k in fs if k.startswith(prefix)]:
                        del fs[k]
                    continue
                if base.startswith(".wh."):
                    victim = f"{parent}/{base[4:]}" if parent else base[4:]
                    for k in [k for k in fs if k == victim or k.startswith(victim + "/")]:
                        del fs[k]
                    continue
                mtype = "dir" if m.isdir() else ("symlink" if m.issym() else ("hardlink" if m.islnk() else "file"))
                fs[rel] = {
                    "type": mtype,
                    "mode": m.mode & 0o7777,
                    "uid": m.uid,
                    "gid": m.gid,
                    "size": m.size,
                    "layer": idx,
                }
                if is_stage2_layer:
                    stage2_added.append((idx, rel, mtype, m.uid, m.gid, m.mode & 0o7777, m.size, cmd_note))

print(f"image total size bytes       : {image_size}")
print(f"total layers                 : {len(layers)}")
print(f"total flattened entries      : {len(fs)}")
print("stage-2 added entries:")
for idx, rel, mtype, uid, gid, mode, size, _ in stage2_added:
    print(f"  L{idx}: {uid}:{gid} {oct(mode)} {mtype:7s} {size:10d} /{rel}")

stage2_non_dirs = {rel for (_, rel, mtype, *_) in stage2_added if mtype != "dir"}
expected_stage2_files = {"usr/local/bin/league-tui", "usr/share/doc/merchant-league/LICENSE"}
if stage2_non_dirs != expected_stage2_files:
    raise SystemExit(f"unexpected Stage-2 non-dir entries: {sorted(stage2_non_dirs)} != {sorted(expected_stage2_files)}")

print("complete sorted image file list:")
for rel in sorted(fs):
    info = fs[rel]
    print(f"  {info['uid']:5d}:{info['gid']:<5d} {oct(info['mode']):>6s} {info['type']:7s} {info['size']:10d} /{rel}")

binary = fs.get("usr/local/bin/league-tui")
if not binary or binary["type"] != "file" or binary["size"] <= 0 or not (binary["mode"] & 0o100):
    raise SystemExit(f"missing or non-executable /usr/local/bin/league-tui: {binary}")

lic = fs.get("usr/share/doc/merchant-league/LICENSE")
expected_lic_size = os.path.getsize("LICENSE")
if not lic or lic["type"] != "file" or lic["size"] != expected_lic_size:
    raise SystemExit(f"missing or mismatched LICENSE: {lic} (expected {expected_lic_size})")

saves_dir = fs.get("saves")
if not saves_dir or saves_dir["type"] != "dir" or (saves_dir["uid"], saves_dir["gid"]) != (65532, 65532):
    raise SystemExit(f"/saves dir missing or wrong owner: {saves_dir}")
if any(p.startswith("saves/") for p in fs):
    raise SystemExit("/saves must be empty in the built image")

forbidden_shells = {"sh", "bash", "dash", "ash", "zsh", "ksh", "fish", "csh", "tcsh"}
forbidden_rust_tools = {"cargo", "rustc", "rustdoc", "rustup", "rust-lld"}
forbidden_cc_bins = {"gcc", "cc", "g++", "c++", "clang", "ld", "as", "ar", "make"}
forbidden_repo_names = {
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "Dockerfile", ".dockerignore",
    "SHIP_GATE.md", "SECURITY.md", "README.md", "CHANGELOG.md", "CLAUDE.md",
}
forbidden_top_dirs = {".git", ".league", "crates", "vendor", "target", "work"}

for rel in fs:
    parts = rel.split("/")
    base = parts[-1]
    in_bin = "bin" in parts or "sbin" in parts
    if in_bin and base in forbidden_shells:
        raise SystemExit(f"forbidden shell binary in image: /{rel}")
    if base in forbidden_rust_tools or (in_bin and base in forbidden_cc_bins):
        raise SystemExit(f"forbidden compiler/toolchain binary in image: /{rel}")
    if base in forbidden_repo_names or base.startswith(".env"):
        raise SystemExit(f"forbidden repo/env file in image: /{rel}")
    if parts[0] in forbidden_top_dirs or any(s in {".git", ".league"} for s in parts):
        raise SystemExit(f"forbidden source/vcs/secret directory in image: /{rel}")
    if rel.endswith((".rs", ".toml", ".rlib", ".rmeta", ".save.json", ".seal", ".replay.json", ".seed")):
        raise SystemExit(f"forbidden source/artifact extension in image: /{rel}")

print("image file-list assertion: PASSED")
PY

ELAPSED_SECS=$(( $(date +%s) - START_EPOCH ))
printf '\n[smoke-container] OK (%ss)\n' "$ELAPSED_SECS"

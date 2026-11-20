#!/usr/bin/env bash
# Offline build and smoke gate for the held Club Mode (`league-tui`) container image.
# Runs in CI (`--network=none`, no login, no registry push) and on Linux hosts via `scripts/verify.sh`.
set -euo pipefail
cd "$(dirname "$0")/.."

IMAGE_TAG="${MERCHANT_LEAGUE_SMOKE_IMAGE:-merchant-league:ci-smoke}"
BASE_TAG="merchant-league:ci-base-$$"
CTX_TAG="merchant-league:ci-ctx-$$"
CTX_POS_TAG="merchant-league:ci-ctx-pos-$$"
CTX_NEG1_TAG="merchant-league:ci-ctx-neg1-$$"
CTX_NEG2_TAG="merchant-league:ci-ctx-neg2-$$"
SAVES_CTR="ml-saves-smoke-$$"
TMP_SMOKE_DIR=$(mktemp -d)
HOST_UID=$(id -u)
HOST_GID=$(id -g)
START_EPOCH=$(date +%s)

cleanup() {
  docker rm -f "$SAVES_CTR" >/dev/null 2>&1 || true
  docker rmi -f "$IMAGE_TAG" "$BASE_TAG" "$CTX_TAG" "$CTX_POS_TAG" "$CTX_NEG1_TAG" "$CTX_NEG2_TAG" >/dev/null 2>&1 || true
  rm -rf "$TMP_SMOKE_DIR"
}
trap cleanup EXIT

step() { printf '\n[smoke-container] == %s\n' "$1"; }

step "0. Check Dockerfile Rust builder tag matches rust-toolchain.toml channel"
TMP_SMOKE_DIR="$TMP_SMOKE_DIR" python3 - <<'PY'
import os
import re

with open("rust-toolchain.toml", encoding="utf-8") as f:
    tc_text = f.read()
with open("Dockerfile", encoding="utf-8") as f:
    df_text = f.read()

m_tc = re.search(r'(?m)^channel\s*=\s*"([^"]+)"\s*$', tc_text)
m_df_rust = re.search(
    r'(?m)^FROM\s+docker\.io/library/rust:([0-9]+(?:\.[0-9]+)*)-bookworm@sha256:[0-9a-f]{64}\s+AS\s+builder\s*$',
    df_text,
)
m_df_base = re.search(
    r'(?m)^FROM\s+(gcr\.io/distroless/cc-debian12:nonroot@sha256:[0-9a-f]{64})\s*$',
    df_text,
)

channel = m_tc.group(1) if m_tc else ""
rust_ver = m_df_rust.group(1) if m_df_rust else ""
base_ref = m_df_base.group(1) if m_df_base else ""
print(f"rust-toolchain.toml channel={channel}; Dockerfile builder rust={rust_ver}; base={base_ref}")

if not channel or not rust_ver or channel != rust_ver or not base_ref:
    raise SystemExit(
        f"toolchain/base parse mismatch: channel={channel!r} rust={rust_ver!r} base={base_ref!r}"
    )

with open(os.path.join(os.environ["TMP_SMOKE_DIR"], "base_image_ref.txt"), "w", encoding="utf-8") as out:
    out.write(base_ref)
PY
BASE_IMAGE_REF=$(cat "$TMP_SMOKE_DIR/base_image_ref.txt")

step "1. Verify real .dockerignore build context (FROM scratch) and negative controls"
cat > "$TMP_SMOKE_DIR/ctx.Dockerfile" <<'EOF'
FROM scratch
COPY . /ctx
EOF

verify_real_context() {
  local ctx_dir="$1"
  local tag="$2"
  local print_summary="$3"
  local build_progress="quiet"
  if [ "$print_summary" = "1" ]; then
    build_progress="plain"
  fi
  docker build --no-cache --progress="$build_progress" -f "$TMP_SMOKE_DIR/ctx.Dockerfile" -t "$tag" "$ctx_dir"
  CTX_TAG_ARG="$tag" PRINT_SUMMARY="$print_summary" python3 - <<'PY'
import io
import json
import os
import posixpath
import subprocess
import tarfile

tag = os.environ["CTX_TAG_ARG"]
print_summary = os.environ["PRINT_SUMMARY"] == "1"

proc = subprocess.Popen(["docker", "image", "save", tag], stdout=subprocess.PIPE)
save_bytes, _ = proc.communicate()
if proc.returncode != 0:
    raise SystemExit(f"docker image save failed for {tag}: {proc.returncode}")

def norm_archive_key(p: str) -> str:
    while p.startswith("./"):
        p = p[2:]
    return p.lstrip("/")


def norm_fs_path(p: str) -> str:
    while p.startswith("./"):
        p = p[2:]
    p = p.strip("/")
    return "" if p == "." else p


with tarfile.open(fileobj=io.BytesIO(save_bytes), mode="r:*") as tf:
    members = {norm_archive_key(m.name): m for m in tf.getmembers()}

    def read_entry(rel_path: str) -> bytes:
        cur = norm_archive_key(rel_path)
        for _ in range(8):
            m = members[cur]
            if m.issym():
                cur = norm_archive_key(posixpath.normpath(posixpath.join(posixpath.dirname(cur), m.linkname)))
                continue
            if m.islnk():
                cur = norm_archive_key(m.linkname)
                continue
            f = tf.extractfile(m)
            if f is None:
                raise SystemExit(f"cannot extract {cur}")
            return f.read()
        raise SystemExit(f"symlink loop for {rel_path}")

    manifest = json.loads(read_entry("manifest.json").decode("utf-8"))[0]
    layers = [norm_archive_key(l) for l in manifest["Layers"]]

    ctx_fs = {}
    for layer_path in layers:
        with tarfile.open(fileobj=io.BytesIO(read_entry(layer_path)), mode="r:*") as ltf:
            for m in ltf:
                rel = norm_fs_path(m.name)
                if not rel or rel == "ctx":
                    continue
                if not rel.startswith("ctx/"):
                    raise SystemExit(f"unexpected path outside /ctx in throwaway context image: /{rel}")
                sub = rel[len("ctx/"):]
                mtype = "dir" if m.isdir() else ("symlink" if m.issym() else ("hardlink" if m.islnk() else "file"))
                ctx_fs[sub] = (mtype, m.size)

expected_top = {"Cargo.toml", "Cargo.lock", "LICENSE", "crates", "vendor"}
top_levels = {p.split("/", 1)[0] for p in ctx_fs}
if top_levels != expected_top:
    raise SystemExit(f"unexpected top-level entries in real build context: {sorted(top_levels)} != {sorted(expected_top)}")

forbidden_segs = {".git", "target", "site", "saves", ".league", "node_modules", "docs", "atlas", "scripts", ".github"}
forbidden_names = {
    ".gitignore", "Dockerfile", ".dockerignore", "rust-toolchain.toml",
    "README.md", "CHANGELOG.md", "SECURITY.md", "SHIP_GATE.md",
}
forbidden_exts = (".save.json", ".seal", ".replay.json", ".seed", ".log")

for p in sorted(ctx_fs):
    parts = p.split("/")
    base = parts[-1]
    if any(seg in forbidden_segs for seg in parts):
        raise SystemExit(f"forbidden directory leaked into real build context: {p}")
    if base.startswith(".env") or base in forbidden_names:
        raise SystemExit(f"forbidden file leaked into real build context: {p}")
    if p.endswith(forbidden_exts):
        raise SystemExit(f"forbidden save/log artifact leaked into real build context: {p}")

if print_summary:
    groups = {}
    total_files = 0
    total_bytes = 0
    for p, (mtype, size) in ctx_fs.items():
        if mtype == "dir":
            continue
        parts = p.split("/")
        grp = "/".join(parts[:2]) if len(parts) >= 2 else parts[0]
        c, b = groups.get(grp, (0, 0))
        groups[grp] = (c + 1, b + size)
        total_files += 1
        total_bytes += size
    for grp in sorted(groups):
        c, b = groups[grp]
        print(f"real context {grp:24s}: {c:4d} files, {b:10d} bytes")
    print(f"real context total allowed     : {total_files} files, {total_bytes} bytes")
PY
}

verify_real_context "." "$CTX_TAG" 1

PROBE_CTX="$TMP_SMOKE_DIR/probe-ctx"
mkdir -p "$PROBE_CTX/crates" "$PROBE_CTX/vendor" "$PROBE_CTX/saves" "$PROBE_CTX/.league" "$PROBE_CTX/site/node_modules/pkg"
printf '[workspace]\n' > "$PROBE_CTX/Cargo.toml"
printf '# lock\n' > "$PROBE_CTX/Cargo.lock"
printf 'MIT\n' > "$PROBE_CTX/LICENSE"
printf 'pub fn ok() {}\n' > "$PROBE_CTX/crates/probe.rs"
printf 'pub fn dep() {}\n' > "$PROBE_CTX/vendor/probe.rs"
printf 'SECRET=root\n' > "$PROBE_CTX/.env"
printf 'SECRET=nested\n' > "$PROBE_CTX/crates/.env.secret"
printf '{}\n' > "$PROBE_CTX/saves/club-1.seal"
printf 'seed\n' > "$PROBE_CTX/.league/key.seed"
printf 'module.exports = {};\n' > "$PROBE_CTX/site/node_modules/pkg/index.js"
printf '# readme\n' > "$PROBE_CTX/README.md"

cp .dockerignore "$PROBE_CTX/.dockerignore"
verify_real_context "$PROBE_CTX" "$CTX_POS_TAG" 0
echo "positive control (planted secrets/saves + real .dockerignore) => exit=0"

grep -v '^\*\*/\.env\*$' .dockerignore > "$PROBE_CTX/.dockerignore"
set +e
NEG1_OUT=$(verify_real_context "$PROBE_CTX" "$CTX_NEG1_TAG" 0 2>&1)
NEG1_RC=$?
set -e
echo "negative control 1 (planted crates/.env.secret + **/.env* removed) => exit=${NEG1_RC} (expected nonzero)"
printf '%s\n' "$NEG1_OUT" | tail -n 5 | sed 's/^/  | /'
[ "$NEG1_RC" -ne 0 ]

: > "$PROBE_CTX/.dockerignore"
set +e
NEG2_OUT=$(verify_real_context "$PROBE_CTX" "$CTX_NEG2_TAG" 0 2>&1)
NEG2_RC=$?
set -e
echo "negative control 2 (planted .env + empty .dockerignore) => exit=${NEG2_RC} (expected nonzero)"
printf '%s\n' "$NEG2_OUT" | tail -n 5 | sed 's/^/  | /'
[ "$NEG2_RC" -ne 0 ]

step "2. Build runtime image (${IMAGE_TAG}; no push)"
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

step "4. Run offline CLI, non-TTY, arbitrary-UID, and no-shell checks (--network=none)"
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

run_check 0 docker run --rm --network=none --user "${HOST_UID}:${HOST_GID}" "$IMAGE_TAG" --help
printf '%s' "$LAST_OUTPUT" | grep -q "Usage: league-tui"

run_check 1 docker run --rm --network=none --user "${HOST_UID}:${HOST_GID}" "$IMAGE_TAG"
printf '%s' "$LAST_OUTPUT" | grep -q "TTY_REQUIRED"

run_check 0 docker run --rm --network=none --user 42424:42424 "$IMAGE_TAG" --help
printf '%s' "$LAST_OUTPUT" | grep -q "Usage: league-tui"

run_check 1 docker run --rm --network=none --user 42424:42424 "$IMAGE_TAG"
printf '%s' "$LAST_OUTPUT" | grep -q "TTY_REQUIRED"

run_check nonzero docker run --rm --network=none --entrypoint /bin/sh "$IMAGE_TAG" -c true
run_check nonzero docker run --rm --network=none --entrypoint /bin/bash "$IMAGE_TAG" -c true

step "5. Prove /saves writeability (default 65532 without mount + documented --user bind mount)"
HOST_DOC_DIR="$TMP_SMOKE_DIR/documented-run"
mkdir -p "$HOST_DOC_DIR/saves"

IMAGE_TAG="$IMAGE_TAG" SAVES_CTR="$SAVES_CTR" HOST_DOC_DIR="$HOST_DOC_DIR" HOST_UID="$HOST_UID" HOST_GID="$HOST_GID" python3 - <<'PY'
import fcntl
import io
import json
import os
import pty
import re
import select
import stat
import struct
import subprocess
import tarfile
import termios
import time

image_tag = os.environ["IMAGE_TAG"]
ctr = os.environ["SAVES_CTR"]
host_doc_dir = os.environ["HOST_DOC_DIR"]
host_uid = int(os.environ["HOST_UID"])
host_gid = int(os.environ["HOST_GID"])
ansi_re = re.compile(r"\x1b\[[0-9;:?]*[A-Za-z]")


def run_pty_lobby_to_first_save(cmd):
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


# 5a. Built-in /saves as default image UID 65532:65532 (no -v mount)
run_pty_lobby_to_first_save([
    "docker", "run", "--name", ctr, "-it", "--network=none", image_tag, "--log-file", "/saves/smoke.log",
])

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
        norm_name = m.name
        while norm_name.startswith("./"):
            norm_name = norm_name[2:]
        norm_name = norm_name.strip("/")
        if not norm_name or norm_name == ".":
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

# 5b. Documented Linux bind-mount command (--user "$(id -u):$(id -g)" -v "<host>/saves:/saves")
host_saves = os.path.join(host_doc_dir, "saves")
run_pty_lobby_to_first_save([
    "docker",
    "run",
    "--rm",
    "-it",
    "--network=none",
    "--user",
    f"{host_uid}:{host_gid}",
    "-v",
    f"{host_saves}:/saves",
    image_tag,
])

host_dir_st = os.stat(host_saves)
print(
    f"host-save-dir: {host_saves} uid={host_dir_st.st_uid} gid={host_dir_st.st_gid} "
    f"mode={oct(stat.S_IMODE(host_dir_st.st_mode))}"
)
for fn in ("club-1.save.json", "club-1.seal"):
    fp = os.path.join(host_saves, fn)
    if not os.path.isfile(fp):
        raise SystemExit(f"missing host save file after documented run: {fp}")
    st = os.stat(fp)
    print(
        f"host-save: {fp} uid={st.st_uid} gid={st.st_gid} "
        f"(expected {host_uid}:{host_gid}) mode={oct(stat.S_IMODE(st.st_mode))} size={st.st_size}"
    )
    if (st.st_uid, st.st_gid) != (host_uid, host_gid):
        raise SystemExit(f"unexpected host file owner for {fp}: {st.st_uid}:{st.st_gid} != {host_uid}:{host_gid}")
    if st.st_size == 0:
        raise SystemExit(f"empty host file after documented run: {fp}")
with open(os.path.join(host_saves, "club-1.save.json"), encoding="utf-8") as sf:
    host_parsed = json.load(sf)
    if host_parsed.get("table_id") != 1:
        raise SystemExit(f"unexpected host save table_id: {host_parsed.get('table_id')}")
    print(f"host-save json table_id={host_parsed.get('table_id')}")
PY

step "6. Diff flattened runtime image file list against pinned distroless base & assert /etc/passwd"
docker pull -q "$BASE_IMAGE_REF" >/dev/null
if ! docker tag "$BASE_IMAGE_REF" "$BASE_TAG" >/dev/null 2>&1; then
  mkdir -p "$TMP_SMOKE_DIR/empty"
  printf 'FROM %s\n' "$BASE_IMAGE_REF" > "$TMP_SMOKE_DIR/base.Dockerfile"
  docker build --quiet -f "$TMP_SMOKE_DIR/base.Dockerfile" -t "$BASE_TAG" "$TMP_SMOKE_DIR/empty" >/dev/null
fi

IMAGE_TAG="$IMAGE_TAG" BASE_TAG="$BASE_TAG" IMAGE_SIZE="$IMAGE_SIZE" HOST_UID="$HOST_UID" python3 - <<'PY'
import hashlib
import io
import json
import os
import posixpath
import subprocess
import tarfile

image_tag = os.environ["IMAGE_TAG"]
base_tag = os.environ["BASE_TAG"]
image_size = os.environ["IMAGE_SIZE"]
host_uid = os.environ["HOST_UID"]


def norm_archive_key(p: str) -> str:
    while p.startswith("./"):
        p = p[2:]
    return p.lstrip("/")


def norm_fs_path(p: str) -> str:
    while p.startswith("./"):
        p = p[2:]
    p = p.strip("/")
    return "" if p == "." else p


def flatten_image(tag: str):
    proc = subprocess.Popen(["docker", "image", "save", tag], stdout=subprocess.PIPE)
    save_bytes, _ = proc.communicate()
    if proc.returncode != 0:
        raise SystemExit(f"docker image save failed for {tag}: {proc.returncode}")

    with tarfile.open(fileobj=io.BytesIO(save_bytes), mode="r:*") as tf:
        members = {norm_archive_key(m.name): m for m in tf.getmembers()}

        def read_entry(rel_path: str) -> bytes:
            cur = norm_archive_key(rel_path)
            for _ in range(8):
                m = members[cur]
                if m.issym():
                    cur = norm_archive_key(posixpath.normpath(posixpath.join(posixpath.dirname(cur), m.linkname)))
                    continue
                if m.islnk():
                    cur = norm_archive_key(m.linkname)
                    continue
                f = tf.extractfile(m)
                if f is None:
                    raise SystemExit(f"cannot extract {rel_path} -> {cur}")
                return f.read()
            raise SystemExit(f"symlink loop in {rel_path}")

        manifest = json.loads(read_entry("manifest.json").decode("utf-8"))[0]
        layers = [norm_archive_key(l) for l in manifest["Layers"]]
        fs = {}
        blobs = {}
        for idx, layer_path in enumerate(layers):
            with tarfile.open(fileobj=io.BytesIO(read_entry(layer_path)), mode="r:*") as ltf:
                for m in ltf:
                    rel = norm_fs_path(m.name)
                    if not rel:
                        continue
                    parent, _, base = rel.rpartition("/")
                    if base == ".wh..wh..opq":
                        prefix = f"{parent}/" if parent else ""
                        for k in [k for k in fs if k.startswith(prefix)]:
                            del fs[k]
                            blobs.pop(k, None)
                        continue
                    if base.startswith(".wh."):
                        victim = f"{parent}/{base[4:]}" if parent else base[4:]
                        for k in [k for k in fs if k == victim or k.startswith(victim + "/")]:
                            del fs[k]
                            blobs.pop(k, None)
                        continue
                    mtype = "dir" if m.isdir() else ("symlink" if m.issym() else ("hardlink" if m.islnk() else "file"))
                    digest = ""
                    if m.isfile():
                        data = ltf.extractfile(m).read()
                        digest = hashlib.sha256(data).hexdigest()
                        if rel in ("etc/passwd",):
                            blobs[rel] = data
                    else:
                        blobs.pop(rel, None)
                    fs[rel] = (
                        mtype,
                        m.mode & 0o7777,
                        m.uid,
                        m.gid,
                        m.size,
                        m.linkname or "",
                        digest,
                    )
        return len(layers), fs, blobs


base_layers, base_fs, _ = flatten_image(base_tag)
runtime_layers, runtime_fs, runtime_blobs = flatten_image(image_tag)

passwd_text = runtime_blobs.get("etc/passwd", b"").decode("utf-8", errors="replace")
passwd_uids = sorted(
    line.split(":")[2]
    for line in passwd_text.splitlines()
    if line.strip() and len(line.split(":")) >= 3
)
print(f"container /etc/passwd UIDs     : {passwd_uids}")
if "42424" in passwd_uids:
    raise SystemExit("unexpected UID 42424 entry in container /etc/passwd")
if host_uid not in ("0", "65532", "65534") and host_uid in passwd_uids:
    raise SystemExit(f"unexpected host UID {host_uid} entry in container /etc/passwd")

removed = sorted(set(base_fs) - set(runtime_fs))
if removed:
    raise SystemExit(f"runtime image removed/whited-out base entries: {removed}")

allowed_copy_ancestors = {"usr", "usr/local", "usr/local/bin", "usr/share", "usr/share/doc"}
modified_nondir = []
modified_other_dirs = []
ancestor_notes = []
for p in sorted(set(base_fs) & set(runtime_fs)):
    if base_fs[p] != runtime_fs[p]:
        btype = base_fs[p][0]
        rtype = runtime_fs[p][0]
        if btype == "dir" and rtype == "dir" and p in allowed_copy_ancestors:
            if runtime_fs[p][2] != 0:
                raise SystemExit(f"COPY ancestor dir {p} changed UID unexpectedly: {base_fs[p]} -> {runtime_fs[p]}")
            ancestor_notes.append(f"/{p} (base uid:gid={base_fs[p][2]}:{base_fs[p][3]} mode={oct(base_fs[p][1])} -> runtime uid:gid={runtime_fs[p][2]}:{runtime_fs[p][3]} mode={oct(runtime_fs[p][1])})")
        elif btype != "dir" or rtype != "dir":
            modified_nondir.append((p, base_fs[p], runtime_fs[p]))
        else:
            modified_other_dirs.append((p, base_fs[p], runtime_fs[p]))

if modified_nondir or modified_other_dirs:
    raise SystemExit(f"runtime image modified base entries: non_dir={modified_nondir} dirs={modified_other_dirs}")

added = sorted(set(runtime_fs) - set(base_fs))
allowed_added_all = {
    "usr", "usr/local", "usr/local/bin", "usr/local/bin/league-tui",
    "usr/share", "usr/share/doc", "usr/share/doc/merchant-league", "usr/share/doc/merchant-league/LICENSE",
    "saves",
}
required_added = {
    "usr/local/bin/league-tui",
    "usr/share/doc/merchant-league",
    "usr/share/doc/merchant-league/LICENSE",
    "saves",
}
added_set = set(added)
if not required_added.issubset(added_set):
    raise SystemExit(f"missing required added paths vs base: {sorted(required_added - added_set)}")
if not added_set.issubset(allowed_added_all):
    raise SystemExit(f"unexpected added paths vs base: {sorted(added_set - allowed_added_all)}")
for p in added:
    if p not in ("usr/local/bin/league-tui", "usr/share/doc/merchant-league/LICENSE") and runtime_fs[p][0] != "dir":
        raise SystemExit(f"added path {p} must be a dir: {runtime_fs[p]}")

print(f"image total size bytes         : {image_size}")
print(f"base image layers / entries    : {base_layers} / {len(base_fs)}")
print(f"runtime image layers / entries : {runtime_layers} / {len(runtime_fs)}")
print(f"removed base entries           : {len(removed)}")
print(f"modified non-dir base entries  : {len(modified_nondir)}")
print(f"COPY ancestor dir notes        : {len(ancestor_notes)} ({', '.join(ancestor_notes) if ancestor_notes else 'none'})")
print(f"added entries vs pinned base ({len(added)}):")
for rel in added:
    mtype, mode, uid, gid, size, _, _ = runtime_fs[rel]
    print(f"  {uid:5d}:{gid:<5d} {oct(mode):>6s} {mtype:7s} {size:10d} /{rel}")

binary = runtime_fs.get("usr/local/bin/league-tui")
if not binary or binary[0] != "file" or binary[4] <= 0 or not (binary[1] & 0o100):
    raise SystemExit(f"missing or non-executable /usr/local/bin/league-tui: {binary}")

lic = runtime_fs.get("usr/share/doc/merchant-league/LICENSE")
with open("LICENSE", "rb") as lf:
    lic_bytes = lf.read()
expected_lic_sha = hashlib.sha256(lic_bytes).hexdigest()
if not lic or lic[0] != "file" or lic[4] != len(lic_bytes) or lic[6] != expected_lic_sha:
    raise SystemExit(f"missing or mismatched LICENSE: {lic} (expected size={len(lic_bytes)} sha256={expected_lic_sha})")

saves_dir = runtime_fs.get("saves")
if not saves_dir or saves_dir[0] != "dir" or (saves_dir[2], saves_dir[3]) != (65532, 65532):
    raise SystemExit(f"/saves dir missing or wrong owner: {saves_dir}")
if any(p.startswith("saves/") for p in runtime_fs):
    raise SystemExit("/saves must be empty in the built image")

forbidden_shells = {"sh", "bash", "dash", "ash", "zsh", "ksh", "fish", "csh", "tcsh"}
forbidden_rust_tools = {"cargo", "rustc", "rustdoc", "rustup", "rust-lld"}
forbidden_cc_bins = {"gcc", "cc", "g++", "c++", "clang", "ld", "as", "ar", "make"}
forbidden_repo_names = {
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "Dockerfile", ".dockerignore",
    "SHIP_GATE.md", "SECURITY.md", "README.md", "CHANGELOG.md", "CLAUDE.md",
}
forbidden_top_dirs = {".git", ".league", "crates", "vendor", "target", "work"}

for rel in runtime_fs:
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

print("image diff against pinned base & deny-list assertion: PASSED")
PY

ELAPSED_SECS=$(( $(date +%s) - START_EPOCH ))
printf '\n[smoke-container] OK (%ss)\n' "$ELAPSED_SECS"

//! Single-container Docker lifecycle for stand-alone `xrpld` under Publisher bounds.

use std::fs::OpenOptions;
use std::io::Read as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use league_ledger::client::Transport;

use crate::client::{StandAlone, StandAloneError, TimedUreq};
use crate::{
    NotStandAlone, OFFICIAL_XRPLD_PINNED_REF, STANDALONE_EXPIRES_AT_LABEL_KEY,
    STANDALONE_HARNESS_LABEL, STANDALONE_NETWORK_ID, stand_alone_xrpld_cfg,
};

/// Process-wide lock guaranteeing at most one stand-alone `xrpld` container runs at a time.
static ACTIVE_CONTAINER_LOCK: Mutex<()> = Mutex::new(());
static CONTAINER_SEQ: AtomicU64 = AtomicU64::new(1);

/// Maximum ephemeral tmpfs bytes permitted by Publisher limit (decimal 5 GB).
pub const MAX_TMPFS_BYTES: u64 = 5_000_000_000;
/// Maximum tmpfs size in MiB that remains `<= 5_000_000_000` bytes (`4_768` MiB).
pub const MAX_TMPFS_MB: u32 = 4_768;
const _: () = assert!(
    (MAX_TMPFS_MB as u64) * 1_024 * 1_024 <= MAX_TMPFS_BYTES
        && ((MAX_TMPFS_MB as u64) + 1) * 1_024 * 1_024 > MAX_TMPFS_BYTES
);

const MAX_CPUS: f64 = 2.0;
const MAX_MEMORY_MB: u64 = 2_048;
/// Hard ceiling on per-container self-expiry watchdog lifetime (`600s` = `10m`).
pub const MAX_CONTAINER_LIFETIME: Duration = Duration::from_secs(600);

const MAX_CAPTURED_OUTPUT_BYTES: u64 = 1_048_576;
const POST_KILL_WAIT_BOUND: Duration = Duration::from_millis(250);
const NORMAL_DRAIN_BOUND: Duration = Duration::from_millis(500);
const CONTAINER_EXPIRES_FORMAT: &str =
    "{{.ID}}\t{{.State}}\t{{.Label \"merchant-league.expires_at\"}}";
const STANDALONE_LOCKFILE_NAME: &str = "merchant-league-standalone-xrpld.lock";

/// Captured subprocess exit status and UTF-8 outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// Exit code (or `-1` if signal-terminated).
    pub status: i32,
    /// Decoded standard output.
    pub stdout: String,
    /// Decoded standard error.
    pub stderr: String,
}

/// Subprocess execution boundary so Docker orchestration tests run without spawning Docker.
pub trait CommandRunner {
    /// Execute `program` with `args`, terminating if elapsed wall time exceeds `timeout`.
    fn run(&self, program: &str, args: &[&str], timeout: Duration)
    -> Result<CommandOutput, String>;
}

impl<R: CommandRunner + ?Sized> CommandRunner for Rc<R> {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, String> {
        (**self).run(program, args, timeout)
    }
}

impl<R: CommandRunner + ?Sized> CommandRunner for Arc<R> {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, String> {
        (**self).run(program, args, timeout)
    }
}

/// Production `docker` CLI runner bounded by explicit poll/kill/drain wall timeouts.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsDockerRunner;

impl CommandRunner for OsDockerRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to spawn {program}: {err}"))?;

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let (out_tx, out_rx) = mpsc::sync_channel::<Vec<u8>>(1);
        let (err_tx, err_rx) = mpsc::sync_channel::<Vec<u8>>(1);

        let _out_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stdout_pipe {
                let _ = pipe.take(MAX_CAPTURED_OUTPUT_BYTES).read_to_end(&mut buf);
            }
            let _ = out_tx.send(buf);
        });
        let _err_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stderr_pipe {
                let _ = pipe.take(MAX_CAPTURED_OUTPUT_BYTES).read_to_end(&mut buf);
            }
            let _ = err_tx.send(buf);
        });

        let deadline = Instant::now() + timeout;
        let exit_status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.code().unwrap_or(-1)),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        wait_child_bounded(&mut child, POST_KILL_WAIT_BOUND);
                        break Err(format!(
                            "{program} timed out after {}s",
                            timeout.as_secs_f32()
                        ));
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    thread::sleep(remaining.min(Duration::from_millis(25)));
                }
                Err(err) => {
                    let _ = child.kill();
                    wait_child_bounded(&mut child, POST_KILL_WAIT_BOUND);
                    break Err(format!("wait on {program} failed: {err}"));
                }
            }
        };

        let drain_bound = if exit_status.is_ok() {
            NORMAL_DRAIN_BOUND
        } else {
            POST_KILL_WAIT_BOUND
        };
        let drain_deadline = Instant::now() + drain_bound;
        let stdout_bytes = recv_bounded(&out_rx, drain_deadline);
        let stderr_bytes = recv_bounded(&err_rx, drain_deadline);
        let status = exit_status?;
        Ok(CommandOutput {
            status,
            stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        })
    }
}

fn wait_child_bounded(child: &mut Child, bound: Duration) {
    let deadline = Instant::now() + bound;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {
                if Instant::now() >= deadline {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn recv_bounded(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> Vec<u8> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    rx.recv_timeout(remaining).unwrap_or_default()
}

/// Resource and image specification for a single stand-alone `xrpld` container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerSpec {
    /// Pinned official image reference (must match `OFFICIAL_XRPLD_PINNED_REF` exactly).
    pub pinned_image: String,
    /// Network ID injected via `[network_id]` (`21337` required).
    pub network_id: u32,
    /// Docker `--cpus` limit (`> 0` and `<= 2`).
    pub cpus: String,
    /// Docker `--memory` limit (`> 0` and `<= 2g`).
    pub memory: String,
    /// Docker `--memory-swap` limit (must equal `--memory`, zero swap expansion).
    pub memory_swap: String,
    /// Ephemeral `/tmp/xrpld` tmpfs mount size in MiB (`1..=4768`, decimal `<= 5 GB`).
    pub tmpfs_size_mb: u32,
    /// Host loopback IP bound for RPC port forwarding (must be `"127.0.0.1"`).
    pub host_bind_ip: String,
    /// Timeout for `docker pull` when the pinned digest is not cached locally.
    pub pull_timeout: Duration,
    /// Timeout for ordinary Docker CLI commands (`ps`, `inspect`, `run`, `port`, `rm`).
    pub cmd_timeout: Duration,
    /// Maximum wait until stand-alone RPC `server_info` satisfies Conflict 11.
    pub startup_timeout: Duration,
    /// Interval between startup readiness polls.
    pub poll_interval: Duration,
    /// In-container self-expiry watchdog duration (`> 0` and `<= 600s`).
    pub max_lifetime: Duration,
}

impl Default for DockerSpec {
    fn default() -> Self {
        Self {
            pinned_image: OFFICIAL_XRPLD_PINNED_REF.to_string(),
            network_id: STANDALONE_NETWORK_ID,
            cpus: "2".to_string(),
            memory: "2g".to_string(),
            memory_swap: "2g".to_string(),
            tmpfs_size_mb: 512,
            host_bind_ip: "127.0.0.1".to_string(),
            pull_timeout: Duration::from_secs(180),
            cmd_timeout: Duration::from_secs(20),
            startup_timeout: Duration::from_secs(45),
            poll_interval: Duration::from_millis(250),
            max_lifetime: Duration::from_secs(300),
        }
    }
}

impl DockerSpec {
    /// Validate that every Publisher limit, exact digest pin, and isolation bound holds.
    pub fn validate(&self) -> Result<(), StandAloneError> {
        if self.pinned_image != OFFICIAL_XRPLD_PINNED_REF {
            return Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
                "image must match exact pinned official reference {OFFICIAL_XRPLD_PINNED_REF}, got {:?}",
                self.pinned_image
            ))));
        }

        let _ = stand_alone_xrpld_cfg(self.network_id)?;

        let cpus: f64 =
            self.cpus.trim().parse().map_err(|_| {
                StandAloneError::Build(format!("invalid cpus limit {:?}", self.cpus))
            })?;
        if !(cpus > 0.0 && cpus <= MAX_CPUS) {
            return Err(StandAloneError::Build(format!(
                "cpus limit {:?} exceeds Publisher max ({MAX_CPUS})",
                self.cpus
            )));
        }

        let mem_mb = parse_memory_limit_mb(&self.memory)?;
        let swap_mb = parse_memory_limit_mb(&self.memory_swap)?;
        if mem_mb == 0 || mem_mb > MAX_MEMORY_MB {
            return Err(StandAloneError::Build(format!(
                "memory limit {:?} must satisfy 0 < memory <= {MAX_MEMORY_MB}m",
                self.memory
            )));
        }
        if swap_mb != mem_mb {
            return Err(StandAloneError::Build(format!(
                "memory_swap {:?} must equal memory {:?} (no swap permitted)",
                self.memory_swap, self.memory
            )));
        }

        if !(1..=MAX_TMPFS_MB).contains(&self.tmpfs_size_mb) {
            return Err(StandAloneError::Build(format!(
                "tmpfs size {}m must be in 1..={MAX_TMPFS_MB}m (decimal <= 5 GB)",
                self.tmpfs_size_mb
            )));
        }
        if self.host_bind_ip != "127.0.0.1" {
            return Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
                "container ports must bind strictly to 127.0.0.1, got {:?}",
                self.host_bind_ip
            ))));
        }
        if self.pull_timeout.is_zero()
            || self.cmd_timeout.is_zero()
            || self.startup_timeout.is_zero()
            || self.poll_interval.is_zero()
        {
            return Err(StandAloneError::Build(
                "DockerSpec timeouts and poll_interval must be positive".into(),
            ));
        }
        if self.max_lifetime.is_zero() || self.max_lifetime > MAX_CONTAINER_LIFETIME {
            return Err(StandAloneError::Build(format!(
                "max_lifetime ({}s) must be in 1..={}s",
                self.max_lifetime.as_secs(),
                MAX_CONTAINER_LIFETIME.as_secs()
            )));
        }
        Ok(())
    }
}

/// Build the `docker run` argument vector enforcing all resource limits, isolated tmpfs, and
/// an in-container self-expiry watchdog.
#[must_use]
pub fn build_run_args(spec: &DockerSpec, container_name: &str, cfg_text: &str) -> Vec<String> {
    let expires_at = current_unix_secs().saturating_add(spec.max_lifetime.as_secs().max(1));
    build_run_args_with_expiry(spec, container_name, cfg_text, expires_at)
}

/// Deterministic variant of [`build_run_args`] attaching an explicit UNIX `expires_at` label.
#[must_use]
pub fn build_run_args_with_expiry(
    spec: &DockerSpec,
    container_name: &str,
    cfg_text: &str,
    expires_at_unix_secs: u64,
) -> Vec<String> {
    let lifetime_secs = spec.max_lifetime.as_secs().max(1);
    let container_script = format!(
        "mkdir -p /tmp/xrpld/db && printf '%s' \"$XRPLD_CFG\" > /tmp/xrpld/xrpld.cfg && \
         /usr/bin/xrpld -a --start --conf /tmp/xrpld/xrpld.cfg & pid=$!; \
         trap 'kill -TERM \"$pid\" 2>/dev/null || true' TERM INT; \
         (sleep {lifetime_secs} && kill -TERM \"$pid\" 2>/dev/null || true; \
          sleep 5 && kill -KILL \"$pid\" 2>/dev/null || true) & wd=$!; \
         wait \"$pid\"; rc=$?; kill \"$wd\" 2>/dev/null || true; exit \"$rc\""
    );
    vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        container_name.to_string(),
        "--stop-timeout=5".to_string(),
        "--label".to_string(),
        STANDALONE_HARNESS_LABEL.to_string(),
        "--label".to_string(),
        format!("{STANDALONE_EXPIRES_AT_LABEL_KEY}={expires_at_unix_secs}"),
        format!("--cpus={}", spec.cpus),
        format!("--memory={}", spec.memory),
        format!("--memory-swap={}", spec.memory_swap),
        "--tmpfs".to_string(),
        format!("/tmp/xrpld:rw,size={}m,mode=1777", spec.tmpfs_size_mb),
        "-p".to_string(),
        format!("{host}::5005", host = spec.host_bind_ip),
        "-e".to_string(),
        format!("XRPLD_CFG={cfg_text}"),
        "--entrypoint".to_string(),
        "/bin/sh".to_string(),
        spec.pinned_image.clone(),
        "-c".to_string(),
        container_script,
    ]
}

/// Parse the ephemeral host port bound on `127.0.0.1` from `docker port <name> 5005/tcp`.
pub fn parse_docker_port(stdout: &str) -> Result<u16, StandAloneError> {
    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(port_str) = line.strip_prefix("127.0.0.1:") {
            let port: u16 = port_str
                .trim()
                .parse()
                .map_err(|_| StandAloneError::Reply(format!("invalid docker port {port_str:?}")))?;
            if port > 0 {
                return Ok(port);
            }
        }
    }
    Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
        "docker port output contained no 127.0.0.1 binding: {stdout:?}"
    ))))
}

struct HostContainerFileLock {
    file: std::fs::File,
}

impl Drop for HostContainerFileLock {
    fn drop(&mut self) {
        // Explicitly unlock before closing the fd so a concurrent `fork()` before `execve()` in
        // another thread cannot keep the kernel open-file-description `flock` alive past `Drop`.
        let _ = self.file.unlock();
    }
}

impl HostContainerFileLock {
    fn try_acquire(lock_path: &Path) -> Result<Self, StandAloneError> {
        if let Some(parent) = lock_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|_| {
                StandAloneError::Transport(format!(
                    "failed to prepare host lock directory for {STANDALONE_LOCKFILE_NAME}"
                ))
            })?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| {
                StandAloneError::Transport(format!(
                    "failed to open host lockfile {STANDALONE_LOCKFILE_NAME}"
                ))
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { file }),
            Err(_) => Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
                "refusing concurrent launch: another process holds {STANDALONE_LOCKFILE_NAME}"
            )))),
        }
    }
}

/// Guarded handle to a single running stand-alone `xrpld` container.
///
/// Dropping the handle (or calling [`Self::teardown`]) always executes `docker rm -f`.
pub struct StandAloneContainer<R: CommandRunner = OsDockerRunner, T: Transport = TimedUreq> {
    runner: R,
    client: StandAlone<T>,
    container_name: String,
    removed: bool,
    _host_lock: HostContainerFileLock,
    _guard: MutexGuard<'static, ()>,
}

impl StandAloneContainer<OsDockerRunner, TimedUreq> {
    /// Start one bounded stand-alone `xrpld` container and verify Conflict 11 readiness.
    pub fn start(spec: DockerSpec) -> Result<Self, StandAloneError> {
        let connect_timeout = spec.cmd_timeout;
        Self::start_with_runner(spec, OsDockerRunner, move |url| {
            StandAlone::connect_loopback(url, connect_timeout)
        })
    }
}

impl<R: CommandRunner, T: Transport> StandAloneContainer<R, T> {
    /// Start a container using injected command runner and client connector (for unit tests).
    pub fn start_with_runner<C>(
        spec: DockerSpec,
        runner: R,
        connector: C,
    ) -> Result<Self, StandAloneError>
    where
        C: Fn(&str) -> Result<StandAlone<T>, StandAloneError>,
    {
        Self::start_with_runner_at(spec, runner, connector, current_unix_secs())
    }

    /// Start a container using injected runner, connector, and explicit UNIX clock (for unit tests).
    pub fn start_with_runner_at<C>(
        spec: DockerSpec,
        runner: R,
        connector: C,
        now_unix_secs: u64,
    ) -> Result<Self, StandAloneError>
    where
        C: Fn(&str) -> Result<StandAlone<T>, StandAloneError>,
    {
        let default_lock = std::env::temp_dir().join(STANDALONE_LOCKFILE_NAME);
        Self::start_with_runner_and_lock_at(spec, runner, connector, now_unix_secs, &default_lock)
    }

    /// Start a container holding both the in-process mutex and an explicit host lockfile path
    /// across sweep, launch, and teardown (`F1` TOCTOU prevention).
    pub fn start_with_runner_and_lock_at<C>(
        spec: DockerSpec,
        runner: R,
        connector: C,
        now_unix_secs: u64,
        lock_path: &Path,
    ) -> Result<Self, StandAloneError>
    where
        C: Fn(&str) -> Result<StandAlone<T>, StandAloneError>,
    {
        spec.validate()?;
        let guard = ACTIVE_CONTAINER_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let host_lock = HostContainerFileLock::try_acquire(lock_path)?;

        sweep_and_check_existing_containers(&runner, &spec, now_unix_secs)?;
        ensure_pinned_image(&runner, &spec)?;

        let seq = CONTAINER_SEQ.fetch_add(1, Ordering::Relaxed);
        let container_name = format!("merchant-league-xrpld-{}-{seq}", std::process::id());
        let cfg_text = stand_alone_xrpld_cfg(spec.network_id)?;
        let expires_at = now_unix_secs.saturating_add(spec.max_lifetime.as_secs().max(1));
        let run_args = build_run_args_with_expiry(&spec, &container_name, &cfg_text, expires_at);
        let arg_refs: Vec<&str> = run_args.iter().map(String::as_str).collect();

        let run_out = match runner.run("docker", &arg_refs, spec.cmd_timeout) {
            Ok(out) => out,
            Err(err) => {
                // Clean up even when `docker run -d` times out after daemon creation.
                let _ = runner.run(
                    "docker",
                    &["rm", "-f", &container_name],
                    Duration::from_secs(15),
                );
                return Err(StandAloneError::Transport(err));
            }
        };
        if run_out.status != 0 {
            let _ = runner.run(
                "docker",
                &["rm", "-f", &container_name],
                Duration::from_secs(15),
            );
            return Err(StandAloneError::Transport(format!(
                "docker run failed (exit {}): {}",
                run_out.status,
                run_out.stderr.trim()
            )));
        }

        match wait_until_ready(&runner, &spec, &container_name, &connector) {
            Ok(client) => Ok(Self {
                runner,
                client,
                container_name,
                removed: false,
                _host_lock: host_lock,
                _guard: guard,
            }),
            Err(err) => {
                let logs = runner
                    .run(
                        "docker",
                        &["logs", "--tail", "80", &container_name],
                        Duration::from_secs(10),
                    )
                    .ok()
                    .map(|out| {
                        format!(
                            "stdout: {} stderr: {}",
                            out.stdout.trim(),
                            out.stderr.trim()
                        )
                    })
                    .unwrap_or_default();
                let _ = runner.run(
                    "docker",
                    &["rm", "-f", &container_name],
                    Duration::from_secs(15),
                );
                Err(StandAloneError::Transport(format!(
                    "{err}; container logs: {logs}"
                )))
            }
        }
    }

    /// Docker container name managed by this handle.
    #[must_use]
    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// Checked loopback RPC client bound to this container's forwarded port.
    #[must_use]
    pub fn client(&self) -> &StandAlone<T> {
        &self.client
    }

    /// Explicitly stop and remove the stand-alone container via `docker rm -f`.
    pub fn teardown(mut self) -> Result<(), StandAloneError> {
        let out = self
            .runner
            .run(
                "docker",
                &["rm", "-f", &self.container_name],
                Duration::from_secs(15),
            )
            .map_err(StandAloneError::Transport)?;
        self.removed = true;
        if out.status != 0 {
            return Err(StandAloneError::Transport(format!(
                "docker rm -f {} failed (exit {}): {}",
                self.container_name,
                out.status,
                out.stderr.trim()
            )));
        }
        Ok(())
    }
}

impl<R: CommandRunner, T: Transport> Drop for StandAloneContainer<R, T> {
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.runner.run(
                "docker",
                &["rm", "-f", &self.container_name],
                Duration::from_secs(15),
            );
            self.removed = true;
        }
    }
}

fn sweep_and_check_existing_containers<R: CommandRunner>(
    runner: &R,
    spec: &DockerSpec,
    now_unix_secs: u64,
) -> Result<(), StandAloneError> {
    let filter = format!("label={STANDALONE_HARNESS_LABEL}");
    let ps_out = runner
        .run(
            "docker",
            &[
                "ps",
                "-a",
                "--filter",
                &filter,
                "--format",
                CONTAINER_EXPIRES_FORMAT,
            ],
            spec.cmd_timeout,
        )
        .map_err(StandAloneError::Transport)?;
    if ps_out.status != 0 {
        return Err(StandAloneError::Transport(format!(
            "docker ps failed (exit {}): {}",
            ps_out.status,
            ps_out.stderr.trim()
        )));
    }
    for raw_line in ps_out.stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let id = parts.next().unwrap_or("").trim();
        if id.is_empty() {
            continue;
        }
        let state = parts.next().unwrap_or("").trim();
        let expires_raw = parts.next().unwrap_or("").trim();
        let expired = expires_raw
            .parse::<u64>()
            .ok()
            .is_some_and(|exp| exp <= now_unix_secs);
        let active_state = state.eq_ignore_ascii_case("running")
            || state.eq_ignore_ascii_case("created")
            || state.eq_ignore_ascii_case("restarting");
        if !active_state || expired {
            let rm_out = runner
                .run("docker", &["rm", "-f", id], Duration::from_secs(15))
                .map_err(StandAloneError::Transport)?;
            if rm_out.status != 0 && !rm_out.stderr.contains("No such container") {
                return Err(StandAloneError::Transport(format!(
                    "failed to sweep stale container {id} (exit {}): {}",
                    rm_out.status,
                    rm_out.stderr.trim()
                )));
            }
        } else {
            return Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
                "refusing concurrent launch: active stand-alone container {id} ({state}) is still running"
            ))));
        }
    }
    Ok(())
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn ensure_pinned_image<R: CommandRunner>(
    runner: &R,
    spec: &DockerSpec,
) -> Result<(), StandAloneError> {
    let inspect = runner
        .run(
            "docker",
            &["image", "inspect", &spec.pinned_image],
            spec.cmd_timeout,
        )
        .map_err(StandAloneError::Transport)?;
    if inspect.status == 0 {
        return Ok(());
    }
    let pull = runner
        .run("docker", &["pull", &spec.pinned_image], spec.pull_timeout)
        .map_err(StandAloneError::Transport)?;
    if pull.status != 0 {
        return Err(StandAloneError::Transport(format!(
            "docker pull {} failed (exit {}): {}",
            spec.pinned_image,
            pull.status,
            pull.stderr.trim()
        )));
    }
    Ok(())
}

fn wait_until_ready<R, T, C>(
    runner: &R,
    spec: &DockerSpec,
    container_name: &str,
    connector: &C,
) -> Result<StandAlone<T>, StandAloneError>
where
    R: CommandRunner,
    T: Transport,
    C: Fn(&str) -> Result<StandAlone<T>, StandAloneError>,
{
    let port_out = runner
        .run(
            "docker",
            &["port", container_name, "5005/tcp"],
            spec.cmd_timeout,
        )
        .map_err(StandAloneError::Transport)?;
    if port_out.status != 0 {
        return Err(StandAloneError::Transport(format!(
            "docker port failed (exit {}): {}",
            port_out.status,
            port_out.stderr.trim()
        )));
    }
    let port = parse_docker_port(&port_out.stdout)?;
    let url = format!("http://{}:{port}", spec.host_bind_ip);
    let deadline = Instant::now() + spec.startup_timeout;
    loop {
        match connector(&url) {
            Ok(client) => return Ok(client),
            Err(err @ StandAloneError::NotStandAlone(_)) => return Err(err),
            Err(err) => {
                if Instant::now() >= deadline {
                    return Err(err);
                }
                thread::sleep(spec.poll_interval);
            }
        }
    }
}

fn parse_memory_limit_mb(text: &str) -> Result<u64, StandAloneError> {
    let trimmed = text.trim().to_ascii_lowercase();
    if let Some(num) = trimmed.strip_suffix('g') {
        let gb: u64 = num.parse().map_err(|_| {
            StandAloneError::Build(format!("invalid gigabyte memory limit {text:?}"))
        })?;
        return gb
            .checked_mul(1_024)
            .ok_or_else(|| StandAloneError::Build(format!("memory limit overflow {text:?}")));
    }
    if let Some(num) = trimmed.strip_suffix('m') {
        return num
            .parse::<u64>()
            .map_err(|_| StandAloneError::Build(format!("invalid megabyte limit {text:?}")));
    }
    Err(StandAloneError::Build(format!(
        "memory limit {text:?} must end with 'm' or 'g'"
    )))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use serde_json::{Value, json};

    use super::*;
    use crate::{OFFICIAL_XRPLD_IMAGE_REPO, STANDALONE_NETWORK_ID};

    struct ScriptedTransport {
        replies: RefCell<VecDeque<Result<Value, String>>>,
    }

    impl ScriptedTransport {
        fn new(replies: Vec<Result<Value, String>>) -> Self {
            Self {
                replies: RefCell::new(VecDeque::from(replies)),
            }
        }
    }

    impl Transport for ScriptedTransport {
        fn post(&self, _url: &str, _body: &Value) -> Result<Value, String> {
            self.replies
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err("no scripted reply".into()))
        }
    }

    struct FakeDockerRunner {
        outcomes: RefCell<VecDeque<Result<CommandOutput, String>>>,
        recorded: RefCell<Vec<Vec<String>>>,
    }

    impl FakeDockerRunner {
        fn new(outcomes: Vec<Result<CommandOutput, String>>) -> Self {
            Self {
                outcomes: RefCell::new(VecDeque::from(outcomes)),
                recorded: RefCell::new(Vec::new()),
            }
        }

        fn commands(&self) -> Vec<Vec<String>> {
            self.recorded.borrow().clone()
        }
    }

    impl CommandRunner for FakeDockerRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            _timeout: Duration,
        ) -> Result<CommandOutput, String> {
            let mut full = Vec::with_capacity(1 + args.len());
            full.push(program.to_string());
            full.extend(args.iter().map(|s| (*s).to_string()));
            self.recorded.borrow_mut().push(full);
            self.outcomes.borrow_mut().pop_front().unwrap_or_else(|| {
                Ok(CommandOutput {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            })
        }
    }

    fn ok_cmd(stdout: &str) -> Result<CommandOutput, String> {
        Ok(CommandOutput {
            status: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    fn fail_cmd(status: i32, stderr: &str) -> Result<CommandOutput, String> {
        Ok(CommandOutput {
            status,
            stdout: String::new(),
            stderr: stderr.to_string(),
        })
    }

    fn standalone_info_connector(
        url: &str,
    ) -> Result<StandAlone<ScriptedTransport>, StandAloneError> {
        StandAlone::with_transport(
            url,
            ScriptedTransport::new(vec![Ok(json!({
                "result": {
                    "status": "success",
                    "info": {
                        "network_id": STANDALONE_NETWORK_ID,
                        "validation_quorum": 0,
                        "peers": 0,
                        "server_state": "full"
                    }
                }
            }))]),
        )
    }

    fn extract_run_container_name(cmds: &[Vec<String>]) -> Option<String> {
        cmds.iter().find_map(|cmd| {
            if cmd.get(1).map(String::as_str) != Some("run") {
                return None;
            }
            let pos = cmd.iter().position(|arg| arg == "--name")?;
            cmd.get(pos + 1).cloned()
        })
    }

    #[test]
    fn docker_spec_validates_image_pin_and_resource_caps() {
        let valid = DockerSpec::default();
        assert!(valid.validate().is_ok());

        // C2: unpinned, repo-prefix spoof, wrong digest, wrong tag, and wrong repo must fail.
        let mut unpinned = valid.clone();
        unpinned.pinned_image = "docker.io/xrplf/xrpld:3.4.1".into();
        assert!(unpinned.validate().is_err());

        let mut prefix_spoof = valid.clone();
        prefix_spoof.pinned_image = format!(
            "{OFFICIAL_XRPLD_IMAGE_REPO}-other:3.4.1@sha256:{}",
            "4".repeat(64)
        );
        assert!(prefix_spoof.validate().is_err());

        let mut wrong_digest = valid.clone();
        wrong_digest.pinned_image = format!(
            "{OFFICIAL_XRPLD_IMAGE_REPO}:3.4.1@sha256:{}",
            "0".repeat(64)
        );
        assert!(wrong_digest.validate().is_err());

        let mut wrong_repo = valid.clone();
        wrong_repo.pinned_image = format!("example.invalid/xrpld:3.4.1@sha256:{}", "a".repeat(64));
        assert!(wrong_repo.validate().is_err());

        // B1: 0 memory, memory > 2048 MiB, and any swap != memory must fail.
        for bad_mem in ["0g", "0m", "3g", "2049m", "4g"] {
            let mut spec = valid.clone();
            spec.memory = bad_mem.into();
            spec.memory_swap = bad_mem.into();
            assert!(spec.validate().is_err(), "expected refusal for {bad_mem}");
        }

        let mut swap_mismatch = valid.clone();
        swap_mismatch.memory = "1g".into();
        swap_mismatch.memory_swap = "2g".into();
        assert!(swap_mismatch.validate().is_err());

        let mut swap_under = valid.clone();
        swap_under.memory = "2g".into();
        swap_under.memory_swap = "1g".into();
        assert!(swap_under.validate().is_err());

        let mut equal_1g = valid.clone();
        equal_1g.memory = "1g".into();
        equal_1g.memory_swap = "1024m".into();
        assert!(equal_1g.validate().is_ok());

        // B1 + C1: tmpfs must be 1..=4768 MiB (<= 5_000_000_000 decimal bytes).
        let mut max_decimal_tmpfs = valid.clone();
        max_decimal_tmpfs.tmpfs_size_mb = MAX_TMPFS_MB;
        assert!(max_decimal_tmpfs.validate().is_ok());

        for bad_tmpfs in [0, MAX_TMPFS_MB + 1, 5_000, 5_120, 6_000] {
            let mut spec = valid.clone();
            spec.tmpfs_size_mb = bad_tmpfs;
            assert!(
                spec.validate().is_err(),
                "expected tmpfs refusal for {bad_tmpfs}m"
            );
        }

        let mut too_many_cpus = valid.clone();
        too_many_cpus.cpus = "4".into();
        assert!(too_many_cpus.validate().is_err());

        let mut zero_lifetime = valid.clone();
        zero_lifetime.max_lifetime = Duration::ZERO;
        assert!(zero_lifetime.validate().is_err());

        let mut huge_lifetime = valid.clone();
        huge_lifetime.max_lifetime = MAX_CONTAINER_LIFETIME + Duration::from_secs(1);
        assert!(huge_lifetime.validate().is_err());

        let mut exposed_bind = valid.clone();
        exposed_bind.host_bind_ip = "0.0.0.0".into();
        assert!(exposed_bind.validate().is_err());

        let mut mainnet_id = valid.clone();
        mainnet_id.network_id = 0;
        assert!(mainnet_id.validate().is_err());
    }

    #[test]
    fn parse_docker_port_requires_loopback_and_valid_port() {
        assert_eq!(parse_docker_port("127.0.0.1:49152\n"), Ok(49_152));
        assert!(parse_docker_port("0.0.0.0:49152\n").is_err());
        assert!(parse_docker_port("").is_err());
        assert!(parse_docker_port("127.0.0.1:0").is_err());
    }

    #[test]
    fn build_run_args_includes_all_publisher_limits() {
        let spec = DockerSpec::default();
        let cfg = "[network_id]\n21337\n";
        let args = build_run_args_with_expiry(&spec, "ml-test-1", cfg, 1_700_000_300);
        assert!(args.contains(&"--stop-timeout=5".to_string()));
        assert!(args.contains(&STANDALONE_HARNESS_LABEL.to_string()));
        assert!(args.contains(&"merchant-league.expires_at=1700000300".to_string()));
        assert!(args.contains(&"--cpus=2".to_string()));
        assert!(args.contains(&"--memory=2g".to_string()));
        assert!(args.contains(&"--memory-swap=2g".to_string()));
        assert!(args.contains(&"/tmp/xrpld:rw,size=512m,mode=1777".to_string()));
        assert!(args.contains(&"127.0.0.1::5005".to_string()));
        assert!(args.contains(&OFFICIAL_XRPLD_PINNED_REF.to_string()));
        assert!(args.iter().any(|arg| arg.contains("sleep 300")));
    }

    #[test]
    fn container_starts_polls_and_tears_down_cleanly() {
        let runner = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd(""),                   // ps -a sweep (clean)
            fail_cmd(1, "no such image"), // inspect misses
            ok_cmd("Pulled"),             // pull succeeds
            ok_cmd("cid-123\n"),          // run succeeds
            ok_cmd("127.0.0.1:55501\n"),  // port returns ephemeral loopback port
            ok_cmd("cid-123\n"),          // teardown rm -f
        ]));

        let container = match StandAloneContainer::start_with_runner(
            DockerSpec::default(),
            Rc::clone(&runner),
            |url| {
                assert_eq!(url, "http://127.0.0.1:55501");
                standalone_info_connector(url)
            },
        ) {
            Ok(c) => c,
            Err(err) => panic!("container start failed: {err}"),
        };

        let expected_name = container.container_name().to_string();
        assert_eq!(container.client().url(), "http://127.0.0.1:55501/");
        assert!(expected_name.starts_with("merchant-league-xrpld-"));
        assert!(container.teardown().is_ok());

        let cmds = runner.commands();
        assert_eq!(cmds.len(), 6);
        assert_eq!(
            cmds.last(),
            Some(&vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                expected_name
            ])
        );
    }

    #[test]
    fn docker_run_timeout_and_nonzero_exit_clean_up_with_rm_f() {
        // A1 path 1: `docker run` returns Err (cmd_timeout fired after daemon creation).
        let runner_timeout = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd(""),                               // ps -a
            ok_cmd("image-present"),                  // image inspect
            Err("docker timed out after 20s".into()), // docker run timeout
            ok_cmd("cleaned-after-timeout\n"),        // rm -f
            ok_cmd(""),                               // subsequent ps -a proves lock released
            ok_cmd("image-present"),                  // subsequent image inspect
            ok_cmd("cid-next\n"),                     // subsequent run
            ok_cmd("127.0.0.1:55504\n"),              // subsequent port
            ok_cmd("cid-next\n"),                     // subsequent teardown
        ]));

        let res_timeout = StandAloneContainer::start_with_runner(
            DockerSpec::default(),
            Rc::clone(&runner_timeout),
            standalone_info_connector,
        );
        assert!(res_timeout.is_err());

        let cmds_after_timeout = runner_timeout.commands();
        let leaked_candidate_name = match extract_run_container_name(&cmds_after_timeout) {
            Some(name) => name,
            None => panic!("expected docker run invocation to record container name"),
        };
        assert_eq!(
            cmds_after_timeout.last(),
            Some(&vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                leaked_candidate_name
            ])
        );

        // Lock was released cleanly so the next container starts and tears down normally.
        let next = match StandAloneContainer::start_with_runner(
            DockerSpec::default(),
            Rc::clone(&runner_timeout),
            standalone_info_connector,
        ) {
            Ok(c) => c,
            Err(err) => panic!("subsequent start after timeout cleanup failed: {err}"),
        };
        assert!(next.teardown().is_ok());

        // A1 path 2: `docker run` returns non-zero status.
        let runner_nonzero = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd(""),
            ok_cmd("image-present"),
            fail_cmd(125, "docker daemon error"),
            ok_cmd(""),
        ]));
        let res_nonzero = StandAloneContainer::start_with_runner(
            DockerSpec::default(),
            Rc::clone(&runner_nonzero),
            standalone_info_connector,
        );
        assert!(res_nonzero.is_err());
        let cmds_nonzero = runner_nonzero.commands();
        let nonzero_name = match extract_run_container_name(&cmds_nonzero) {
            Some(name) => name,
            None => panic!("expected docker run invocation"),
        };
        assert_eq!(
            cmds_nonzero.last(),
            Some(&vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                nonzero_name
            ])
        );
    }

    #[test]
    fn container_cleans_up_with_rm_f_when_startup_fails() {
        // Path 1: NotStandAlone (wrong network_id 0) triggers logs + `rm -f <name>`.
        let runner_wrong_net = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd(""),                   // ps -a
            ok_cmd("image-present"),      // inspect
            ok_cmd("cid-456\n"),          // run
            ok_cmd("127.0.0.1:55502\n"),  // port
            ok_cmd("startup log output"), // logs on failure
            ok_cmd("cid-456\n"),          // rm -f cleanup
        ]));

        let spec = DockerSpec {
            startup_timeout: Duration::from_millis(20),
            poll_interval: Duration::from_millis(5),
            ..Default::default()
        };

        let err = StandAloneContainer::start_with_runner(
            spec.clone(),
            Rc::clone(&runner_wrong_net),
            |url| {
                StandAlone::with_transport(
                    url,
                    ScriptedTransport::new(vec![Ok(json!({
                        "result": {
                            "status": "success",
                            "info": {
                                "network_id": 0,
                                "validation_quorum": 0,
                                "peers": 0,
                                "server_state": "full"
                            }
                        }
                    }))]),
                )
            },
        );
        assert!(err.is_err());
        let cmds = runner_wrong_net.commands();
        let name = match extract_run_container_name(&cmds) {
            Some(name) => name,
            None => panic!("missing run command"),
        };
        assert_eq!(
            cmds.last(),
            Some(&vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                name
            ])
        );

        // Path 2: `docker port` failure triggers `rm -f <name>`.
        let runner_port_err = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd(""),
            ok_cmd("image-present"),
            ok_cmd("cid-port\n"),
            fail_cmd(1, "no forwarded port"),
            ok_cmd("log output"),
            ok_cmd("cid-port\n"),
        ]));
        let err_port = StandAloneContainer::start_with_runner(
            spec.clone(),
            Rc::clone(&runner_port_err),
            standalone_info_connector,
        );
        assert!(err_port.is_err());
        let port_cmds = runner_port_err.commands();
        let port_name = match extract_run_container_name(&port_cmds) {
            Some(n) => n,
            None => panic!("missing run command"),
        };
        assert_eq!(
            port_cmds.last(),
            Some(&vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                port_name
            ])
        );

        // Path 3: `startup_timeout` expiry during connector polling triggers `rm -f <name>`.
        let runner_poll_timeout = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd(""),
            ok_cmd("image-present"),
            ok_cmd("cid-poll\n"),
            ok_cmd("127.0.0.1:55505\n"),
            ok_cmd("booting"),
            ok_cmd("cid-poll\n"),
        ]));
        let err_poll =
            StandAloneContainer::start_with_runner(spec, Rc::clone(&runner_poll_timeout), |_| {
                Err::<StandAlone<ScriptedTransport>, _>(StandAloneError::Transport(
                    "connection refused".into(),
                ))
            });
        assert!(err_poll.is_err());
        let poll_cmds = runner_poll_timeout.commands();
        let poll_name = match extract_run_container_name(&poll_cmds) {
            Some(n) => n,
            None => panic!("missing run command"),
        };
        assert_eq!(
            poll_cmds.last(),
            Some(&vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                poll_name
            ])
        );
    }

    #[test]
    fn dropping_container_without_teardown_invokes_rm_f() {
        let runner = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd(""),
            ok_cmd("image-present"),
            ok_cmd("cid-789\n"),
            ok_cmd("127.0.0.1:55503\n"),
            ok_cmd("cid-789\n"),
        ]));
        let container = match StandAloneContainer::start_with_runner(
            DockerSpec::default(),
            Rc::clone(&runner),
            standalone_info_connector,
        ) {
            Ok(c) => c,
            Err(err) => panic!("container start failed: {err}"),
        };
        let expected_name = container.container_name().to_string();
        assert_eq!(runner.commands().len(), 4);
        drop(container);

        let cmds_after_drop = runner.commands();
        assert_eq!(cmds_after_drop.len(), 5);
        assert_eq!(
            cmds_after_drop.last(),
            Some(&vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                expected_name
            ])
        );
    }

    #[test]
    fn sweep_cleans_stale_containers_and_refuses_active_cross_process_container() {
        let runner_sweep = Rc::new(FakeDockerRunner::new(vec![
            ok_cmd("stale-exited\texited\t9999999\nexpired-run\trunning\t1500\n"),
            ok_cmd("stale-exited\n"),
            ok_cmd("expired-run\n"),
            ok_cmd("image-present"),
            ok_cmd("cid-fresh\n"),
            ok_cmd("127.0.0.1:55509\n"),
            ok_cmd("cid-fresh\n"),
        ]));
        let container = match StandAloneContainer::start_with_runner_at(
            DockerSpec::default(),
            Rc::clone(&runner_sweep),
            standalone_info_connector,
            2_000,
        ) {
            Ok(c) => c,
            Err(err) => panic!("sweep + start failed: {err}"),
        };
        assert!(container.teardown().is_ok());
        let cmds = runner_sweep.commands();
        assert_eq!(
            cmds[1],
            vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                "stale-exited".to_string()
            ]
        );
        assert_eq!(
            cmds[2],
            vec![
                "docker".to_string(),
                "rm".to_string(),
                "-f".to_string(),
                "expired-run".to_string()
            ]
        );

        // Active unexpired container stops launch before image inspect / run.
        let runner_busy = Rc::new(FakeDockerRunner::new(vec![ok_cmd(
            "active-other\trunning\t5000\n",
        )]));
        let busy_res = StandAloneContainer::start_with_runner_at(
            DockerSpec::default(),
            Rc::clone(&runner_busy),
            standalone_info_connector,
            2_000,
        );
        assert!(matches!(busy_res, Err(StandAloneError::NotStandAlone(_))));
        assert_eq!(runner_busy.commands().len(), 1);

        // Active running container with missing expiry label also stops launch.
        let runner_nolabel = Rc::new(FakeDockerRunner::new(vec![ok_cmd(
            "active-nolabel\trunning\t\n",
        )]));
        let nolabel_res = StandAloneContainer::start_with_runner_at(
            DockerSpec::default(),
            Rc::clone(&runner_nolabel),
            standalone_info_connector,
            2_000,
        );
        assert!(matches!(
            nolabel_res,
            Err(StandAloneError::NotStandAlone(_))
        ));
        assert_eq!(runner_nolabel.commands().len(), 1);
    }

    #[test]
    fn os_docker_runner_executes_nulls_stdin_and_bounds_timeout_drain() {
        let runner = OsDockerRunner;
        let out = runner.run("cargo", &["--version"], Duration::from_secs(5));
        assert!(matches!(out, Ok(ref res) if res.status == 0 && res.stdout.contains("cargo")));

        // Verify stdin is Stdio::null() (stdin reader sees immediate EOF without hanging).
        #[cfg(windows)]
        let eof_out = runner.run(
            "cmd",
            &["/c", "find", "/c", "/v", ""],
            Duration::from_secs(2),
        );
        #[cfg(not(windows))]
        let eof_out = runner.run("sh", &["-c", "cat"], Duration::from_secs(2));
        assert!(eof_out.is_ok());

        // C4: verify timeout kills child and bounds post-kill pipe drain even if a grandchild keeps pipes open.
        let started = Instant::now();
        #[cfg(windows)]
        let timeout_res = runner.run(
            "cmd",
            &["/c", "ping", "-n", "5", "127.0.0.1"],
            Duration::from_millis(120),
        );
        #[cfg(not(windows))]
        let timeout_res = runner.run(
            "sh",
            &["-c", "sleep 4 & sleep 4"],
            Duration::from_millis(120),
        );
        let elapsed = started.elapsed();

        assert!(
            matches!(timeout_res, Err(ref msg) if msg.contains("timed out")),
            "expected timeout error, got {timeout_res:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "timeout + bounded pipe drain took {elapsed:?} (expected < 2s)"
        );
    }

    #[test]
    fn f1_cross_lock_refuses_concurrent_container_launch() {
        let Ok(tmp) = tempfile::NamedTempFile::new() else {
            panic!("temp lockfile failed");
        };
        let lock_path = tmp.path().to_path_buf();
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path);
        let Ok(file) = holder else {
            panic!("open lockfile failed");
        };
        assert!(file.try_lock().is_ok(), "external lock acquire failed");

        let runner = Rc::new(FakeDockerRunner::new(vec![ok_cmd("")]));
        let res = StandAloneContainer::start_with_runner_and_lock_at(
            DockerSpec::default(),
            Rc::clone(&runner),
            standalone_info_connector,
            2_000,
            &lock_path,
        );
        assert!(
            matches!(res, Err(StandAloneError::NotStandAlone(_))),
            "expected NotStandAlone while external handle holds host lockfile"
        );
        assert!(
            runner.commands().is_empty(),
            "refusal under host lockfile must happen before any docker ps/run"
        );
        let _ = file.unlock();
        drop(file);

        // Also confirm an unexpired `created` container is treated as active rather than swept.
        let Ok(tmp_created) = tempfile::NamedTempFile::new() else {
            panic!("temp lockfile for created-state check failed");
        };
        let runner_created = Rc::new(FakeDockerRunner::new(vec![ok_cmd(
            "active-creating\tcreated\t5000\n",
        )]));
        let created_res = StandAloneContainer::start_with_runner_and_lock_at(
            DockerSpec::default(),
            Rc::clone(&runner_created),
            standalone_info_connector,
            2_000,
            tmp_created.path(),
        );
        assert!(matches!(
            created_res,
            Err(StandAloneError::NotStandAlone(_))
        ));
        assert_eq!(runner_created.commands().len(), 1);
    }
}

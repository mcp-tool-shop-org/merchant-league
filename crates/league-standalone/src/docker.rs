//! Single-container Docker lifecycle for stand-alone `xrpld` under Publisher bounds.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use league_ledger::client::Transport;

use crate::client::{StandAlone, StandAloneError, TimedUreq};
use crate::{
    NotStandAlone, OFFICIAL_XRPLD_IMAGE_REPO, OFFICIAL_XRPLD_PINNED_REF, STANDALONE_HARNESS_LABEL,
    STANDALONE_NETWORK_ID, stand_alone_xrpld_cfg,
};

/// Process-wide lock guaranteeing at most one stand-alone `xrpld` container runs at a time.
static ACTIVE_CONTAINER_LOCK: Mutex<()> = Mutex::new(());
static CONTAINER_SEQ: AtomicU64 = AtomicU64::new(1);

const MAX_TMPFS_MB: u32 = 5_120;
const MAX_CPUS: f64 = 2.0;
const MAX_MEMORY_MB: u64 = 2_048;

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

/// Production `docker` CLI runner bounded by explicit poll/kill wall timeouts.
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
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to spawn {program}: {err}"))?;

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let out_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = stdout_pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });
        let err_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = stderr_pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });

        let deadline = Instant::now() + timeout;
        let exit_status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.code().unwrap_or(-1)),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(format!(
                            "{program} timed out after {}s",
                            timeout.as_secs_f32()
                        ));
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!("wait on {program} failed: {err}"));
                }
            }
        };

        let stdout_bytes = out_handle.join().unwrap_or_default();
        let stderr_bytes = err_handle.join().unwrap_or_default();
        let status = exit_status?;
        Ok(CommandOutput {
            status,
            stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        })
    }
}

/// Resource and image specification for a single stand-alone `xrpld` container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerSpec {
    /// Pinned official image reference including `@sha256:<64-hex>`.
    pub pinned_image: String,
    /// Network ID injected via `[network_id]` (`21337` required).
    pub network_id: u32,
    /// Docker `--cpus` limit (`<= 2`).
    pub cpus: String,
    /// Docker `--memory` limit (`<= 2g`).
    pub memory: String,
    /// Docker `--memory-swap` limit (`<= 2g`).
    pub memory_swap: String,
    /// Ephemeral `/tmp/xrpld` tmpfs mount size in MiB (`1..=5120`).
    pub tmpfs_size_mb: u32,
    /// Host loopback IP bound for RPC port forwarding (must be `"127.0.0.1"`).
    pub host_bind_ip: String,
    /// Timeout for `docker pull` when the pinned digest is not cached locally.
    pub pull_timeout: Duration,
    /// Timeout for ordinary Docker CLI commands (`inspect`, `run`, `port`, `rm`).
    pub cmd_timeout: Duration,
    /// Maximum wait until stand-alone RPC `server_info` satisfies Conflict 11.
    pub startup_timeout: Duration,
    /// Interval between startup readiness polls.
    pub poll_interval: Duration,
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
        }
    }
}

impl DockerSpec {
    /// Validate that every Publisher limit, digest pin, and Conflict 11 bound holds.
    pub fn validate(&self) -> Result<(), StandAloneError> {
        if !self.pinned_image.starts_with(OFFICIAL_XRPLD_IMAGE_REPO) {
            return Err(StandAloneError::NotStandAlone(NotStandAlone::new(format!(
                "image must belong to {OFFICIAL_XRPLD_IMAGE_REPO}, got {:?}",
                self.pinned_image
            ))));
        }
        let Some((_, digest)) = self.pinned_image.split_once("@sha256:") else {
            return Err(StandAloneError::NotStandAlone(NotStandAlone::new(
                "image reference must be pinned by @sha256:<digest>",
            )));
        };
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StandAloneError::NotStandAlone(NotStandAlone::new(
                "pinned @sha256 digest must contain exactly 64 hex characters",
            )));
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
        if mem_mb > MAX_MEMORY_MB || swap_mb > MAX_MEMORY_MB {
            return Err(StandAloneError::Build(format!(
                "memory/swap limits ({}, {}) exceed {MAX_MEMORY_MB}m limit",
                self.memory, self.memory_swap
            )));
        }

        if !(1..=MAX_TMPFS_MB).contains(&self.tmpfs_size_mb) {
            return Err(StandAloneError::Build(format!(
                "tmpfs size {}m must be in 1..={MAX_TMPFS_MB}m",
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
        Ok(())
    }
}

/// Build the `docker run` argument vector enforcing all resource limits and isolated tmpfs.
#[must_use]
pub fn build_run_args(spec: &DockerSpec, container_name: &str, cfg_text: &str) -> Vec<String> {
    vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        container_name.to_string(),
        "--label".to_string(),
        STANDALONE_HARNESS_LABEL.to_string(),
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
        "mkdir -p /tmp/xrpld/db && printf '%s' \"$XRPLD_CFG\" > /tmp/xrpld/xrpld.cfg && exec /usr/bin/xrpld -a --start --conf /tmp/xrpld/xrpld.cfg".to_string(),
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

/// Guarded handle to a single running stand-alone `xrpld` container.
///
/// Dropping the handle (or calling [`Self::teardown`]) always executes `docker rm -f`.
pub struct StandAloneContainer<R: CommandRunner = OsDockerRunner, T: Transport = TimedUreq> {
    runner: R,
    client: StandAlone<T>,
    container_name: String,
    removed: bool,
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
        spec.validate()?;
        let guard = ACTIVE_CONTAINER_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        ensure_pinned_image(&runner, &spec)?;

        let seq = CONTAINER_SEQ.fetch_add(1, Ordering::Relaxed);
        let container_name = format!("merchant-league-xrpld-{}-{seq}", std::process::id());
        let cfg_text = stand_alone_xrpld_cfg(spec.network_id)?;
        let run_args = build_run_args(&spec, &container_name, &cfg_text);
        let arg_refs: Vec<&str> = run_args.iter().map(String::as_str).collect();

        let run_out = runner
            .run("docker", &arg_refs, spec.cmd_timeout)
            .map_err(StandAloneError::Transport)?;
        if run_out.status != 0 {
            let _ = runner.run("docker", &["rm", "-f", &container_name], spec.cmd_timeout);
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
    use crate::STANDALONE_NETWORK_ID;

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

    #[test]
    fn docker_spec_validates_image_pin_and_resource_caps() {
        let valid = DockerSpec::default();
        assert!(valid.validate().is_ok());

        let mut unpinned = valid.clone();
        unpinned.pinned_image = "docker.io/xrplf/xrpld:3.4.1".into();
        assert!(unpinned.validate().is_err());

        let mut wrong_repo = valid.clone();
        wrong_repo.pinned_image = format!("example.invalid/xrpld:3.4.1@sha256:{}", "a".repeat(64));
        assert!(wrong_repo.validate().is_err());

        let mut too_many_cpus = valid.clone();
        too_many_cpus.cpus = "4".into();
        assert!(too_many_cpus.validate().is_err());

        let mut too_much_ram = valid.clone();
        too_much_ram.memory = "4g".into();
        assert!(too_much_ram.validate().is_err());

        let mut too_big_tmpfs = valid.clone();
        too_big_tmpfs.tmpfs_size_mb = 6_000;
        assert!(too_big_tmpfs.validate().is_err());

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
        let args = build_run_args(&spec, "ml-test-1", cfg);
        assert!(args.contains(&"--cpus=2".to_string()));
        assert!(args.contains(&"--memory=2g".to_string()));
        assert!(args.contains(&"--memory-swap=2g".to_string()));
        assert!(args.contains(&"/tmp/xrpld:rw,size=512m,mode=1777".to_string()));
        assert!(args.contains(&"127.0.0.1::5005".to_string()));
        assert!(args.contains(&OFFICIAL_XRPLD_PINNED_REF.to_string()));
    }

    #[test]
    fn container_starts_polls_and_tears_down_cleanly() {
        let runner = FakeDockerRunner::new(vec![
            fail_cmd(1, "no such image"), // inspect misses
            ok_cmd("Pulled"),             // pull succeeds
            ok_cmd("cid-123\n"),          // run succeeds
            ok_cmd("127.0.0.1:55501\n"),  // port returns ephemeral loopback port
            ok_cmd("cid-123\n"),          // teardown rm -f
        ]);

        let container =
            match StandAloneContainer::start_with_runner(DockerSpec::default(), runner, |url| {
                assert_eq!(url, "http://127.0.0.1:55501");
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
            }) {
                Ok(c) => c,
                Err(err) => panic!("container start failed: {err}"),
            };

        assert_eq!(container.client().url(), "http://127.0.0.1:55501/");
        assert!(
            container
                .container_name()
                .starts_with("merchant-league-xrpld-")
        );
        assert!(container.teardown().is_ok());
    }

    #[test]
    fn container_cleans_up_with_rm_f_when_startup_fails() {
        let runner = FakeDockerRunner::new(vec![
            ok_cmd("image-present"),
            ok_cmd("cid-456\n"),
            ok_cmd("127.0.0.1:55502\n"),
            ok_cmd("startup log output"), // logs on failure
            ok_cmd("cid-456\n"),          // rm -f cleanup
        ]);

        let spec = DockerSpec {
            startup_timeout: Duration::from_millis(20),
            poll_interval: Duration::from_millis(5),
            ..Default::default()
        };

        let err = StandAloneContainer::start_with_runner(spec, runner, |url| {
            // Returning NotStandAlone (wrong network_id 0) aborts startup immediately and triggers rm -f
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
        });
        assert!(err.is_err());
    }

    #[test]
    fn dropping_container_without_teardown_invokes_rm_f() {
        let runner = FakeDockerRunner::new(vec![
            ok_cmd("image-present"),
            ok_cmd("cid-789\n"),
            ok_cmd("127.0.0.1:55503\n"),
            ok_cmd("cid-789\n"),
        ]);
        let container =
            match StandAloneContainer::start_with_runner(DockerSpec::default(), runner, |url| {
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
            }) {
                Ok(c) => c,
                Err(err) => panic!("container start failed: {err}"),
            };
        let cmds_before_drop = container.runner.commands().len();
        assert_eq!(cmds_before_drop, 3);
        drop(container);
    }

    #[test]
    fn os_docker_runner_executes_and_enforces_timeouts() {
        let runner = OsDockerRunner;
        // Run `cargo --version` or a quick built-in binary rather than spawning docker.
        let out = runner.run("cargo", &["--version"], Duration::from_secs(5));
        assert!(matches!(out, Ok(ref res) if res.status == 0 && res.stdout.contains("cargo")));
    }
}

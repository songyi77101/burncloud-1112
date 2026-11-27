//! 受信任 llama-server 的身份、依赖与设备能力探测。

use chrono::Utc;
use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const MAX_SUMMARY_BYTES: usize = 512;
const COPY_BUFFER_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RequestedBackend {
    Auto,
    Cpu,
    Metal,
}

impl RequestedBackend {
    #[allow(
        dead_code,
        reason = "自动部署 backend 请求将在任务 032 接入运行时准备契约"
    )]
    pub(super) fn parse(value: &str) -> Result<Self, RuntimeProbeError> {
        match value {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "metal" => Ok(Self::Metal),
            "cuda" => Err(RuntimeProbeError::UnsupportedBackend),
            _ => Err(RuntimeProbeError::UnsupportedBackend),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeProbeEntry {
    pub(super) target: String,
    pub(super) server_relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeProbeSummary {
    pub(super) version_output: String,
    pub(super) help_probe_passed: bool,
    pub(super) dependencies_loaded: bool,
    pub(super) metal_device_count: Option<u32>,
    pub(super) effective_backend: String,
    pub(super) probed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedRuntimeCandidate {
    pub(super) executable: PathBuf,
    pub(super) summary: RuntimeProbeSummary,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RuntimeProbeError {
    InvalidArchive,
    Failed,
    DeviceUnavailable,
    UnsupportedBackend,
    UnsupportedPlatform,
    Cancelled,
}

impl RuntimeProbeError {
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::InvalidArchive => "runtime_archive_invalid",
            Self::Failed => "runtime_probe_failed",
            Self::DeviceUnavailable => "runtime_device_unavailable",
            Self::UnsupportedBackend => "unsupported_backend",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for RuntimeProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidArchive => "运行时归档指定的服务端路径无效",
            Self::Failed => "llama-server 运行时探测失败",
            Self::DeviceUnavailable => "请求的 Metal 设备不可用",
            Self::UnsupportedBackend => "当前 Gitee 运行时不支持请求的 backend",
            Self::UnsupportedPlatform => "当前目标平台没有受支持的运行时",
            Self::Cancelled => "运行时探测已取消",
        })
    }
}

impl Error for RuntimeProbeError {}

#[derive(Debug)]
struct ProbeOutput {
    text: String,
    successful: bool,
    truncated: bool,
}

#[derive(Debug)]
struct LimitedBytes {
    bytes: Vec<u8>,
    truncated: bool,
}

enum ProbeCompletion {
    Completed(Result<ProbeOutput, RuntimeProbeError>),
    Cancelled,
    TimedOut,
}

/// 使用固定参数探测清单指定的服务端，不经 shell 执行或暴露子进程诊断。
pub(super) async fn probe_runtime(
    extracted_root: &Path,
    entry: &RuntimeProbeEntry,
    requested_backend: RequestedBackend,
    cancellation: &CancellationToken,
) -> Result<PreparedRuntimeCandidate, RuntimeProbeError> {
    if !is_supported_target(&entry.target) {
        return Err(RuntimeProbeError::UnsupportedPlatform);
    }
    if requested_backend == RequestedBackend::Metal && !is_macos_target(&entry.target) {
        return Err(RuntimeProbeError::UnsupportedBackend);
    }

    let executable = locate_server(extracted_root, entry)?;
    if cancellation.is_cancelled() {
        return Err(RuntimeProbeError::Cancelled);
    }

    let version = Box::pin(run_probe_process(&executable, "--version", cancellation)).await?;
    let version_output = validate_probe_output(&version)?;
    let help = Box::pin(run_probe_process(&executable, "--help", cancellation)).await?;
    validate_help_probe(&help)?;

    let metal_device_count =
        if is_macos_target(&entry.target) && requested_backend != RequestedBackend::Cpu {
            let devices = Box::pin(run_probe_process(
                &executable,
                "--list-devices",
                cancellation,
            ))
            .await?;
            validate_probe_output(&devices)?;
            Some(count_metal_devices(&devices.text))
        } else {
            None
        };
    let effective_backend =
        select_effective_backend(&entry.target, requested_backend, metal_device_count)?;

    Ok(PreparedRuntimeCandidate {
        executable,
        summary: RuntimeProbeSummary {
            version_output,
            help_probe_passed: true,
            dependencies_loaded: true,
            metal_device_count,
            effective_backend: effective_backend.to_string(),
            probed_at: Utc::now().to_rfc3339(),
        },
    })
}

fn is_supported_target(target: &str) -> bool {
    matches!(
        target,
        "x86_64-pc-windows-msvc"
            | "x86_64-apple-darwin"
            | "aarch64-apple-darwin"
            | "x86_64-unknown-linux-gnu"
    )
}

fn is_macos_target(target: &str) -> bool {
    matches!(target, "x86_64-apple-darwin" | "aarch64-apple-darwin")
}

fn select_effective_backend(
    target: &str,
    requested_backend: RequestedBackend,
    metal_device_count: Option<u32>,
) -> Result<&'static str, RuntimeProbeError> {
    if !is_supported_target(target) {
        return Err(RuntimeProbeError::UnsupportedPlatform);
    }
    match requested_backend {
        RequestedBackend::Cpu => Ok("cpu"),
        RequestedBackend::Metal if !is_macos_target(target) => {
            Err(RuntimeProbeError::UnsupportedBackend)
        }
        RequestedBackend::Metal if metal_device_count.unwrap_or_default() == 0 => {
            Err(RuntimeProbeError::DeviceUnavailable)
        }
        RequestedBackend::Metal => Ok("metal"),
        RequestedBackend::Auto if is_macos_target(target) => {
            if metal_device_count.unwrap_or_default() > 0 {
                Ok("metal")
            } else {
                Ok("cpu")
            }
        }
        RequestedBackend::Auto => Ok("cpu"),
    }
}

fn locate_server(
    extracted_root: &Path,
    entry: &RuntimeProbeEntry,
) -> Result<PathBuf, RuntimeProbeError> {
    let root_metadata =
        fs::symlink_metadata(extracted_root).map_err(|_| RuntimeProbeError::InvalidArchive)?;
    if !root_metadata.file_type().is_dir() {
        return Err(RuntimeProbeError::InvalidArchive);
    }
    let canonical_root =
        fs::canonicalize(extracted_root).map_err(|_| RuntimeProbeError::InvalidArchive)?;
    let relative = &entry.server_relative_path;
    if relative.is_empty() || relative.len() > 4096 || relative.contains('\0') {
        return Err(RuntimeProbeError::InvalidArchive);
    }
    let normalized = relative.replace('\\', "/");
    if normalized.starts_with('/') || normalized.as_bytes().get(1) == Some(&b':') {
        return Err(RuntimeProbeError::InvalidArchive);
    }
    let expected_name = if entry.target == "x86_64-pc-windows-msvc" {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let segments = normalized.split('/').collect::<Vec<_>>();
    if segments.is_empty()
        || segments.iter().any(|segment| {
            segment.is_empty() || *segment == "." || *segment == ".." || segment.contains(':')
        })
        || !segments
            .last()
            .is_some_and(|name| name.eq_ignore_ascii_case(expected_name))
    {
        return Err(RuntimeProbeError::InvalidArchive);
    }

    let mut current = canonical_root.clone();
    for (index, segment) in segments.iter().enumerate() {
        current.push(segment);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| RuntimeProbeError::InvalidArchive)?;
        let is_last = index + 1 == segments.len();
        if metadata.file_type().is_symlink()
            || (is_last && !metadata.file_type().is_file())
            || (!is_last && !metadata.file_type().is_dir())
        {
            return Err(RuntimeProbeError::InvalidArchive);
        }
    }
    let executable = fs::canonicalize(current).map_err(|_| RuntimeProbeError::InvalidArchive)?;
    if !executable.starts_with(&canonical_root) {
        return Err(RuntimeProbeError::InvalidArchive);
    }
    Ok(executable)
}

fn validate_probe_output(output: &ProbeOutput) -> Result<String, RuntimeProbeError> {
    if !output.successful || output.truncated || output.text.trim().is_empty() {
        return Err(RuntimeProbeError::Failed);
    }
    let summary = output
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut bounded = String::new();
    for character in summary.chars() {
        if bounded.len() + character.len_utf8() > MAX_SUMMARY_BYTES {
            break;
        }
        if !character.is_control() {
            bounded.push(character);
        }
    }
    if bounded.is_empty() {
        return Err(RuntimeProbeError::Failed);
    }
    Ok(bounded)
}

fn validate_help_probe(output: &ProbeOutput) -> Result<(), RuntimeProbeError> {
    validate_probe_output(output)?;
    let identity = output.text.to_ascii_lowercase();
    if !identity.contains("llama-server") && !identity.contains("llama.cpp") {
        return Err(RuntimeProbeError::Failed);
    }
    Ok(())
}
fn count_metal_devices(output: &str) -> u32 {
    let count = output
        .lines()
        .filter(|line| {
            let line = line.trim().to_ascii_lowercase();
            line.starts_with("metal")
                && line.as_bytes().get(5).is_some_and(u8::is_ascii_digit)
                && line.contains(':')
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

async fn run_probe_process(
    executable: &Path,
    argument: &'static str,
    cancellation: &CancellationToken,
) -> Result<ProbeOutput, RuntimeProbeError> {
    if cancellation.is_cancelled() {
        return Err(RuntimeProbeError::Cancelled);
    }
    let mut command = Command::new(executable);
    command
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(parent) = executable.parent() {
        command.current_dir(parent);
    }
    let mut child = command.spawn().map_err(|_| RuntimeProbeError::Failed)?;
    let stdout = child.stdout.take().ok_or(RuntimeProbeError::Failed)?;
    let stderr = child.stderr.take().ok_or(RuntimeProbeError::Failed)?;
    let half_limit = MAX_CAPTURE_BYTES / 2;
    let outcome = {
        let completion = async {
            let stdout = read_limited(stdout, half_limit);
            let stderr = read_limited(stderr, half_limit);
            let status = child.wait();
            let (stdout, stderr, status) = tokio::join!(stdout, stderr, status);
            let stdout = stdout.map_err(|_| RuntimeProbeError::Failed)?;
            let stderr = stderr.map_err(|_| RuntimeProbeError::Failed)?;
            let status = status.map_err(|_| RuntimeProbeError::Failed)?;
            let mut bytes = stdout.bytes;
            bytes.extend_from_slice(&stderr.bytes);
            Ok(ProbeOutput {
                text: String::from_utf8_lossy(&bytes).into_owned(),
                successful: status.success(),
                truncated: stdout.truncated || stderr.truncated,
            })
        };
        tokio::pin!(completion);
        tokio::select! {
            result = &mut completion => ProbeCompletion::Completed(result),
            _ = cancellation.cancelled() => ProbeCompletion::Cancelled,
            _ = tokio::time::sleep(PROBE_TIMEOUT) => ProbeCompletion::TimedOut,
        }
    };
    match outcome {
        ProbeCompletion::Completed(result) => result,
        ProbeCompletion::Cancelled => {
            terminate_child(&mut child).await?;
            Err(RuntimeProbeError::Cancelled)
        }
        ProbeCompletion::TimedOut => {
            terminate_child(&mut child).await?;
            Err(RuntimeProbeError::Failed)
        }
    }
}

async fn terminate_child(child: &mut Child) -> Result<(), RuntimeProbeError> {
    match child.start_kill() {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
        Err(_) => return Err(RuntimeProbeError::Failed),
    }
    child.wait().await.map_err(|_| RuntimeProbeError::Failed)?;
    Ok(())
}

async fn read_limited<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> io::Result<LimitedBytes> {
    let mut bytes = Vec::with_capacity(limit.min(COPY_BUFFER_BYTES));
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let available = limit.saturating_sub(bytes.len());
        let retained = read.min(available);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    Ok(LimitedBytes { bytes, truncated })
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;

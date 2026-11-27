use async_trait::async_trait;
use burncloud_download::{download_runtime_archive, RuntimeDownloadProgress};
use burncloud_node_runtime::{
    ModelUrlRuntimeAdapter, PreparedArtifact, PreparedRuntime, ProcessPlan, ProcessSpec,
    ReadinessTarget, RuntimeAdapter, RuntimeAdapterError, RuntimePrepareError, RuntimePreparer,
    RuntimeRequest,
};
use reqwest::Url;
use std::{
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use tempfile::NamedTempFile;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

mod archive;
mod cache;
mod manifest;
mod probe;

use archive::{extract_runtime_archive, RuntimeArchiveError, RuntimeArchiveLimits};
use cache::{
    acquire_runtime_install_lock, check_cached_runtime, commit_runtime_staging,
    create_runtime_staging, quarantine_cached_runtime, read_runtime_cache_root,
    runtime_download_paths, RuntimeCacheError, RuntimeCacheKey, RuntimeCacheSpec,
    RuntimeInstallLockError, RuntimeInstallPhase, RuntimeInstallProgress, RuntimeInstallRegistry,
    RuntimeInstallStatus,
};
use manifest::{resolve_runtime_entry, ResolvedRuntimeEntry, RuntimeManifestError};
use probe::{
    probe_runtime, PreparedRuntimeCandidate, RequestedBackend, RuntimeProbeEntry, RuntimeProbeError,
};

static RUNTIME_INSTALLS: OnceLock<RuntimeInstallRegistry> = OnceLock::new();

#[derive(Debug, Default)]
pub(crate) struct LlamaCppRuntimePreparer;

#[async_trait]
impl RuntimePreparer for LlamaCppRuntimePreparer {
    fn supported_runtimes(&self) -> Vec<String> {
        vec!["llama.cpp".to_string()]
    }

    async fn prepare(
        &self,
        request: RuntimeRequest,
    ) -> Result<PreparedRuntime, RuntimePrepareError> {
        // 先从固定清单校验运行时身份和本机平台，再允许任何本地文件或网络操作。
        let entry = resolve_runtime_entry(&request, RequestedBackend::Auto)
            .map_err(PreparationFailure::from_manifest)
            .map_err(PreparationFailure::into_runtime_error)?;
        if let Some(executable) = explicit_runtime_executable(
            std::env::var_os("BURNCLOUD_LLAMA_SERVER").map(PathBuf::from),
        )
        .map_err(PreparationFailure::into_runtime_error)?
        {
            return prepare_explicit_runtime(&executable, &entry)
                .await
                .map_err(PreparationFailure::into_runtime_error);
        }

        let cache_root = runtime_cache_root().map_err(PreparationFailure::into_runtime_error)?;
        let (key, spec) =
            runtime_cache_identity(&entry).map_err(PreparationFailure::into_runtime_error)?;
        let registry = RUNTIME_INSTALLS
            .get_or_init(RuntimeInstallRegistry::default)
            .clone();
        let install_root = cache_root.clone();
        let install_entry = entry.clone();
        let install_spec = spec.clone();
        let mut subscription = registry
            .subscribe_or_start(
                key.clone(),
                move |job_key, cancellation, progress| async move {
                    prepare_managed_runtime(
                        &install_root,
                        &job_key,
                        &install_spec,
                        &install_entry,
                        cancellation,
                        progress,
                    )
                    .await
                    .map_err(|error| error.to_string())
                },
            )
            .await;

        let status = subscription.wait().await;
        match status {
            RuntimeInstallStatus::Ready(installed) => {
                let expected = key.cache_directory(&cache_root);
                if installed != expected {
                    return Err(PreparationFailure::new(
                        "runtime_cache_invalid",
                        "运行时共享作业返回了不匹配的缓存身份",
                    )
                    .into_runtime_error());
                }
                check_cached_runtime(&cache_root, &spec)
                    .map_err(PreparationFailure::from_cache)
                    .map_err(PreparationFailure::into_runtime_error)?
                    .ok_or_else(|| {
                        PreparationFailure::new(
                            "runtime_cache_invalid",
                            "运行时缓存提交后未能通过完整性检查",
                        )
                        .into_runtime_error()
                    })
            }
            RuntimeInstallStatus::Failed(message) => {
                Err(RuntimePrepareError::PrepareFailed(message))
            }
            RuntimeInstallStatus::Cancelled => {
                Err(PreparationFailure::new("cancelled", "运行时准备已取消").into_runtime_error())
            }
            RuntimeInstallStatus::Interrupted => Err(PreparationFailure::new(
                "runtime_install_interrupted",
                "运行时共享准备作业已中断",
            )
            .into_runtime_error()),
            RuntimeInstallStatus::Queued | RuntimeInstallStatus::Installing => {
                Err(PreparationFailure::new(
                    "runtime_install_interrupted",
                    "运行时共享准备作业未进入终态",
                )
                .into_runtime_error())
            }
        }
    }
}

#[derive(Debug)]
struct PreparationFailure {
    code: &'static str,
    message: &'static str,
}

impl PreparationFailure {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }

    fn from_manifest(error: RuntimeManifestError) -> Self {
        Self::new(error.code(), error.message())
    }

    fn from_cache(error: RuntimeCacheError) -> Self {
        match error {
            RuntimeCacheError::ConfigurationInvalid(_) => {
                Self::new("runtime_configuration_invalid", "运行时缓存配置无效")
            }
            RuntimeCacheError::CacheInvalid(_) => {
                Self::new("runtime_cache_invalid", "运行时缓存不可用")
            }
        }
    }

    fn from_lock(error: RuntimeInstallLockError) -> Self {
        match error {
            RuntimeInstallLockError::Cancelled => Self::new("cancelled", "运行时准备已取消"),
            RuntimeInstallLockError::CacheInvalid(_) => {
                Self::new("runtime_cache_invalid", "无法取得运行时缓存安装锁")
            }
        }
    }

    fn from_download(error: burncloud_download::RuntimeDownloadError) -> Self {
        let code = error.code();
        let message = match code {
            "runtime_source_unavailable" => "固定 Gitee 运行时资源不可用",
            "runtime_checksum_mismatch" => "运行时归档 SHA-256 校验失败",
            "runtime_size_mismatch" => "运行时归档长度校验失败",
            "runtime_manifest_invalid" => "固定运行时归档身份无效",
            "cancelled" => "运行时准备已取消",
            _ => "运行时归档下载失败",
        };
        Self::new(code, message)
    }

    fn from_archive(error: RuntimeArchiveError) -> Self {
        let code = error.code();
        let message = match code {
            "cancelled" => "运行时准备已取消",
            "runtime_archive_invalid" => "运行时归档不满足安全解压要求",
            _ => "运行时归档处理失败",
        };
        Self::new(code, message)
    }

    fn from_probe(error: RuntimeProbeError) -> Self {
        match error {
            RuntimeProbeError::InvalidArchive => {
                Self::new("runtime_archive_invalid", "运行时服务端路径无效")
            }
            RuntimeProbeError::Failed => {
                Self::new("runtime_probe_failed", "llama-server 运行时探测失败")
            }
            RuntimeProbeError::DeviceUnavailable => {
                Self::new("runtime_device_unavailable", "请求的 Metal 设备不可用")
            }
            RuntimeProbeError::UnsupportedBackend => {
                Self::new("unsupported_backend", "当前来源不支持请求的运行时 backend")
            }
            RuntimeProbeError::UnsupportedPlatform => Self::new(
                "unsupported_platform",
                "当前平台没有受支持的 llama.cpp 运行时",
            ),
            RuntimeProbeError::Cancelled => Self::new("cancelled", "运行时准备已取消"),
        }
    }

    fn into_runtime_error(self) -> RuntimePrepareError {
        RuntimePrepareError::PrepareFailed(format!("{}: {}", self.code, self.message))
    }
}

impl std::fmt::Display for PreparationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

fn explicit_runtime_executable(
    configured: Option<PathBuf>,
) -> Result<Option<PathBuf>, PreparationFailure> {
    let Some(executable) = configured else {
        return Ok(None);
    };
    if !executable.is_absolute() {
        return Err(PreparationFailure::new(
            "runtime_configuration_invalid",
            "BURNCLOUD_LLAMA_SERVER 必须是绝对路径",
        ));
    }
    let metadata = std::fs::symlink_metadata(&executable).map_err(|_| {
        PreparationFailure::new(
            "runtime_configuration_invalid",
            "BURNCLOUD_LLAMA_SERVER 指定的文件不可用",
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(PreparationFailure::new(
            "runtime_configuration_invalid",
            "BURNCLOUD_LLAMA_SERVER 必须指向普通文件",
        ));
    }
    Ok(Some(executable))
}

async fn prepare_explicit_runtime(
    executable: &Path,
    entry: &ResolvedRuntimeEntry,
) -> Result<PreparedRuntime, PreparationFailure> {
    let parent = executable.parent().ok_or_else(|| {
        PreparationFailure::new(
            "runtime_configuration_invalid",
            "BURNCLOUD_LLAMA_SERVER 缺少父目录",
        )
    })?;
    let filename = executable
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| {
            PreparationFailure::new(
                "runtime_configuration_invalid",
                "BURNCLOUD_LLAMA_SERVER 文件名无效",
            )
        })?;
    let probe_entry = RuntimeProbeEntry {
        target: entry.target.clone(),
        server_relative_path: filename.to_string(),
    };
    let candidate = probe_runtime(
        parent,
        &probe_entry,
        RequestedBackend::Auto,
        &CancellationToken::new(),
    )
    .await
    .map_err(|error| match error {
        RuntimeProbeError::DeviceUnavailable => PreparationFailure::from_probe(error),
        RuntimeProbeError::Cancelled => PreparationFailure::from_probe(error),
        RuntimeProbeError::UnsupportedBackend | RuntimeProbeError::UnsupportedPlatform => {
            PreparationFailure::from_probe(error)
        }
        RuntimeProbeError::InvalidArchive | RuntimeProbeError::Failed => PreparationFailure::new(
            "runtime_configuration_invalid",
            "BURNCLOUD_LLAMA_SERVER 未通过 llama-server 身份与依赖探测",
        ),
    })?;
    Ok(prepared_runtime(candidate))
}

fn runtime_cache_root() -> Result<PathBuf, PreparationFailure> {
    let executable = std::env::current_exe().map_err(|_| {
        PreparationFailure::new("runtime_cache_invalid", "无法定位 BurnCloud 安装目录")
    })?;
    let installation_root = executable.parent().ok_or_else(|| {
        PreparationFailure::new("runtime_cache_invalid", "无法确定 BurnCloud 安装目录")
    })?;
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .ok_or_else(|| {
            PreparationFailure::new("runtime_cache_invalid", "无法确定 BurnCloud 项目目录")
        })?;
    let target_dir = project_root.join("target");
    let override_value = std::env::var_os("BURNCLOUD_RUNTIME_CACHE_DIR");
    read_runtime_cache_root(installation_root, &target_dir, override_value.as_deref())
        .map_err(PreparationFailure::from_cache)
}

fn runtime_cache_identity(
    entry: &ResolvedRuntimeEntry,
) -> Result<(RuntimeCacheKey, RuntimeCacheSpec), PreparationFailure> {
    let key = RuntimeCacheKey::new(
        entry.version.clone(),
        entry.target.clone(),
        entry.package_kind.clone(),
        entry.archive.sha256(),
    )
    .map_err(PreparationFailure::from_cache)?;
    let spec = RuntimeCacheSpec::new(
        key.clone(),
        entry.archive.filename(),
        entry.archive.size_bytes(),
        entry.server_relative_path.clone(),
    )
    .map_err(PreparationFailure::from_cache)?;
    Ok((key, spec))
}

async fn prepare_managed_runtime(
    cache_root: &Path,
    key: &RuntimeCacheKey,
    spec: &RuntimeCacheSpec,
    entry: &ResolvedRuntimeEntry,
    cancellation_updates: watch::Receiver<bool>,
    reporter: cache::RuntimeInstallProgressReporter,
) -> Result<PathBuf, PreparationFailure> {
    let cancellation = CancellationToken::new();
    let bridge_token = cancellation.clone();
    let mut bridge_updates = cancellation_updates.clone();
    let bridge = tokio::spawn(async move {
        loop {
            if *bridge_updates.borrow() {
                bridge_token.cancel();
                break;
            }
            if bridge_updates.changed().await.is_err() {
                break;
            }
        }
    });
    let result = prepare_managed_runtime_inner(
        cache_root,
        key,
        spec,
        entry,
        &cancellation,
        cancellation_updates,
        reporter,
    )
    .await;
    bridge.abort();
    result
}

#[allow(
    clippy::too_many_arguments,
    reason = "安装流程需要显式携带缓存身份、可信清单和共享取消/进度句柄"
)]
async fn prepare_managed_runtime_inner(
    cache_root: &Path,
    key: &RuntimeCacheKey,
    spec: &RuntimeCacheSpec,
    entry: &ResolvedRuntimeEntry,
    cancellation: &CancellationToken,
    cancellation_updates: watch::Receiver<bool>,
    reporter: cache::RuntimeInstallProgressReporter,
) -> Result<PathBuf, PreparationFailure> {
    // 同一摘要的缓存读写和损坏隔离由跨进程锁串行化，锁内再次检查避免重复安装。
    let _install_lock = acquire_runtime_install_lock(cache_root, key, cancellation_updates.clone())
        .await
        .map_err(PreparationFailure::from_lock)?;
    if check_cached_runtime(cache_root, spec)
        .map_err(PreparationFailure::from_cache)?
        .is_some()
    {
        let extracted = key.cache_directory(cache_root).join("extracted");
        match probe_runtime(
            &extracted,
            &entry.probe_entry(),
            entry.requested_backend,
            cancellation,
        )
        .await
        {
            Ok(_) => return Ok(key.cache_directory(cache_root)),
            Err(error @ RuntimeProbeError::DeviceUnavailable)
            | Err(error @ RuntimeProbeError::UnsupportedBackend)
            | Err(error @ RuntimeProbeError::UnsupportedPlatform)
            | Err(error @ RuntimeProbeError::Cancelled) => {
                return Err(PreparationFailure::from_probe(error));
            }
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    "已通过安装锁隔离未通过探测的运行时缓存"
                );
                quarantine_cached_runtime(cache_root, spec)
                    .map_err(PreparationFailure::from_cache)?;
            }
        }
    }
    if cancellation.is_cancelled() || *cancellation_updates.borrow() {
        return Err(PreparationFailure::new("cancelled", "运行时准备已取消"));
    }

    let download_paths = runtime_download_paths(cache_root, key, entry.archive.filename())
        .map_err(PreparationFailure::from_cache)?;
    let download_dir = download_paths
        .partial_archive
        .parent()
        .ok_or_else(|| PreparationFailure::new("runtime_cache_invalid", "运行时下载目录无效"))?;
    let (download_progress, progress_receiver) = watch::channel(RuntimeDownloadProgress::default());
    let progress_reporter = reporter.clone();
    let progress_task = tokio::spawn(forward_download_progress(
        progress_receiver,
        progress_reporter,
    ));
    let download_result = download_runtime_archive(
        &entry.archive,
        download_dir,
        &download_progress,
        cancellation,
    )
    .await;
    drop(download_progress);
    progress_task.await.map_err(|_| {
        PreparationFailure::new("runtime_download_failed", "运行时下载进度作业异常终止")
    })?;
    let verified = download_result.map_err(PreparationFailure::from_download)?;

    if cancellation.is_cancelled() || *cancellation_updates.borrow() {
        return Err(PreparationFailure::new("cancelled", "运行时准备已取消"));
    }
    let staging = create_runtime_staging(cache_root, &Uuid::new_v4().to_string())
        .map_err(PreparationFailure::from_cache)?;
    let staged_archive = staging
        .path()
        .join("archive")
        .join(entry.archive.filename());
    tokio::fs::copy(verified.path(), &staged_archive)
        .await
        .map_err(|_| {
            PreparationFailure::new("runtime_cache_invalid", "无法保存已校验的运行时归档")
        })?;

    let limits = RuntimeArchiveLimits::from_manifest(
        entry.max_path_bytes,
        entry.max_entries,
        entry.max_file_bytes,
        entry.max_unpacked_bytes,
    )
    .map_err(PreparationFailure::from_archive)?;
    let extracted_root = extract_runtime_archive(
        &verified,
        &entry.target,
        &entry.server_relative_path,
        staging.path(),
        &limits,
        cancellation_updates.clone(),
    )
    .await
    .map_err(PreparationFailure::from_archive)?;
    probe_runtime(
        &extracted_root,
        &entry.probe_entry(),
        entry.requested_backend,
        cancellation,
    )
    .await
    .map_err(PreparationFailure::from_probe)?;
    if cancellation.is_cancelled() || *cancellation_updates.borrow() {
        return Err(PreparationFailure::new("cancelled", "运行时准备已取消"));
    }
    commit_runtime_staging(&staging, spec).map_err(PreparationFailure::from_cache)
}

async fn forward_download_progress(
    mut receiver: watch::Receiver<RuntimeDownloadProgress>,
    reporter: cache::RuntimeInstallProgressReporter,
) {
    while receiver.changed().await.is_ok() {
        let progress = *receiver.borrow_and_update();
        reporter.publish(RuntimeInstallProgress {
            phase: RuntimeInstallPhase::Downloading,
            bytes_received: progress.completed_bytes,
            total_bytes: Some(progress.total_bytes),
        });
    }
}

fn prepared_runtime(candidate: PreparedRuntimeCandidate) -> PreparedRuntime {
    PreparedRuntime {
        executable: candidate.executable.to_string_lossy().into_owned(),
    }
}

#[derive(Debug)]
pub(crate) struct LlamaCppRuntimeAdapter {
    model_id: String,
    // preset 必须保留到子进程读取完成，因此由 adapter 持有而不是局部临时删除。
    preset_file: Mutex<Option<NamedTempFile>>,
}

impl LlamaCppRuntimeAdapter {
    pub(crate) fn new(model_id: String) -> Self {
        Self {
            model_id,
            preset_file: Mutex::new(None),
        }
    }
}

#[async_trait]
impl RuntimeAdapter for LlamaCppRuntimeAdapter {
    async fn plan(
        &self,
        runtime: &PreparedRuntime,
        artifact: &PreparedArtifact,
    ) -> Result<ProcessPlan, RuntimeAdapterError> {
        // 适配器不能生成可执行的空程序计划。
        if runtime.executable.is_empty() {
            return Err(RuntimeAdapterError::PlanFailed(
                "runtime executable must not be empty".to_string(),
            ));
        }
        // 计划只接受上游已校验工件，避免把未验证路径交给真实模型进程。
        if !artifact.is_verified() {
            return Err(RuntimeAdapterError::PlanFailed(
                "prepared artifact has not been verified".to_string(),
            ));
        }
        // Verified 状态不保证路径仍存在，启动前再次确认文件系统对象可用。
        if !artifact.local_path().is_file() {
            return Err(RuntimeAdapterError::PlanFailed(format!(
                "prepared artifact is not a file: {}",
                artifact.local_path().display()
            )));
        }

        // 使用系统分配的空闲 loopback 端口，避免硬编码端口与其他服务冲突。
        let port = allocate_loopback_port().await?;
        let local_endpoint = format!("http://127.0.0.1:{port}");
        Ok(ProcessPlan {
            process: ProcessSpec {
                program: runtime.executable.clone(),
                // 直接使用已校验工件的本机路径，部署层不复制或重新定位模型文件。
                args: vec![
                    "-m".to_string(),
                    artifact.local_path().to_string_lossy().into_owned(),
                    "--host".to_string(),
                    "127.0.0.1".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                    "--alias".to_string(),
                    self.model_id.clone(),
                    "--parallel".to_string(),
                    configured_node_slots()?.to_string(),
                ],
            },
            // readiness 与本次实际监听端口绑定，避免计划和探测访问不同实例。
            readiness: ReadinessTarget {
                endpoint: format!("{local_endpoint}/health"),
            },
            local_endpoint,
        })
    }
}

#[async_trait]
impl ModelUrlRuntimeAdapter for LlamaCppRuntimeAdapter {
    async fn plan_model_url(
        &self,
        runtime: &PreparedRuntime,
        model_url: &str,
    ) -> Result<ProcessPlan, RuntimeAdapterError> {
        // 使用 URL 解析器校验 scheme/host 并安全编码后续的查询参数。
        let model_url = Url::parse(model_url)
            .map_err(|error| RuntimeAdapterError::PlanFailed(error.to_string()))?;
        if !matches!(model_url.scheme(), "http" | "https") || model_url.host_str().is_none() {
            return Err(RuntimeAdapterError::PlanFailed(
                "model URL must use HTTP or HTTPS".to_string(),
            ));
        }
        // preset 以 model_id 作为 section 标识；拒绝控制字符和括号以免改变其结构。
        if self.model_id.is_empty()
            || self
                .model_id
                .chars()
                .any(|character| matches!(character, '[' | ']' | '\r' | '\n'))
        {
            return Err(RuntimeAdapterError::PlanFailed(
                "model_id cannot be represented in a llama.cpp preset".to_string(),
            ));
        }

        // llama.cpp router 通过 preset 注册模型别名，并在服务启动时加载对应模型。
        let mut preset_file = NamedTempFile::new()
            .map_err(|error| RuntimeAdapterError::PlanFailed(error.to_string()))?;
        writeln!(preset_file, "[{}]", self.model_id)
            .and_then(|()| writeln!(preset_file, "model-url = {}", model_url))
            .and_then(|()| writeln!(preset_file, "load-on-startup = true"))
            .map_err(|error| RuntimeAdapterError::PlanFailed(error.to_string()))?;
        let preset_path = preset_file.path().to_string_lossy().into_owned();
        // trait 对象要求 adapter 可跨线程共享，互斥锁保护临时文件所有权与生命周期。
        *self
            .preset_file
            .lock()
            .map_err(|error| RuntimeAdapterError::PlanFailed(error.to_string()))? =
            Some(preset_file);

        // 每个部署使用独立 loopback 端口，使 readiness 和 endpoint 都指向本次进程。
        let port = allocate_loopback_port().await?;

        let endpoint = format!("http://127.0.0.1:{port}");
        // readiness 查询精确 model_id；HTTP 200 但模型未加载时不能算部署就绪。
        let mut readiness_url = Url::parse(&format!("{endpoint}/v1/models"))
            .map_err(|error| RuntimeAdapterError::PlanFailed(error.to_string()))?;
        readiness_url
            .query_pairs_mut()
            .append_pair("model_id", &self.model_id);
        Ok(ProcessPlan {
            process: ProcessSpec {
                program: runtime.executable.clone(),
                // 将 preset 和绑定地址传给 llama-server，adapter 生成的参数是启动唯一来源。
                args: vec![
                    "--models-preset".to_string(),
                    preset_path,
                    "--host".to_string(),
                    "127.0.0.1".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                    "--alias".to_string(),
                    self.model_id.clone(),
                    "--parallel".to_string(),
                    configured_node_slots()?.to_string(),
                ],
            },
            readiness: ReadinessTarget {
                endpoint: readiness_url.to_string(),
            },
            local_endpoint: endpoint,
        })
    }
}

fn configured_node_slots() -> Result<u32, RuntimeAdapterError> {
    match std::env::var("BURNCLOUD_NODE_MODEL_SLOTS") {
        Ok(raw) => parse_node_slots(&raw),
        Err(std::env::VarError::NotPresent) => Ok(1),
        Err(error) => Err(RuntimeAdapterError::PlanFailed(format!(
            "failed to read BURNCLOUD_NODE_MODEL_SLOTS: {error}"
        ))),
    }
}

fn parse_node_slots(raw: &str) -> Result<u32, RuntimeAdapterError> {
    if !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()) {
        if let Ok(slots @ 1..=u32::MAX) = raw.parse::<u32>() {
            return Ok(slots);
        }
    }
    Err(RuntimeAdapterError::PlanFailed(
        "BURNCLOUD_NODE_MODEL_SLOTS must be a positive integer".to_string(),
    ))
}

async fn allocate_loopback_port() -> Result<u16, RuntimeAdapterError> {
    // 让操作系统挑选端口，避免固定端口冲突并限定服务只绑定本机回环接口。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| RuntimeAdapterError::PlanFailed(error.to_string()))?;
    // 只把已分配端口写入计划；listener 随函数返回释放，子进程随后绑定该端口。
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| RuntimeAdapterError::PlanFailed(error.to_string()))
}

#[cfg(test)]
#[path = "node_runtime/tests.rs"]
mod tests;

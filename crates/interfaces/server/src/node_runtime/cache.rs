//! 运行时缓存身份、落盘布局、安装锁和共享作业基础设施。

use burncloud_node_runtime::PreparedRuntime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tempfile::NamedTempFile;
use tokio::{
    sync::{watch, Mutex as AsyncMutex},
    time::sleep,
};
use uuid::Uuid;

const CACHE_METADATA_SCHEMA: u32 = 1;
const MAX_METADATA_BYTES: u64 = 64 * 1024;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(150);
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(super) enum RuntimeCacheError {
    ConfigurationInvalid(String),
    CacheInvalid(String),
}

impl fmt::Display for RuntimeCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigurationInvalid(message) | Self::CacheInvalid(message) => {
                formatter.write_str(message)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct RuntimeCacheKey {
    version: String,
    target: String,
    package_kind: String,
    sha256: String,
}

impl RuntimeCacheKey {
    pub(super) fn new(
        version: impl Into<String>,
        target: impl Into<String>,
        package_kind: impl Into<String>,
        sha256: impl Into<String>,
    ) -> Result<Self, RuntimeCacheError> {
        let key = Self {
            version: version.into(),
            target: target.into(),
            package_kind: package_kind.into(),
            sha256: sha256.into(),
        };
        if !is_safe_path_segment(&key.version)
            || !is_safe_path_segment(&key.target)
            || !is_safe_path_segment(&key.package_kind)
            || !is_sha256(&key.sha256)
        {
            return Err(RuntimeCacheError::ConfigurationInvalid(
                "运行时缓存身份包含非法路径段或摘要".to_string(),
            ));
        }
        Ok(key)
    }

    pub(super) fn cache_directory(&self, cache_root: &Path) -> PathBuf {
        cache_root
            .join(&self.version)
            .join(&self.target)
            .join(&self.package_kind)
            .join(&self.sha256)
    }

    fn lock_filename(&self) -> String {
        format!(
            "{}-{}-{}-{}.lock",
            self.version, self.target, self.package_kind, self.sha256
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeCacheSpec {
    key: RuntimeCacheKey,
    archive_filename: String,
    archive_size_bytes: u64,
    server_relative_path: String,
}

impl RuntimeCacheSpec {
    pub(super) fn new(
        key: RuntimeCacheKey,
        archive_filename: impl Into<String>,
        archive_size_bytes: u64,
        server_relative_path: impl Into<String>,
    ) -> Result<Self, RuntimeCacheError> {
        let archive_filename = archive_filename.into();
        let server_relative_path = server_relative_path.into();
        if !is_safe_filename(&archive_filename)
            || archive_size_bytes == 0
            || !is_safe_relative_path(&server_relative_path)
        {
            return Err(RuntimeCacheError::ConfigurationInvalid(
                "运行时缓存清单包含非法文件名、大小或服务端相对路径".to_string(),
            ));
        }
        Ok(Self {
            key,
            archive_filename,
            archive_size_bytes,
            server_relative_path,
        })
    }

    fn metadata(&self) -> RuntimeCacheMetadata {
        RuntimeCacheMetadata {
            schema_version: CACHE_METADATA_SCHEMA,
            key: self.key.clone(),
            archive_filename: self.archive_filename.clone(),
            archive_size_bytes: self.archive_size_bytes,
            server_relative_path: self.server_relative_path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeCacheMetadata {
    schema_version: u32,
    key: RuntimeCacheKey,
    archive_filename: String,
    archive_size_bytes: u64,
    server_relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeDownloadPaths {
    pub(super) partial_archive: PathBuf,
    pub(super) sidecar: PathBuf,
}

fn runtime_cache_candidate(
    override_value: Option<&OsStr>,
    user_data_root: Option<&Path>,
) -> Result<PathBuf, RuntimeCacheError> {
    if let Some(value) = override_value {
        let path = PathBuf::from(value);
        if path.as_os_str().is_empty() || !path.is_absolute() {
            return Err(RuntimeCacheError::ConfigurationInvalid(
                "BURNCLOUD_RUNTIME_CACHE_DIR 必须是非空绝对路径".to_string(),
            ));
        }
        return Ok(path);
    }

    user_data_root
        .map(|path| path.join("BurnCloud").join("runtimes"))
        .ok_or_else(|| RuntimeCacheError::CacheInvalid("无法解析当前用户数据目录".to_string()))
}

pub(super) fn read_runtime_cache_root(
    installation_root: &Path,
    target_dir: &Path,
    override_value: Option<&OsStr>,
) -> Result<PathBuf, RuntimeCacheError> {
    // 先解析并校验最终规范路径，再创建目录，避免错误配置先在安装目录写入。
    let user_data_root = dirs::data_local_dir();
    let candidate = runtime_cache_candidate(override_value, user_data_root.as_deref())?;
    let resolved_candidate = resolve_path_with_missing_tail(&candidate).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法解析运行时缓存目录：{error}"))
    })?;
    let forbidden = forbidden_runtime_roots(installation_root, target_dir).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法验证运行时缓存边界：{error}"))
    })?;
    reject_cache_root_overlap(&resolved_candidate, &forbidden, override_value.is_some())?;

    fs::create_dir_all(&resolved_candidate).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法创建运行时缓存目录：{error}"))
    })?;
    let canonical_root = fs::canonicalize(&resolved_candidate).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法规范化运行时缓存目录：{error}"))
    })?;
    reject_cache_root_overlap(&canonical_root, &forbidden, override_value.is_some())?;

    let probe = NamedTempFile::new_in(&canonical_root).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("运行时缓存目录不可写：{error}"))
    })?;
    probe.as_file().sync_all().map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法同步运行时缓存目录探测文件：{error}"))
    })?;
    probe.close().map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法清理运行时缓存目录探测文件：{error}"))
    })?;
    Ok(canonical_root)
}

fn forbidden_runtime_roots(
    installation_root: &Path,
    target_dir: &Path,
) -> io::Result<Vec<PathBuf>> {
    let mut roots = vec![
        resolve_path_with_missing_tail(installation_root)?,
        resolve_path_with_missing_tail(target_dir)?,
    ];
    if let Some(project_root) = cargo_project_root(target_dir) {
        roots.push(resolve_path_with_missing_tail(&project_root)?);
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn cargo_project_root(target_dir: &Path) -> Option<PathBuf> {
    target_dir
        .ancestors()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name == OsStr::new("target"))
        })
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn reject_cache_root_overlap(
    cache_root: &Path,
    forbidden_roots: &[PathBuf],
    is_override: bool,
) -> Result<(), RuntimeCacheError> {
    if forbidden_roots
        .iter()
        .any(|forbidden| paths_overlap(cache_root, forbidden))
    {
        let message =
            "运行时缓存目录不能位于安装目录、项目目录或 target 目录中，也不能包含这些目录";
        return Err(if is_override {
            RuntimeCacheError::ConfigurationInvalid(message.to_string())
        } else {
            RuntimeCacheError::CacheInvalid(message.to_string())
        });
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn resolve_path_with_missing_tail(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let normalized = normalize_absolute_path(&absolute);
    let mut existing_prefix = normalized.clone();
    let mut missing_tail = Vec::<OsString>::new();

    loop {
        match fs::symlink_metadata(&existing_prefix) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = existing_prefix.file_name().ok_or(error)?;
                missing_tail.push(name.to_os_string());
                if !existing_prefix.pop() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "路径没有可解析的现存父目录",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }

    let mut resolved = fs::canonicalize(existing_prefix)?;
    for component in missing_tail.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

pub(super) fn runtime_download_paths(
    cache_root: &Path,
    key: &RuntimeCacheKey,
    filename: &str,
) -> Result<RuntimeDownloadPaths, RuntimeCacheError> {
    if !is_safe_filename(filename) {
        return Err(RuntimeCacheError::ConfigurationInvalid(
            "运行时归档文件名不安全".to_string(),
        ));
    }
    let directory = cache_root
        .join("downloads")
        .join(&key.version)
        .join(&key.target);
    let partial_archive = directory.join(format!("{filename}.part"));
    let sidecar = directory.join(format!("{filename}.part.json"));
    Ok(RuntimeDownloadPaths {
        partial_archive,
        sidecar,
    })
}

pub(super) fn quarantine_cached_runtime(
    cache_root: &Path,
    spec: &RuntimeCacheSpec,
) -> Result<(), RuntimeCacheError> {
    let cache_directory = spec.key.cache_directory(cache_root);
    match fs::symlink_metadata(&cache_directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeCacheError::CacheInvalid(format!(
            "无法检查待隔离的运行时缓存：{error}"
        ))),
        Ok(_) => quarantine_cache_entry(cache_root, &cache_directory, &spec.key),
    }
}

pub(super) fn check_cached_runtime(
    cache_root: &Path,
    spec: &RuntimeCacheSpec,
) -> Result<Option<PreparedRuntime>, RuntimeCacheError> {
    let cache_directory = spec.key.cache_directory(cache_root);
    match fs::symlink_metadata(&cache_directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(RuntimeCacheError::CacheInvalid(format!(
                "无法读取运行时缓存目录：{error}"
            )))
        }
        Ok(_) => {}
    }

    let checked = validate_cached_runtime(cache_root, &cache_directory, spec);
    match checked {
        Ok(executable) => Ok(Some(PreparedRuntime {
            executable: executable.to_string_lossy().into_owned(),
        })),
        Err(reason) => {
            quarantine_cache_entry(cache_root, &cache_directory, &spec.key)?;
            tracing::warn!(reason = %reason, "已隔离不完整或损坏的 llama.cpp 运行时缓存");
            Ok(None)
        }
    }
}

fn validate_cache_key_parent(cache_root: &Path, key: &RuntimeCacheKey) -> Result<PathBuf, String> {
    let canonical_root = fs::canonicalize(cache_root).map_err(|error| error.to_string())?;
    let mut current = canonical_root.clone();
    for segment in [&key.version, &key.target, &key.package_kind] {
        current.push(segment);
        ensure_real_directory(&current)?;
        let canonical = fs::canonicalize(&current).map_err(|error| error.to_string())?;
        if !canonical.starts_with(&canonical_root) {
            return Err("缓存 key 父路径越出缓存根目录".to_string());
        }
        current = canonical;
    }
    Ok(current)
}
fn validate_cached_runtime(
    cache_root: &Path,
    cache_directory: &Path,
    spec: &RuntimeCacheSpec,
) -> Result<PathBuf, String> {
    let root = fs::canonicalize(cache_root).map_err(|error| error.to_string())?;
    let expected_parent = validate_cache_key_parent(cache_root, &spec.key)?;
    let package_type = fs::symlink_metadata(cache_directory).map_err(|error| error.to_string())?;
    if !package_type.file_type().is_dir() {
        return Err("缓存 key 不是普通目录".to_string());
    }
    let canonical_cache = fs::canonicalize(cache_directory).map_err(|error| error.to_string())?;
    if !canonical_cache.starts_with(&root)
        || canonical_cache.parent() != Some(expected_parent.as_path())
    {
        return Err("缓存 key 路径越出缓存根目录或身份父路径异常".to_string());
    }

    // ready 只用于快速定位；下面仍逐项核对元数据、归档摘要与服务端路径。
    let ready_path = cache_directory.join("ready");
    ensure_plain_file(&ready_path)?;
    let ready_size = fs::metadata(&ready_path)
        .map_err(|error| error.to_string())?
        .len();
    let expected_ready = ready_marker(&spec.key);
    if ready_size != expected_ready.len() as u64 {
        return Err("ready 标记长度无效".to_string());
    }
    let ready = fs::read_to_string(&ready_path).map_err(|error| error.to_string())?;
    if ready != expected_ready {
        return Err("ready 标记与缓存身份不匹配".to_string());
    }

    let metadata_path = cache_directory.join("runtime.json");
    ensure_plain_file(&metadata_path)?;
    let metadata_size = fs::metadata(&metadata_path)
        .map_err(|error| error.to_string())?
        .len();
    if metadata_size > MAX_METADATA_BYTES {
        return Err("runtime.json 超过允许大小".to_string());
    }
    let metadata_bytes = fs::read(&metadata_path).map_err(|error| error.to_string())?;
    let metadata: RuntimeCacheMetadata =
        serde_json::from_slice(&metadata_bytes).map_err(|error| error.to_string())?;
    if metadata != spec.metadata() {
        return Err("runtime.json 身份与受信任 manifest 不匹配".to_string());
    }

    let archive_directory = cache_directory.join("archive");
    let extracted_directory = cache_directory.join("extracted");
    ensure_real_directory(&archive_directory)?;
    ensure_real_directory(&extracted_directory)?;
    let canonical_archive_directory =
        fs::canonicalize(&archive_directory).map_err(|error| error.to_string())?;
    let canonical_extracted_directory =
        fs::canonicalize(&extracted_directory).map_err(|error| error.to_string())?;
    if !canonical_archive_directory.starts_with(&canonical_cache)
        || !canonical_extracted_directory.starts_with(&canonical_cache)
    {
        return Err("归档或解压目录越出缓存 key 边界".to_string());
    }

    let archive_path = resolve_regular_file_under(&archive_directory, &spec.archive_filename)?;
    verify_archive_file(&archive_path, spec)?;

    let canonical_executable =
        resolve_regular_file_under(&extracted_directory, &spec.server_relative_path)?;
    if canonical_executable.to_str().is_none() {
        return Err("llama-server 路径无法表示为 UTF-8".to_string());
    }
    Ok(canonical_executable)
}

fn resolve_regular_file_under(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !is_safe_relative_path(relative) {
        return Err("缓存文件相对路径不安全".to_string());
    }
    ensure_real_directory(root)?;
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let mut path = root.to_path_buf();
    let mut components = Path::new(relative).components().peekable();
    while let Some(component) = components.next() {
        match component {
            Component::Normal(part) => path.push(part),
            _ => return Err("缓存文件相对路径包含非法组件".to_string()),
        }
        if components.peek().is_some() {
            ensure_real_directory(&path)?;
        } else {
            ensure_plain_file(&path)?;
        }
    }
    let canonical_path = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("缓存文件路径越出其所属目录".to_string());
    }
    Ok(canonical_path)
}

fn verify_archive_file(path: &Path, spec: &RuntimeCacheSpec) -> Result<(), String> {
    let archive_size = fs::metadata(path).map_err(|error| error.to_string())?.len();
    if archive_size != spec.archive_size_bytes {
        return Err("缓存归档长度与 manifest 不匹配".to_string());
    }
    let archive_sha256 = hash_file(path).map_err(|error| error.to_string())?;
    if archive_sha256 != spec.key.sha256 {
        return Err("缓存归档 SHA-256 与 manifest 不匹配".to_string());
    }
    Ok(())
}

fn validate_staging_payload(
    staging_path: &Path,
    spec: &RuntimeCacheSpec,
) -> Result<(), RuntimeCacheError> {
    let archive_directory = staging_path.join("archive");
    let extracted_directory = staging_path.join("extracted");
    ensure_real_directory(&archive_directory).map_err(RuntimeCacheError::CacheInvalid)?;
    ensure_real_directory(&extracted_directory).map_err(RuntimeCacheError::CacheInvalid)?;
    let archive_path = resolve_regular_file_under(&archive_directory, &spec.archive_filename)
        .map_err(RuntimeCacheError::CacheInvalid)?;
    verify_archive_file(&archive_path, spec).map_err(RuntimeCacheError::CacheInvalid)?;
    resolve_regular_file_under(&extracted_directory, &spec.server_relative_path)
        .map_err(RuntimeCacheError::CacheInvalid)?;
    Ok(())
}
fn ensure_plain_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() {
        return Err(format!("缓存文件不是普通文件：{}", path.display()));
    }
    Ok(())
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_dir() {
        return Err(format!("缓存目录不是普通目录：{}", path.display()));
    }
    Ok(())
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(encode_hex(&hasher.finalize()))
}

fn quarantine_cache_entry(
    cache_root: &Path,
    cache_directory: &Path,
    key: &RuntimeCacheKey,
) -> Result<(), RuntimeCacheError> {
    let expected_parent =
        validate_cache_key_parent(cache_root, key).map_err(RuntimeCacheError::CacheInvalid)?;
    let parent = cache_directory
        .parent()
        .ok_or_else(|| RuntimeCacheError::CacheInvalid("缓存 key 目录缺少父目录".to_string()))?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法验证损坏缓存父目录：{error}"))
    })?;
    if canonical_parent != expected_parent {
        return Err(RuntimeCacheError::CacheInvalid(
            "拒绝隔离位于缓存根目录之外的异常路径".to_string(),
        ));
    }
    let quarantine_root = ensure_safe_directory(cache_root, &[".quarantine"])?;
    let destination = quarantine_root.join(format!("{}-{}", key.sha256, Uuid::new_v4()));
    match fs::rename(cache_directory, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeCacheError::CacheInvalid(format!(
            "无法隔离损坏的运行时缓存：{error}"
        ))),
    }
}

fn ensure_safe_directory(root: &Path, relative: &[&str]) -> Result<PathBuf, RuntimeCacheError> {
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法解析运行时缓存根目录：{error}"))
    })?;
    let mut current = canonical_root.clone();
    for segment in relative {
        if !is_safe_path_segment(segment) {
            return Err(RuntimeCacheError::CacheInvalid(
                "运行时缓存内部目录包含非法路径段".to_string(),
            ));
        }
        current.push(segment);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(RuntimeCacheError::CacheInvalid(format!(
                    "运行时缓存内部路径不是普通目录：{}",
                    current.display()
                )))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|create_error| {
                    RuntimeCacheError::CacheInvalid(format!(
                        "无法创建运行时缓存内部目录：{create_error}"
                    ))
                })?;
            }
            Err(error) => {
                return Err(RuntimeCacheError::CacheInvalid(format!(
                    "无法读取运行时缓存内部目录：{error}"
                )))
            }
        }
        let canonical = fs::canonicalize(&current).map_err(|error| {
            RuntimeCacheError::CacheInvalid(format!("无法规范化运行时缓存内部目录：{error}"))
        })?;
        if !canonical.starts_with(&canonical_root) {
            return Err(RuntimeCacheError::CacheInvalid(
                "运行时缓存内部路径越出缓存根目录".to_string(),
            ));
        }
        current = canonical;
    }
    Ok(current)
}

fn ready_marker(key: &RuntimeCacheKey) -> String {
    format!("schema={CACHE_METADATA_SCHEMA}:sha256={}", key.sha256)
}

pub(super) struct RuntimeStagingGuard {
    cache_root: PathBuf,
    staging_root: PathBuf,
    path: PathBuf,
}

impl RuntimeStagingGuard {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

fn cleanup_staging_path(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_dir() {
        return fs::remove_dir_all(path);
    }
    if metadata.file_type().is_symlink() {
        return fs::remove_file(path);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "运行时 staging 路径已不是目录",
    ))
}

impl Drop for RuntimeStagingGuard {
    fn drop(&mut self) {
        if !fs::canonicalize(&self.staging_root).is_ok_and(|path| path == self.staging_root) {
            tracing::warn!("无法验证运行时 staging 父目录，跳过清理以避免越界删除");
            return;
        }
        if let Err(error) = cleanup_staging_path(&self.path) {
            tracing::warn!(error = %error, "清理未提交的运行时 staging 失败");
        }
    }
}
pub(super) fn create_runtime_staging(
    cache_root: &Path,
    job_id: &str,
) -> Result<RuntimeStagingGuard, RuntimeCacheError> {
    if !is_safe_path_segment(job_id) {
        return Err(RuntimeCacheError::CacheInvalid(
            "运行时共享作业标识不是安全路径段".to_string(),
        ));
    }
    let canonical_root = fs::canonicalize(cache_root).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法解析运行时缓存根目录：{error}"))
    })?;
    let staging_root = ensure_safe_directory(&canonical_root, &[".staging"])?;
    let path = staging_root.join(job_id);
    fs::create_dir(&path).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法创建运行时 staging：{error}"))
    })?;
    let guard = RuntimeStagingGuard {
        cache_root: canonical_root,
        staging_root,
        path,
    };
    for child in ["archive", "extracted"] {
        fs::create_dir(guard.path.join(child)).map_err(|error| {
            RuntimeCacheError::CacheInvalid(format!("无法创建 staging 子目录：{error}"))
        })?;
    }
    Ok(guard)
}

fn validate_staging_guard(staging: &RuntimeStagingGuard) -> Result<(), RuntimeCacheError> {
    let current_root = fs::canonicalize(&staging.cache_root).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法验证 staging 缓存根目录：{error}"))
    })?;
    let current_staging_root = fs::canonicalize(&staging.staging_root).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法验证 staging 父目录：{error}"))
    })?;
    let current_path = fs::canonicalize(&staging.path).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法验证 staging 目录：{error}"))
    })?;
    if current_root != staging.cache_root
        || current_staging_root != staging.staging_root
        || current_path.parent() != Some(staging.staging_root.as_path())
        || !current_path.starts_with(&current_root)
    {
        return Err(RuntimeCacheError::CacheInvalid(
            "staging 路径已变化或越出缓存根目录".to_string(),
        ));
    }
    ensure_real_directory(&current_path).map_err(RuntimeCacheError::CacheInvalid)
}
pub(super) fn commit_runtime_staging(
    staging: &RuntimeStagingGuard,
    spec: &RuntimeCacheSpec,
) -> Result<PathBuf, RuntimeCacheError> {
    validate_staging_guard(staging)?;
    validate_staging_payload(&staging.path, spec)?;
    // 元数据与 ready 写入 staging；只有完整目录准备好后才一次性原子改名。
    let metadata = spec.metadata();
    let metadata_bytes = serde_json::to_vec(&metadata).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法序列化运行时元数据：{error}"))
    })?;
    write_new_synced_file(&staging.path.join("runtime.json"), &metadata_bytes)?;
    write_new_synced_file(
        &staging.path.join("ready"),
        ready_marker(&spec.key).as_bytes(),
    )?;

    validate_staging_guard(staging)?;
    let parent = ensure_safe_directory(
        &staging.cache_root,
        &[
            spec.key.version.as_str(),
            spec.key.target.as_str(),
            spec.key.package_kind.as_str(),
        ],
    )?;
    let destination = parent.join(&spec.key.sha256);
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err(RuntimeCacheError::CacheInvalid(
                "运行时缓存 key 已存在，拒绝覆盖".to_string(),
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RuntimeCacheError::CacheInvalid(format!(
                "无法检查运行时缓存提交目标：{error}"
            )))
        }
    }
    fs::rename(&staging.path, &destination).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法原子提交运行时缓存：{error}"))
    })?;
    Ok(destination)
}

fn write_new_synced_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeCacheError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            RuntimeCacheError::CacheInvalid(format!("无法创建运行时元数据文件：{error}"))
        })?;
    file.write_all(contents).map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法写入运行时元数据文件：{error}"))
    })?;
    file.sync_all().map_err(|error| {
        RuntimeCacheError::CacheInvalid(format!("无法同步运行时元数据文件：{error}"))
    })
}

#[derive(Debug)]
pub(super) enum RuntimeInstallLockError {
    Cancelled,
    CacheInvalid(String),
}

impl fmt::Display for RuntimeInstallLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("运行时锁等待已取消"),
            Self::CacheInvalid(message) => formatter.write_str(message),
        }
    }
}

pub(super) struct RuntimeInstallLock {
    _file: File,
}

pub(super) async fn acquire_runtime_install_lock(
    cache_root: &Path,
    key: &RuntimeCacheKey,
    mut cancellation: watch::Receiver<bool>,
) -> Result<RuntimeInstallLock, RuntimeInstallLockError> {
    let lock_directory = ensure_safe_directory(cache_root, &[".locks"])
        .map_err(|error| RuntimeInstallLockError::CacheInvalid(error.to_string()))?;
    // 保留锁文件本身，避免并发删除后不同进程锁住同一路径的不同文件对象。
    let lock_path = lock_directory.join(key.lock_filename());
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| {
            RuntimeInstallLockError::CacheInvalid(format!("无法打开运行时安装锁：{error}"))
        })?;

    loop {
        if *cancellation.borrow() {
            return Err(RuntimeInstallLockError::Cancelled);
        }
        match file.try_lock() {
            Ok(()) => return Ok(RuntimeInstallLock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(RuntimeInstallLockError::CacheInvalid(format!(
                    "无法取得运行时安装锁：{error}"
                )))
            }
        }
        tokio::select! {
            _ = sleep(LOCK_RETRY_INTERVAL) => {}
            changed = cancellation.changed() => {
                if changed.is_err() || *cancellation.borrow() {
                    return Err(RuntimeInstallLockError::Cancelled);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeInstallPhase {
    Queued,
    Downloading,
}

impl RuntimeInstallPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuntimeInstallProgress {
    pub(super) phase: RuntimeInstallPhase,
    pub(super) bytes_received: u64,
    pub(super) total_bytes: Option<u64>,
}

impl Default for RuntimeInstallProgress {
    fn default() -> Self {
        Self {
            phase: RuntimeInstallPhase::Queued,
            bytes_received: 0,
            total_bytes: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RuntimeInstallStatus {
    Queued,
    Installing,
    Ready(PathBuf),
    Failed(String),
    Cancelled,
    Interrupted,
}

impl RuntimeInstallStatus {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Ready(_) | Self::Failed(_) | Self::Cancelled | Self::Interrupted
        )
    }
}

struct RuntimeInstallJob {
    #[allow(dead_code, reason = "任务 032 将共享作业 ID 暴露在运行时子进度中")]
    id: Uuid,
    key: RuntimeCacheKey,
    status: watch::Sender<RuntimeInstallStatus>,
    progress: watch::Sender<RuntimeInstallProgress>,
    cancellation: watch::Sender<bool>,
    subscriber_count: Mutex<usize>,
}

impl RuntimeInstallJob {
    fn new(key: RuntimeCacheKey) -> Self {
        let (status, _) = watch::channel(RuntimeInstallStatus::Queued);
        let (progress, _) = watch::channel(RuntimeInstallProgress::default());
        let (cancellation, _) = watch::channel(false);
        Self {
            id: Uuid::new_v4(),
            key,
            status,
            progress,
            cancellation,
            subscriber_count: Mutex::new(0),
        }
    }

    fn subscribe(self: &Arc<Self>) -> RuntimeInstallSubscription {
        let mut count = self
            .subscriber_count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *count += 1;
        drop(count);
        RuntimeInstallSubscription {
            job: Arc::clone(self),
            status: self.status.subscribe(),
            progress: self.progress.subscribe(),
        }
    }

    fn try_subscribe(self: &Arc<Self>) -> Option<RuntimeInstallSubscription> {
        let mut count = self
            .subscriber_count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *self.cancellation.borrow() {
            return None;
        }
        *count += 1;
        drop(count);
        Some(RuntimeInstallSubscription {
            job: Arc::clone(self),
            status: self.status.subscribe(),
            progress: self.progress.subscribe(),
        })
    }

    fn cancellation_receiver(&self) -> watch::Receiver<bool> {
        self.cancellation.subscribe()
    }

    fn progress_reporter(&self) -> RuntimeInstallProgressReporter {
        RuntimeInstallProgressReporter {
            sender: self.progress.clone(),
        }
    }

    fn publish_status(&self, status: RuntimeInstallStatus) {
        self.status.send_replace(status);
    }
}

pub(super) struct RuntimeInstallSubscription {
    job: Arc<RuntimeInstallJob>,
    status: watch::Receiver<RuntimeInstallStatus>,
    #[allow(dead_code, reason = "任务 032 将共享作业进度转为父部署任务快照")]
    progress: watch::Receiver<RuntimeInstallProgress>,
}

#[allow(dead_code, reason = "任务 032 将共享作业身份和进度接入父部署任务")]
impl RuntimeInstallSubscription {
    pub(super) fn job_id(&self) -> Uuid {
        self.job.id
    }

    pub(super) fn key(&self) -> &RuntimeCacheKey {
        &self.job.key
    }

    pub(super) fn status(&self) -> RuntimeInstallStatus {
        self.status.borrow().clone()
    }

    pub(super) fn progress(&self) -> RuntimeInstallProgress {
        *self.progress.borrow()
    }

    pub(super) async fn wait(&mut self) -> RuntimeInstallStatus {
        loop {
            let current = self.status();
            if current.is_terminal() {
                return current;
            }
            if self.status.changed().await.is_err() {
                return RuntimeInstallStatus::Interrupted;
            }
        }
    }
}

impl RuntimeInstallJob {
    fn release_subscriber(&self) {
        let mut count = self
            .subscriber_count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *count == 0 {
            tracing::error!("运行时共享作业订阅计数不变量被破坏");
            return;
        }
        *count -= 1;
        if *count == 0 && !self.status.borrow().is_terminal() {
            self.cancellation.send_replace(true);
        }
    }
}

impl Drop for RuntimeInstallSubscription {
    fn drop(&mut self) {
        self.job.release_subscriber();
    }
}

#[derive(Clone)]
pub(super) struct RuntimeInstallProgressReporter {
    sender: watch::Sender<RuntimeInstallProgress>,
}

impl RuntimeInstallProgressReporter {
    pub(super) fn publish(&self, progress: RuntimeInstallProgress) {
        tracing::trace!(
            phase = progress.phase.label(),
            bytes_received = progress.bytes_received,
            total_bytes = progress.total_bytes,
            "运行时共享作业进度更新"
        );
        self.sender.send_replace(progress);
    }
}

#[derive(Default, Clone)]
pub(super) struct RuntimeInstallRegistry {
    jobs: Arc<AsyncMutex<HashMap<RuntimeCacheKey, Weak<RuntimeInstallJob>>>>,
}

impl RuntimeInstallRegistry {
    pub(super) async fn subscribe_or_start<F, Fut>(
        &self,
        key: RuntimeCacheKey,
        install: F,
    ) -> RuntimeInstallSubscription
    where
        F: FnOnce(RuntimeCacheKey, watch::Receiver<bool>, RuntimeInstallProgressReporter) -> Fut
            + Send
            + 'static,
        Fut: std::future::Future<Output = Result<PathBuf, String>> + Send + 'static,
    {
        let (job, subscription) = {
            let mut jobs = self.jobs.lock().await;
            // 注册表只持有弱引用，作业生命周期由 worker 与各父任务订阅共同决定。
            jobs.retain(|_, job| job.strong_count() > 0);
            if let Some(existing) = jobs.get(&key).and_then(Weak::upgrade) {
                if let Some(subscription) = existing.try_subscribe() {
                    return subscription;
                }
                jobs.remove(&key);
            }

            let job = Arc::new(RuntimeInstallJob::new(key.clone()));
            let subscription = job.subscribe();
            jobs.insert(key.clone(), Arc::downgrade(&job));
            (job, subscription)
        };

        let worker_job = Arc::clone(&job);
        tokio::spawn(async move {
            worker_job.publish_status(RuntimeInstallStatus::Installing);
            let result = install(
                worker_job.key.clone(),
                worker_job.cancellation_receiver(),
                worker_job.progress_reporter(),
            )
            .await;
            let status = match result {
                Ok(path) => RuntimeInstallStatus::Ready(path),
                Err(error) if *worker_job.cancellation.borrow() => {
                    tracing::debug!(error = %error, "最后一个运行时订阅已取消共享准备作业");
                    RuntimeInstallStatus::Cancelled
                }
                Err(error) => RuntimeInstallStatus::Failed(error),
            };
            worker_job.publish_status(status);
        });
        subscription
    }
}

fn is_safe_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_safe_filename(value: &str) -> bool {
    is_safe_path_segment(value) && !value.starts_with('.')
}

fn is_safe_relative_path(value: &str) -> bool {
    if value.is_empty() || value.contains('\0') || value.contains('\\') {
        return false;
    }
    let path = Path::new(value);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;

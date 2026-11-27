//! 运行时归档安全解压实现。

use burncloud_download::VerifiedArchive;
use flate2::read::GzDecoder;
use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};
use tokio::sync::watch;
use zip::ZipArchive;

const DEFAULT_MAX_PATH_BYTES: u64 = 4096;
const DEFAULT_MAX_ENTRIES: u64 = 100_000;
const DEFAULT_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuntimeArchiveLimits {
    max_path_bytes: u64,
    max_entries: u64,
    max_file_bytes: u64,
    max_unpacked_bytes: u64,
}

impl Default for RuntimeArchiveLimits {
    fn default() -> Self {
        Self {
            max_path_bytes: DEFAULT_MAX_PATH_BYTES,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_unpacked_bytes: DEFAULT_MAX_UNPACKED_BYTES,
        }
    }
}

impl RuntimeArchiveLimits {
    pub(super) fn from_manifest(
        max_path_bytes: u64,
        max_entries: u64,
        max_file_bytes: u64,
        max_unpacked_bytes: u64,
    ) -> Result<Self, RuntimeArchiveError> {
        if max_path_bytes == 0
            || max_path_bytes > DEFAULT_MAX_PATH_BYTES
            || max_entries == 0
            || max_entries > DEFAULT_MAX_ENTRIES
            || max_file_bytes == 0
            || max_file_bytes > DEFAULT_MAX_FILE_BYTES
            || max_unpacked_bytes == 0
            || max_unpacked_bytes > DEFAULT_MAX_UNPACKED_BYTES
        {
            return Err(RuntimeArchiveError::Invalid(
                "manifest 运行时归档限制超出安全上限".to_string(),
            ));
        }
        Ok(Self {
            max_path_bytes,
            max_entries,
            max_file_bytes,
            max_unpacked_bytes,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RuntimeArchiveError {
    Invalid(String),
    Cancelled,
    Io(String),
    Worker(String),
}

impl RuntimeArchiveError {
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "runtime_archive_invalid",
            Self::Cancelled => "cancelled",
            Self::Io(_) | Self::Worker(_) => "runtime_archive_io",
        }
    }
}

impl fmt::Display for RuntimeArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) | Self::Io(message) | Self::Worker(message) => {
                formatter.write_str(message)
            }
            Self::Cancelled => formatter.write_str("运行时归档解压已取消"),
        }
    }
}

impl Error for RuntimeArchiveError {}

#[derive(Debug)]
struct ExtractedRuntime {
    extracted_root: PathBuf,
    entry_count: u64,
    unpacked_bytes: u64,
}

#[derive(Default)]
struct ArchiveStatistics {
    entry_count: u64,
    unpacked_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeArchiveFormat {
    Zip,
    TarGz,
}

impl RuntimeArchiveFormat {
    fn for_target(target: &str) -> Result<Self, RuntimeArchiveError> {
        match target {
            "x86_64-pc-windows-msvc" => Ok(Self::Zip),
            "x86_64-apple-darwin" | "aarch64-apple-darwin" | "x86_64-unknown-linux-gnu" => {
                Ok(Self::TarGz)
            }
            _ => Err(RuntimeArchiveError::Invalid(format!(
                "运行时归档 target 不受支持：{target}"
            ))),
        }
    }

    fn matches_path(self, path: &Path) -> bool {
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        match self {
            Self::Zip => filename.ends_with(".zip"),
            Self::TarGz => filename.ends_with(".tar.gz"),
        }
    }
}

struct ExtractedDirectoryGuard {
    path: PathBuf,
    keep: bool,
}

fn cleanup_extracted_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path),
        Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "运行时解压路径类型已变化",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl Drop for ExtractedDirectoryGuard {
    fn drop(&mut self) {
        if !self.keep {
            if let Err(error) = cleanup_extracted_directory(&self.path) {
                tracing::warn!(path = %self.path.display(), error = %error, "清理失败的运行时解压目录失败");
            }
        }
    }
}

/// 在阻塞线程中逐项解压，避免大型归档的读写阻塞异步执行器。
pub(super) async fn extract_runtime_archive(
    archive: &VerifiedArchive,
    target: &str,
    server_relative_path: &str,
    staging_root: &Path,
    limits: &RuntimeArchiveLimits,
    cancellation: watch::Receiver<bool>,
) -> Result<PathBuf, RuntimeArchiveError> {
    let archive_path = archive.path().to_path_buf();
    let expected_size = archive.size_bytes();
    let target = target.to_owned();
    let server_relative_path = server_relative_path.to_owned();
    let staging_root = staging_root.to_path_buf();
    let limits = *limits;
    let extracted = tokio::task::spawn_blocking(move || {
        extract_archive_file(
            &archive_path,
            expected_size,
            &target,
            &server_relative_path,
            &staging_root,
            &limits,
            &cancellation,
        )
    })
    .await
    .map_err(|error| RuntimeArchiveError::Worker(format!("运行时解压线程异常：{error}")))??;
    tracing::debug!(
        entries = extracted.entry_count,
        unpacked_bytes = extracted.unpacked_bytes,
        path = %extracted.extracted_root.display(),
        "运行时归档已安全解压"
    );
    Ok(extracted.extracted_root)
}
#[allow(
    clippy::too_many_arguments,
    reason = "归档解压需要显式携带清单身份、staging、资源限制和取消信号"
)]
fn extract_archive_file(
    archive_path: &Path,
    expected_size: u64,
    target: &str,
    server_relative_path: &str,
    staging_root: &Path,
    limits: &RuntimeArchiveLimits,
    cancellation: &watch::Receiver<bool>,
) -> Result<ExtractedRuntime, RuntimeArchiveError> {
    let (extracted_root, mut guard) = prepare_extracted_directory(staging_root)?;
    check_cancelled(cancellation)?;
    let metadata = fs::symlink_metadata(archive_path)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法读取已校验运行时归档：{error}")))?;
    if !metadata.file_type().is_file() || metadata.len() != expected_size {
        return Err(RuntimeArchiveError::Invalid(
            "已校验运行时归档的文件类型或大小已变化".to_string(),
        ));
    }
    let format = RuntimeArchiveFormat::for_target(target)?;
    if !format.matches_path(archive_path) {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档格式与 manifest target 不匹配".to_string(),
        ));
    }

    let server_relative =
        validate_archive_entry_path(server_relative_path, false, &extracted_root, limits)?;
    let mut statistics = ArchiveStatistics::default();
    match format {
        RuntimeArchiveFormat::Zip => extract_zip(
            archive_path,
            &extracted_root,
            limits,
            cancellation,
            &mut statistics,
        )?,
        RuntimeArchiveFormat::TarGz => extract_tar_gz(
            archive_path,
            &extracted_root,
            limits,
            cancellation,
            &mut statistics,
        )?,
    }
    check_cancelled(cancellation)?;
    validate_server_file(&extracted_root, &server_relative)?;
    guard.keep = true;
    Ok(ExtractedRuntime {
        extracted_root,
        entry_count: statistics.entry_count,
        unpacked_bytes: statistics.unpacked_bytes,
    })
}

fn prepare_extracted_directory(
    staging_root: &Path,
) -> Result<(PathBuf, ExtractedDirectoryGuard), RuntimeArchiveError> {
    let metadata = fs::symlink_metadata(staging_root)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法读取运行时 staging：{error}")))?;
    if !metadata.file_type().is_dir() {
        return Err(RuntimeArchiveError::Invalid(
            "运行时 staging 必须是普通目录".to_string(),
        ));
    }
    let staging_root = fs::canonicalize(staging_root)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法规范运行时 staging：{error}")))?;
    let extracted_path = staging_root.join("extracted");
    match fs::symlink_metadata(&extracted_path) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(RuntimeArchiveError::Invalid(
                "运行时 staging/extracted 不是普通目录".to_string(),
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&extracted_path).map_err(|error| {
                RuntimeArchiveError::Io(format!("无法创建运行时解压目录：{error}"))
            })?;
        }
        Err(error) => {
            return Err(RuntimeArchiveError::Io(format!(
                "无法检查运行时解压目录：{error}"
            )));
        }
    }
    let canonical_extracted = fs::canonicalize(&extracted_path)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法规范运行时解压目录：{error}")))?;
    if canonical_extracted.parent() != Some(staging_root.as_path()) {
        return Err(RuntimeArchiveError::Invalid(
            "运行时解压目录越出 staging".to_string(),
        ));
    }
    let mut entries = fs::read_dir(&canonical_extracted)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法检查运行时解压目录：{error}")))?;
    match entries.next() {
        Some(Ok(_)) => {
            return Err(RuntimeArchiveError::Invalid(
                "运行时解压目录必须为空".to_string(),
            ));
        }
        Some(Err(error)) => {
            return Err(RuntimeArchiveError::Io(format!(
                "无法读取运行时解压目录：{error}"
            )));
        }
        None => {}
    }
    let guard = ExtractedDirectoryGuard {
        path: canonical_extracted.clone(),
        keep: false,
    };
    Ok((canonical_extracted, guard))
}

fn extract_zip(
    archive_path: &Path,
    extracted_root: &Path,
    limits: &RuntimeArchiveLimits,
    cancellation: &watch::Receiver<bool>,
    statistics: &mut ArchiveStatistics,
) -> Result<(), RuntimeArchiveError> {
    let file = File::open(archive_path)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法打开运行时 ZIP：{error}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| RuntimeArchiveError::Invalid(format!("运行时 ZIP 无效：{error}")))?;
    let count = u64::try_from(archive.len())
        .map_err(|_| RuntimeArchiveError::Invalid("运行时 ZIP entry 数量溢出".to_string()))?;
    if count > limits.max_entries {
        return Err(RuntimeArchiveError::Invalid(
            "运行时 ZIP entry 数量超过限制".to_string(),
        ));
    }
    for index in 0..archive.len() {
        check_cancelled(cancellation)?;
        let mut entry = archive.by_index(index).map_err(|error| {
            RuntimeArchiveError::Invalid(format!("无法读取运行时 ZIP entry：{error}"))
        })?;
        let name = std::str::from_utf8(entry.name_raw()).map_err(|error| {
            RuntimeArchiveError::Invalid(format!("运行时 ZIP 路径不是 UTF-8：{error}"))
        })?;
        let is_directory = entry.is_dir() || name.ends_with('/') || name.ends_with('\\');
        let mode = entry.unix_mode();
        validate_zip_entry_kind(is_directory, entry.is_symlink(), mode)?;
        let destination = validate_archive_entry_path(name, is_directory, extracted_root, limits)?;
        record_entry(statistics, limits)?;
        let size = entry.size();
        if is_directory {
            if size != 0 {
                return Err(RuntimeArchiveError::Invalid(
                    "运行时 ZIP 目录 entry 不应包含文件内容".to_string(),
                ));
            }
            create_directory_chain(extracted_root, &destination)?;
        } else {
            write_entry_with_limits(
                &mut entry,
                extracted_root,
                &destination,
                size,
                mode.unwrap_or(0o644),
                limits,
                cancellation,
                statistics,
            )?;
        }
    }
    Ok(())
}

fn validate_zip_entry_kind(
    is_directory: bool,
    is_symlink: bool,
    unix_mode: Option<u32>,
) -> Result<(), RuntimeArchiveError> {
    const TYPE_MASK: u32 = 0o170000;
    const DIRECTORY_TYPE: u32 = 0o040000;
    const REGULAR_TYPE: u32 = 0o100000;
    if is_symlink {
        return Err(RuntimeArchiveError::Invalid(
            "运行时 ZIP 不允许符号链接 entry".to_string(),
        ));
    }
    let entry_type = unix_mode.map(|mode| mode & TYPE_MASK).unwrap_or_default();
    let valid = matches!(
        (is_directory, entry_type),
        (true, 0 | DIRECTORY_TYPE) | (false, 0 | REGULAR_TYPE)
    );
    if valid {
        Ok(())
    } else {
        Err(RuntimeArchiveError::Invalid(
            "运行时 ZIP 包含非普通文件或目录 entry".to_string(),
        ))
    }
}
fn extract_tar_gz(
    archive_path: &Path,
    extracted_root: &Path,
    limits: &RuntimeArchiveLimits,
    cancellation: &watch::Receiver<bool>,
    statistics: &mut ArchiveStatistics,
) -> Result<(), RuntimeArchiveError> {
    let file = File::open(archive_path)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法打开运行时 TAR.GZ：{error}")))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| RuntimeArchiveError::Invalid(format!("运行时 TAR.GZ 无效：{error}")))?
        .raw(true);
    for entry in entries {
        check_cancelled(cancellation)?;
        let mut entry = entry.map_err(|error| {
            RuntimeArchiveError::Invalid(format!("无法读取运行时 TAR.GZ entry：{error}"))
        })?;
        record_entry(statistics, limits)?;
        let entry_type = entry.header().entry_type();
        let is_directory = entry_type.is_dir();
        if !is_directory && !entry_type.is_file() {
            return Err(RuntimeArchiveError::Invalid(
                "运行时 TAR.GZ 包含链接或特殊 entry".to_string(),
            ));
        }
        let raw_path = entry.path_bytes();
        let name = std::str::from_utf8(raw_path.as_ref()).map_err(|error| {
            RuntimeArchiveError::Invalid(format!("运行时 TAR.GZ 路径不是 UTF-8：{error}"))
        })?;
        let destination = validate_archive_entry_path(name, is_directory, extracted_root, limits)?;
        let size = entry.size();
        if is_directory {
            if size != 0 {
                return Err(RuntimeArchiveError::Invalid(
                    "运行时 TAR.GZ 目录 entry 不应包含文件内容".to_string(),
                ));
            }
            create_directory_chain(extracted_root, &destination)?;
        } else {
            let mode = entry.header().mode().map_err(|error| {
                RuntimeArchiveError::Invalid(format!("运行时 TAR.GZ 文件权限无效：{error}"))
            })?;
            write_entry_with_limits(
                &mut entry,
                extracted_root,
                &destination,
                size,
                mode,
                limits,
                cancellation,
                statistics,
            )?;
        }
    }
    Ok(())
}

fn validate_archive_entry_path(
    entry_name: &str,
    is_directory: bool,
    extracted_root: &Path,
    limits: &RuntimeArchiveLimits,
) -> Result<PathBuf, RuntimeArchiveError> {
    let path_bytes = u64::try_from(entry_name.len())
        .map_err(|_| RuntimeArchiveError::Invalid("运行时归档路径长度无法表示".to_string()))?;
    if entry_name.is_empty() || path_bytes > limits.max_path_bytes || entry_name.contains('\0') {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档 entry 路径为空、过长或包含空字节".to_string(),
        ));
    }
    let normalized = entry_name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.as_bytes().get(1) == Some(&b':') {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档 entry 不允许绝对、UNC 或驱动器路径".to_string(),
        ));
    }
    let relative = if is_directory && normalized.ends_with('/') {
        &normalized[..normalized.len() - 1]
    } else {
        normalized.as_str()
    };
    if relative.is_empty() || (!is_directory && normalized.ends_with('/')) {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档 entry 路径格式无效".to_string(),
        ));
    }
    let mut destination = extracted_root.to_path_buf();
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains(':') {
            return Err(RuntimeArchiveError::Invalid(
                "运行时归档 entry 包含非法路径段".to_string(),
            ));
        }
        destination.push(segment);
    }
    if !destination.starts_with(extracted_root) {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档 entry 越出 staging 根目录".to_string(),
        ));
    }
    Ok(destination)
}

fn record_entry(
    statistics: &mut ArchiveStatistics,
    limits: &RuntimeArchiveLimits,
) -> Result<(), RuntimeArchiveError> {
    let count = statistics
        .entry_count
        .checked_add(1)
        .ok_or_else(|| RuntimeArchiveError::Invalid("运行时归档 entry 数量溢出".to_string()))?;
    if count > limits.max_entries {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档 entry 数量超过限制".to_string(),
        ));
    }
    statistics.entry_count = count;
    Ok(())
}

fn create_directory_chain(
    extracted_root: &Path,
    destination: &Path,
) -> Result<(), RuntimeArchiveError> {
    let relative = destination.strip_prefix(extracted_root).map_err(|_| {
        RuntimeArchiveError::Invalid("运行时归档目标越出 staging 根目录".to_string())
    })?;
    let mut current = extracted_root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(name) => current.push(name),
            _ => {
                return Err(RuntimeArchiveError::Invalid(
                    "运行时归档目标路径包含非普通组件".to_string(),
                ))
            }
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(RuntimeArchiveError::Invalid(
                    "运行时归档目标路径与现有非目录对象冲突".to_string(),
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let metadata = fs::symlink_metadata(&current).map_err(|error| {
                            RuntimeArchiveError::Io(format!("无法复查运行时解压目录：{error}"))
                        })?;
                        if !metadata.file_type().is_dir() {
                            return Err(RuntimeArchiveError::Invalid(
                                "运行时归档目标路径与现有非目录对象冲突".to_string(),
                            ));
                        }
                    }
                    Err(error) => {
                        return Err(RuntimeArchiveError::Io(format!(
                            "无法创建运行时解压目录：{error}"
                        )));
                    }
                }
            }
            Err(error) => {
                return Err(RuntimeArchiveError::Io(format!(
                    "无法检查运行时解压目录：{error}"
                )));
            }
        }
    }
    Ok(())
}
#[allow(
    clippy::too_many_arguments,
    reason = "逐文件写入需比较声明大小、资源限额、取消状态与累计统计"
)]
fn write_entry_with_limits(
    reader: &mut impl Read,
    extracted_root: &Path,
    destination: &Path,
    declared_size: u64,
    mode: u32,
    limits: &RuntimeArchiveLimits,
    cancellation: &watch::Receiver<bool>,
    statistics: &mut ArchiveStatistics,
) -> Result<(), RuntimeArchiveError> {
    if declared_size > limits.max_file_bytes {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档单文件展开大小超过限制".to_string(),
        ));
    }
    let total_after_entry = statistics
        .unpacked_bytes
        .checked_add(declared_size)
        .ok_or_else(|| RuntimeArchiveError::Invalid("运行时归档总展开大小溢出".to_string()))?;
    if total_after_entry > limits.max_unpacked_bytes {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档总展开大小超过限制".to_string(),
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| RuntimeArchiveError::Invalid("运行时归档文件目标缺少父目录".to_string()))?;
    create_directory_chain(extracted_root, parent)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                RuntimeArchiveError::Invalid("运行时归档包含重复或冲突的文件路径".to_string())
            } else {
                RuntimeArchiveError::Io(format!("无法创建解压后的运行时文件：{error}"))
            }
        })?;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut written = 0u64;
    loop {
        check_cancelled(cancellation)?;
        let read = reader.read(&mut buffer).map_err(|error| {
            RuntimeArchiveError::Invalid(format!("运行时归档文件数据损坏：{error}"))
        })?;
        if read == 0 {
            break;
        }
        let chunk = u64::try_from(read)
            .map_err(|_| RuntimeArchiveError::Invalid("归档数据长度无法表示".to_string()))?;
        written = written.checked_add(chunk).ok_or_else(|| {
            RuntimeArchiveError::Invalid("运行时归档单文件展开大小溢出".to_string())
        })?;
        if written > declared_size
            || written > limits.max_file_bytes
            || statistics
                .unpacked_bytes
                .checked_add(written)
                .is_none_or(|total| total > limits.max_unpacked_bytes)
        {
            return Err(RuntimeArchiveError::Invalid(
                "运行时归档实际展开大小超过声明或资源限制".to_string(),
            ));
        }
        output.write_all(&buffer[..read]).map_err(|error| {
            RuntimeArchiveError::Io(format!("写入解压后的运行时文件失败：{error}"))
        })?;
    }
    if written != declared_size {
        return Err(RuntimeArchiveError::Invalid(
            "运行时归档文件实际大小与声明不一致".to_string(),
        ));
    }
    apply_file_mode(&output, mode)
        .map_err(|error| RuntimeArchiveError::Io(format!("设置运行时文件权限失败：{error}")))?;
    statistics.unpacked_bytes = total_after_entry;
    Ok(())
}

#[cfg(unix)]
fn apply_file_mode(file: &File, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // 当前用户始终可读写，执行位只授予当前用户；不保留归档中的组/其他写权限和特殊位。
    let mut safe_mode = 0o600 | (mode & 0o044);
    if mode & 0o111 != 0 {
        safe_mode |= 0o100;
    }
    file.set_permissions(fs::Permissions::from_mode(safe_mode))
}

#[cfg(not(unix))]
fn apply_file_mode(_file: &File, _mode: u32) -> io::Result<()> {
    Ok(())
}

fn validate_server_file(
    extracted_root: &Path,
    server_path: &Path,
) -> Result<(), RuntimeArchiveError> {
    let relative = server_path.strip_prefix(extracted_root).map_err(|_| {
        RuntimeArchiveError::Invalid("manifest 服务端路径越出运行时解压目录".to_string())
    })?;
    let components: Vec<_> = relative.components().collect();
    let mut current = extracted_root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Normal(name) => current.push(name),
            _ => {
                return Err(RuntimeArchiveError::Invalid(
                    "manifest 服务端路径包含非法组件".to_string(),
                ))
            }
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            RuntimeArchiveError::Invalid(format!("manifest 指定的 llama-server 不存在：{error}"))
        })?;
        let is_last = index + 1 == components.len();
        if is_last && !metadata.file_type().is_file() {
            return Err(RuntimeArchiveError::Invalid(
                "manifest 指定的 llama-server 不是普通文件".to_string(),
            ));
        }
        if !is_last && !metadata.file_type().is_dir() {
            return Err(RuntimeArchiveError::Invalid(
                "manifest 服务端路径的父级不是普通目录".to_string(),
            ));
        }
    }
    let canonical_root = fs::canonicalize(extracted_root)
        .map_err(|error| RuntimeArchiveError::Io(format!("无法规范运行时解压目录：{error}")))?;
    let canonical_server = fs::canonicalize(server_path).map_err(|error| {
        RuntimeArchiveError::Invalid(format!("manifest 指定的 llama-server 无效：{error}"))
    })?;
    if !canonical_server.starts_with(&canonical_root) {
        return Err(RuntimeArchiveError::Invalid(
            "manifest 指定的 llama-server 越出运行时解压目录".to_string(),
        ));
    }
    Ok(())
}

fn check_cancelled(cancellation: &watch::Receiver<bool>) -> Result<(), RuntimeArchiveError> {
    if *cancellation.borrow() {
        Err(RuntimeArchiveError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;

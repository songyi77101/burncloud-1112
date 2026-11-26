use futures_util::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderName, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, RANGE},
    redirect::Policy,
    Client, Response, StatusCode, Url,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::watch,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const PROVIDER: &str = "gitee";
const REPOSITORY: &str = "songyi777/llama-cpp";
const COMMIT: &str = "51d01f6e526bc6d45b9d252b1d2d487efb6625d8";
const RAW_URL: &str = "https://gitee.com/songyi777/llama-cpp/raw";
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_SIDECAR_BYTES: u64 = 64 * 1024;
const CHECKPOINT_INTERVAL_BYTES: u64 = 1024 * 1024;
const MAX_ATTEMPTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(45);
const RETRY_DELAY: Duration = Duration::from_millis(250);
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

/// 受信任 Gitee 运行时归档的固定身份。
///
/// URL 由本模块按固定仓库和 commit 生成，调用方不能传入任意下载地址。大小和 SHA-256
/// 必须来自服务端 manifest，不得直接使用 HTTP 请求字段。
///
/// # Notes
/// 构造器只接受四个平台的固定 target 与文件名；调用方提供的长度和摘要仍须来自可信服务端清单。
///
/// # Examples
/// ```
/// use burncloud_download::RuntimeArchive;
/// let archive = RuntimeArchive::new("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip", 25_262_139, "5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d")?;
/// assert_eq!(archive.target(), "x86_64-pc-windows-msvc");
/// # Ok::<(), burncloud_download::RuntimeDownloadError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeArchive {
    target: String,
    filename: String,
    size_bytes: u64,
    sha256: String,
}

impl RuntimeArchive {
    /// 校验并构造四个平台之一的固定归档身份。
    ///
    /// # Examples
    /// ```
    /// use burncloud_download::RuntimeArchive;
    /// let archive = RuntimeArchive::new("x86_64-unknown-linux-gnu", "llama-b12757-bin-ubuntu-x64.tar.gz", 23_425_964, "99e34cc6a39f3afda735d43d76d15d6ead2297406cc9c0cf421b771fd3a1a84a")?;
    /// assert_eq!(archive.filename(), "llama-b12757-bin-ubuntu-x64.tar.gz");
    /// # Ok::<(), burncloud_download::RuntimeDownloadError>(())
    /// ```
    ///
    /// # Notes
    /// 长度与摘要由可信 manifest 提供；此构造器不会从 Gitee 动态发现或刷新它们。
    pub fn new(
        target: impl Into<String>,
        filename: impl Into<String>,
        size_bytes: u64,
        sha256: impl Into<String>,
    ) -> Result<Self, RuntimeDownloadError> {
        let target = target.into();
        let filename = filename.into();
        let sha256 = sha256.into();
        let identity_is_known = matches!(
            (target.as_str(), filename.as_str()),
            ("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip")
                | (
                    "aarch64-apple-darwin",
                    "llama-b12757-bin-macos-arm64.tar.gz"
                )
                | ("x86_64-apple-darwin", "llama-b12757-bin-macos-x64.tar.gz")
                | (
                    "x86_64-unknown-linux-gnu",
                    "llama-b12757-bin-ubuntu-x64.tar.gz"
                )
        );
        if !identity_is_known
            || !is_safe_filename(&filename)
            || size_bytes == 0
            || size_bytes > MAX_ARCHIVE_BYTES
            || !is_sha256(&sha256)
        {
            return Err(RuntimeDownloadError::InvalidArchiveIdentity);
        }
        Ok(Self {
            target,
            filename,
            size_bytes,
            sha256,
        })
    }

    /// 返回固定 Cargo target。
    ///
    /// # Examples
    /// ```
    /// # use burncloud_download::RuntimeArchive;
    /// # let archive = RuntimeArchive::new("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip", 1, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
    /// assert_eq!(archive.target(), "x86_64-pc-windows-msvc");
    /// # Ok::<(), burncloud_download::RuntimeDownloadError>(())
    /// ```
    ///
    /// # Notes
    /// 该值与构造时校验通过的平台身份一致。
    pub fn target(&self) -> &str {
        &self.target
    }
    /// 返回 manifest 登记的归档基名。
    ///
    /// # Examples
    /// ```
    /// # use burncloud_download::RuntimeArchive;
    /// # let archive = RuntimeArchive::new("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip", 1, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
    /// assert_eq!(archive.filename(), "llama-b12757-bin-win-cpu-x64.zip");
    /// # Ok::<(), burncloud_download::RuntimeDownloadError>(())
    /// ```
    ///
    /// # Notes
    /// 文件名属于固定资源身份的一部分，不包含目录。
    pub fn filename(&self) -> &str {
        &self.filename
    }
    /// 返回期望压缩字节数。
    ///
    /// # Examples
    /// ```
    /// # use burncloud_download::RuntimeArchive;
    /// # let archive = RuntimeArchive::new("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip", 25_262_139, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
    /// assert_eq!(archive.size_bytes(), 25_262_139);
    /// # Ok::<(), burncloud_download::RuntimeDownloadError>(())
    /// ```
    ///
    /// # Notes
    /// 下载器会将实际长度与此值比较。
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    /// 返回期望的完整小写 SHA-256。
    ///
    /// # Examples
    /// ```
    /// # use burncloud_download::RuntimeArchive;
    /// # let sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    /// # let archive = RuntimeArchive::new("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip", 1, sha256)?;
    /// assert_eq!(archive.sha256(), sha256);
    /// # Ok::<(), burncloud_download::RuntimeDownloadError>(())
    /// ```
    ///
    /// # Notes
    /// 此值用于对完整下载文件重新计算并校验 SHA-256。
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    fn url(&self) -> Result<Url, RuntimeDownloadError> {
        Url::parse(&format!("{RAW_URL}/{COMMIT}/{}", self.filename))
            .map_err(|_| RuntimeDownloadError::InvalidArchiveIdentity)
    }
}

/// 已完整核对大小及 SHA-256 的运行时归档。
///
/// 此类型仅由 [`download_runtime_archive`] 返回，路径指向下载目录中的已验证文件。
///
/// # Examples
/// ```no_run
/// use burncloud_download::{download_runtime_archive, RuntimeArchive, RuntimeDownloadProgress};
/// use std::path::Path;
/// use tokio::sync::watch;
/// use tokio_util::sync::CancellationToken;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let archive = RuntimeArchive::new("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip", 25_262_139, "5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d")?;
/// let (progress, _receiver) = watch::channel(RuntimeDownloadProgress::default());
/// let verified = download_runtime_archive(&archive, Path::new("C:/Users/example/AppData/Local/BurnCloud/runtimes/downloads"), &progress, &CancellationToken::new()).await?;
/// assert_eq!(verified.size_bytes(), archive.size_bytes());
/// assert_eq!(verified.sha256(), archive.sha256());
/// # Ok(())
/// # }
/// ```
///
/// # Notes
/// 该类型不表示仅依据 HTTP 声明或 sidecar 推断的完整文件；返回前已重新核对实际长度和 SHA-256。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArchive {
    path: PathBuf,
    size_bytes: u64,
    sha256: String,
}
impl VerifiedArchive {
    /// 返回已校验归档的绝对路径。
    ///
    /// # Examples
    /// 请参见 [`VerifiedArchive`] 的完整下载示例。
    ///
    /// # Notes
    /// 路径位于调用方传入的受信任运行时下载目录内。
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// 返回实际压缩字节数。
    ///
    /// # Examples
    /// 请参见 [`VerifiedArchive`] 的完整下载示例。
    ///
    /// # Notes
    /// 返回值是文件系统中的实际长度，并已与 manifest 期望长度相等。
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    /// 返回重新计算的完整 SHA-256。
    ///
    /// # Examples
    /// 请参见 [`VerifiedArchive`] 的完整下载示例。
    ///
    /// # Notes
    /// 返回值已经与 manifest 中的预期摘要匹配。
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// 原生运行时归档下载的累计进度快照。
///
/// # Examples
/// ```
/// use burncloud_download::RuntimeDownloadProgress;
/// let progress = RuntimeDownloadProgress {
///     completed_bytes: 512,
///     total_bytes: 1024,
///     bytes_per_second: 128,
/// };
/// assert_eq!(progress.completed_bytes, 512);
/// ```
///
/// # Notes
/// 发生实体变化或重新下载时，`completed_bytes` 可以回退；速率只基于当前传输尝试新增的字节计算。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeDownloadProgress {
    /// 已接收字节数；远端实体变化时允许回退。
    pub completed_bytes: u64,
    /// manifest 固定的压缩字节数。
    pub total_bytes: u64,
    /// 当前传输尝试实际新增字节的平均速率，单位为字节/秒。
    pub bytes_per_second: u64,
}

/// Gitee 固定运行时归档下载错误。
///
/// # Examples
/// ```
/// use burncloud_download::RuntimeDownloadError;
/// assert_eq!(RuntimeDownloadError::ChecksumMismatch.code(), "runtime_checksum_mismatch");
/// ```
///
/// # Notes
/// 对外返回时优先使用 [`RuntimeDownloadError::code`]；不要将可能含本地路径或请求细节的底层错误文本直接返回给客户端。
#[derive(Debug, Error)]
pub enum RuntimeDownloadError {
    /// 归档身份字段不是四个平台之一的固定资源。
    #[error("运行时归档身份无效")]
    InvalidArchiveIdentity,
    /// 下载目录不是绝对且安全的路径。
    #[error("运行时下载目录无效")]
    InvalidDownloadDirectory,
    /// 固定来源拒绝访问、资源不存在或返回了不允许的重定向。
    #[error("固定 Gitee 运行时来源不可用（HTTP {0}）")]
    SourceUnavailable(u16),
    /// 上游返回可重试的服务器错误。
    #[error("运行时资源服务器暂时错误（HTTP {0}）")]
    ServerStatus(u16),
    /// HTTP 请求或响应传输失败。
    #[error("运行时归档传输失败：{0}")]
    Transport(#[source] reqwest::Error),
    /// 传输超过允许的无进展时限。
    #[error("运行时归档传输无进展超时")]
    NoProgressTimeout,
    /// 本地文件或断点侧车操作失败。
    #[error("运行时下载文件操作失败：{0}")]
    Io(#[from] io::Error),
    /// HTTP 响应不是完整包或可信续传范围。
    #[error("运行时归档响应无效")]
    InvalidResponse,
    /// 实际归档长度不符合 manifest。
    #[error("运行时归档长度校验失败")]
    SizeMismatch,
    /// 实际完整 SHA-256 不符合 manifest。
    #[error("运行时归档 SHA-256 校验失败")]
    ChecksumMismatch,
    /// 当前下载订阅已取消。
    #[error("运行时归档下载已取消")]
    Cancelled,
    /// 固定来源在有限重试后仍未提供有效归档。
    #[error("固定 Gitee 运行时来源在有限重试后仍不可用")]
    RetriesExhausted,
}

impl RuntimeDownloadError {
    /// 返回自动部署任务使用的稳定错误码。
    ///
    /// # Examples
    /// ```
    /// use burncloud_download::RuntimeDownloadError;
    /// assert_eq!(RuntimeDownloadError::SourceUnavailable(404).code(), "runtime_source_unavailable");
    /// ```
    ///
    /// # Notes
    /// 新增错误变体时应同步确定对应的稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArchiveIdentity | Self::InvalidDownloadDirectory => {
                "runtime_manifest_invalid"
            }
            Self::SourceUnavailable(_) => "runtime_source_unavailable",
            Self::SizeMismatch => "runtime_size_mismatch",
            Self::ChecksumMismatch => "runtime_checksum_mismatch",
            Self::Cancelled => "cancelled",
            Self::Transport(_)
            | Self::ServerStatus(_)
            | Self::NoProgressTimeout
            | Self::Io(_)
            | Self::InvalidResponse
            | Self::RetriesExhausted => "runtime_download_failed",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePartialMetadata {
    schema_version: u32,
    provider: String,
    repository: String,
    commit: String,
    target: String,
    filename: String,
    url: String,
    expected_size: u64,
    expected_sha256: String,
    strong_etag: String,
    total_bytes: u64,
    committed_bytes: u64,
    updated_at: String,
}

struct PartialFile {
    file: File,
    metadata: RuntimePartialMetadata,
    sidecar_path: PathBuf,
    received_bytes: u64,
    checkpoint_bytes: u64,
}

struct DownloadContext<'a> {
    archive: &'a RuntimeArchive,
    url: &'a Url,
    client: &'a Client,
    progress: &'a watch::Sender<RuntimeDownloadProgress>,
    cancellation: &'a CancellationToken,
}

/// 从固定 Gitee commit 流式获取并校验运行时归档。
///
/// `download_dir` 必须由服务端缓存管理器解析为用户数据目录中的运行时下载目录，不能取自
/// HTTP 请求。进度通过 watch 通道报告；取消时仅保留带强 ETag 与连续长度证明的断点。
///
/// # Notes
/// 生产 URL、仓库和 commit 固定在服务端代码中；该函数不提供客户端可控的版本、SHA、URL 或解压路径。
///
/// # Examples
/// ```no_run
/// use burncloud_download::{download_runtime_archive, RuntimeArchive, RuntimeDownloadProgress};
/// use std::path::Path;
/// use tokio::sync::watch;
/// use tokio_util::sync::CancellationToken;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let archive = RuntimeArchive::new("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip", 25_262_139, "5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d")?;
/// let (progress, _receiver) = watch::channel(RuntimeDownloadProgress::default());
/// let cancellation = CancellationToken::new();
/// let _verified = download_runtime_archive(&archive, Path::new("C:/Users/example/AppData/Local/BurnCloud/runtimes/downloads"), &progress, &cancellation).await?;
/// # Ok(())
/// # }
/// ```
pub async fn download_runtime_archive(
    archive: &RuntimeArchive,
    download_dir: &Path,
    progress: &watch::Sender<RuntimeDownloadProgress>,
    cancellation: &CancellationToken,
) -> Result<VerifiedArchive, RuntimeDownloadError> {
    download_runtime_archive_at(
        archive,
        download_dir,
        archive.url()?,
        progress,
        cancellation,
    )
    .await
}

async fn download_runtime_archive_at(
    archive: &RuntimeArchive,
    download_dir: &Path,
    url: Url,
    progress: &watch::Sender<RuntimeDownloadProgress>,
    cancellation: &CancellationToken,
) -> Result<VerifiedArchive, RuntimeDownloadError> {
    validate_download_directory(download_dir)?;
    fs::create_dir_all(download_dir).await?;
    let part_path = download_dir.join(format!("{}.part", archive.filename));
    let sidecar_path = download_dir.join(format!("{}.part.json", archive.filename));
    let final_path = download_dir.join(&archive.filename);
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(Policy::none())
        .build()
        .map_err(RuntimeDownloadError::Transport)?;
    if let Some(verified) = verify_existing_archive(archive, &final_path).await? {
        publish_progress(progress, archive.size_bytes, archive.size_bytes, 0);
        return Ok(verified);
    }

    let context = DownloadContext {
        archive,
        url: &url,
        client: &client,
        progress,
        cancellation,
    };
    let mut last_error = RuntimeDownloadError::RetriesExhausted;
    for attempt in 0..MAX_ATTEMPTS {
        if cancellation.is_cancelled() {
            return Err(RuntimeDownloadError::Cancelled);
        }
        match download_attempt(&context, &part_path, &sidecar_path).await {
            Ok(()) => {
                let actual_size = fs::metadata(&part_path).await?.len();
                if actual_size != archive.size_bytes {
                    last_error = RuntimeDownloadError::SizeMismatch;
                    reset_partial(&part_path, &sidecar_path).await?;
                    publish_progress(progress, 0, archive.size_bytes, 0);
                } else {
                    let actual_sha256 = hash_file(&part_path).await?;
                    if actual_sha256 != archive.sha256 {
                        last_error = RuntimeDownloadError::ChecksumMismatch;
                        reset_partial(&part_path, &sidecar_path).await?;
                        publish_progress(progress, 0, archive.size_bytes, 0);
                    } else {
                        replace_verified_file(&part_path, &final_path).await?;
                        remove_sidecar(&sidecar_path).await?;
                        publish_progress(progress, actual_size, archive.size_bytes, 0);
                        return Ok(VerifiedArchive {
                            path: final_path,
                            size_bytes: actual_size,
                            sha256: actual_sha256,
                        });
                    }
                }
            }
            Err(RuntimeDownloadError::Cancelled) => return Err(RuntimeDownloadError::Cancelled),
            Err(error @ RuntimeDownloadError::SourceUnavailable(_)) => return Err(error),
            Err(error) if is_retryable(&error) => last_error = error,
            Err(error) => {
                reset_partial(&part_path, &sidecar_path).await?;
                return Err(error);
            }
        }
        if attempt + 1 < MAX_ATTEMPTS {
            tokio::select! {
                _ = cancellation.cancelled() => return Err(RuntimeDownloadError::Cancelled),
                _ = tokio::time::sleep(RETRY_DELAY * (attempt as u32 + 1)) => {}
            }
        }
    }
    Err(last_error)
}

async fn download_attempt(
    context: &DownloadContext<'_>,
    part_path: &Path,
    sidecar_path: &Path,
) -> Result<(), RuntimeDownloadError> {
    let mut partial = open_partial(context.archive, context.url, part_path, sidecar_path).await?;
    publish_progress(
        context.progress,
        partial.received_bytes,
        context.archive.size_bytes,
        0,
    );
    let has_resume = partial.metadata.committed_bytes > 0;
    let mut request = context.client.get(context.url.clone());
    if has_resume {
        request = request
            .header(
                RANGE,
                format!("bytes={}-", partial.metadata.committed_bytes),
            )
            .header(IF_RANGE, partial.metadata.strong_etag.clone());
    }
    let response = send_request(request, context.cancellation).await?;
    match response.status() {
        StatusCode::OK => {
            if response.headers().contains_key(CONTENT_RANGE) {
                return Err(RuntimeDownloadError::InvalidResponse);
            }
            begin_full_response(
                &mut partial,
                context.archive,
                context.url,
                &response,
                context.progress,
            )
            .await?;
            stream_response(
                response,
                context.archive,
                partial,
                context.progress,
                context.cancellation,
            )
            .await
        }
        StatusCode::PARTIAL_CONTENT if has_resume => {
            let resume_matches = match response_matches_partial(
                context.archive,
                context.url,
                &partial.metadata,
                &response,
            ) {
                Ok(matches) => matches,
                // Content-Length 无效时不能把 206 当作可续传实体；安全地丢弃响应并全量重下。
                Err(RuntimeDownloadError::InvalidResponse) => false,
                Err(error) => return Err(error),
            };
            if resume_matches {
                stream_response(
                    response,
                    context.archive,
                    partial,
                    context.progress,
                    context.cancellation,
                )
                .await
            } else {
                drop(response);
                drop(partial);
                reset_partial(part_path, sidecar_path).await?;
                publish_progress(context.progress, 0, context.archive.size_bytes, 0);
                download_full_response(context, part_path, sidecar_path).await
            }
        }
        status if status.is_redirection() || status.is_client_error() => {
            Err(RuntimeDownloadError::SourceUnavailable(status.as_u16()))
        }
        status if status.is_server_error() => {
            Err(RuntimeDownloadError::ServerStatus(status.as_u16()))
        }
        _ => Err(RuntimeDownloadError::InvalidResponse),
    }
}

async fn download_full_response(
    context: &DownloadContext<'_>,
    part_path: &Path,
    sidecar_path: &Path,
) -> Result<(), RuntimeDownloadError> {
    let response = send_request(
        context.client.get(context.url.clone()),
        context.cancellation,
    )
    .await?;
    if response.status().is_redirection() || response.status().is_client_error() {
        return Err(RuntimeDownloadError::SourceUnavailable(
            response.status().as_u16(),
        ));
    }
    if response.status().is_server_error() {
        return Err(RuntimeDownloadError::ServerStatus(
            response.status().as_u16(),
        ));
    }
    if response.status() != StatusCode::OK || response.headers().contains_key(CONTENT_RANGE) {
        return Err(RuntimeDownloadError::InvalidResponse);
    }
    let mut partial =
        open_empty_partial(context.archive, context.url, part_path, sidecar_path).await?;
    begin_full_response(
        &mut partial,
        context.archive,
        context.url,
        &response,
        context.progress,
    )
    .await?;
    stream_response(
        response,
        context.archive,
        partial,
        context.progress,
        context.cancellation,
    )
    .await
}

async fn send_request(
    request: reqwest::RequestBuilder,
    cancellation: &CancellationToken,
) -> Result<Response, RuntimeDownloadError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(RuntimeDownloadError::Cancelled),
        result = timeout(CONNECT_TIMEOUT, request.send()) => match result {
            Err(_) => Err(RuntimeDownloadError::NoProgressTimeout),
            Ok(Err(error)) => Err(RuntimeDownloadError::Transport(error)),
            Ok(Ok(response)) => Ok(response),
        }
    }
}

async fn begin_full_response(
    partial: &mut PartialFile,
    archive: &RuntimeArchive,
    url: &Url,
    response: &Response,
    progress: &watch::Sender<RuntimeDownloadProgress>,
) -> Result<(), RuntimeDownloadError> {
    if response.url() != url {
        return Err(RuntimeDownloadError::InvalidResponse);
    }
    if parse_header_u64(response, &CONTENT_LENGTH)?
        .is_some_and(|length| length != archive.size_bytes)
    {
        return Err(RuntimeDownloadError::SizeMismatch);
    }
    partial.file.set_len(0).await?;
    partial.file.seek(io::SeekFrom::Start(0)).await?;
    partial.received_bytes = 0;
    partial.checkpoint_bytes = 0;
    publish_progress(progress, 0, archive.size_bytes, 0);
    partial.metadata = metadata_for_response(archive, url, response);
    if partial.metadata.strong_etag.is_empty() {
        remove_sidecar(&partial.sidecar_path).await?;
    } else {
        save_checkpoint(partial).await?;
    }
    Ok(())
}

fn metadata_for_response(
    archive: &RuntimeArchive,
    url: &Url,
    response: &Response,
) -> RuntimePartialMetadata {
    let strong_etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_strong_etag(value))
        .unwrap_or_default()
        .to_string();
    RuntimePartialMetadata {
        schema_version: 1,
        provider: PROVIDER.to_string(),
        repository: REPOSITORY.to_string(),
        commit: COMMIT.to_string(),
        target: archive.target.clone(),
        filename: archive.filename.clone(),
        url: url.as_str().to_string(),
        expected_size: archive.size_bytes,
        expected_sha256: archive.sha256.clone(),
        strong_etag,
        total_bytes: archive.size_bytes,
        committed_bytes: 0,
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn response_matches_partial(
    archive: &RuntimeArchive,
    url: &Url,
    metadata: &RuntimePartialMetadata,
    response: &Response,
) -> Result<bool, RuntimeDownloadError> {
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok());
    let range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range);
    let (Some(etag), Some(ParsedContentRange::Partial { start, end, total })) = (etag, range)
    else {
        return Ok(false);
    };
    if start != metadata.committed_bytes
        || start == 0
        || start > end
        || end.checked_add(1) != Some(total)
        || total != archive.size_bytes
        || metadata.total_bytes != total
        || metadata.strong_etag != etag
        || !is_strong_etag(etag)
        || metadata.url != url.as_str()
        || metadata.expected_size != archive.size_bytes
        || metadata.expected_sha256 != archive.sha256
        || metadata.target != archive.target
        || metadata.filename != archive.filename
        || response.url() != url
    {
        return Ok(false);
    }
    let content_length_matches = content_length_matches(response.headers(), total - start)?;
    Ok(content_length_matches)
}

async fn stream_response(
    response: Response,
    archive: &RuntimeArchive,
    mut partial: PartialFile,
    progress: &watch::Sender<RuntimeDownloadProgress>,
    cancellation: &CancellationToken,
) -> Result<(), RuntimeDownloadError> {
    let mut stream = response.bytes_stream();
    let attempt_started_at = Instant::now();
    let attempt_start_bytes = partial.received_bytes;
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => { persist_or_discard_partial(&mut partial).await?; return Err(RuntimeDownloadError::Cancelled); }
            result = timeout(NO_PROGRESS_TIMEOUT, stream.next()) => match result {
                Err(_) => { persist_or_discard_partial(&mut partial).await?; return Err(RuntimeDownloadError::NoProgressTimeout); }
                Ok(next) => next,
            }
        };
        let Some(next) = next else {
            break;
        };
        let bytes = match next {
            Ok(bytes) => bytes,
            Err(error) => {
                persist_or_discard_partial(&mut partial).await?;
                return Err(RuntimeDownloadError::Transport(error));
            }
        };
        let new_size = partial
            .received_bytes
            .checked_add(bytes.len() as u64)
            .filter(|size| *size <= archive.size_bytes)
            .ok_or(RuntimeDownloadError::SizeMismatch)?;
        partial.file.write_all(&bytes).await?;
        partial.received_bytes = new_size;
        publish_progress(
            progress,
            new_size,
            archive.size_bytes,
            average_speed(
                new_size.saturating_sub(attempt_start_bytes),
                attempt_started_at.elapsed(),
            ),
        );
        if is_strong_etag(&partial.metadata.strong_etag)
            && new_size.saturating_sub(partial.checkpoint_bytes) >= CHECKPOINT_INTERVAL_BYTES
        {
            partial.metadata.committed_bytes = new_size;
            partial.metadata.updated_at = chrono::Utc::now().to_rfc3339();
            save_checkpoint(&mut partial).await?;
        }
    }
    partial.file.flush().await?;
    partial.file.sync_data().await?;
    if partial.received_bytes != archive.size_bytes {
        persist_or_discard_partial(&mut partial).await?;
        return Err(RuntimeDownloadError::SizeMismatch);
    }
    Ok(())
}

async fn open_partial(
    archive: &RuntimeArchive,
    url: &Url,
    part_path: &Path,
    sidecar_path: &Path,
) -> Result<PartialFile, RuntimeDownloadError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(part_path)
        .await?;
    let actual_length = file.metadata().await?.len();
    let metadata = read_checkpoint(sidecar_path).await?.filter(|meta| {
        meta.schema_version == 1
            && meta.provider == PROVIDER
            && meta.repository == REPOSITORY
            && meta.commit == COMMIT
            && meta.target == archive.target
            && meta.filename == archive.filename
            && meta.url == url.as_str()
            && meta.expected_size == archive.size_bytes
            && meta.expected_sha256 == archive.sha256
            && meta.total_bytes == archive.size_bytes
            && meta.committed_bytes > 0
            && meta.committed_bytes <= actual_length
            && meta.committed_bytes < archive.size_bytes
            && is_strong_etag(&meta.strong_etag)
    });
    let mut file = file;
    let (metadata, committed_bytes) = match metadata {
        Some(meta) => {
            if actual_length != meta.committed_bytes {
                file.set_len(meta.committed_bytes).await?;
            }
            file.seek(io::SeekFrom::Start(meta.committed_bytes)).await?;
            let committed = meta.committed_bytes;
            (meta, committed)
        }
        None => {
            file.set_len(0).await?;
            file.seek(io::SeekFrom::Start(0)).await?;
            remove_sidecar(sidecar_path).await?;
            (empty_metadata(archive, url), 0)
        }
    };
    Ok(PartialFile {
        file,
        metadata,
        sidecar_path: sidecar_path.to_path_buf(),
        received_bytes: committed_bytes,
        checkpoint_bytes: committed_bytes,
    })
}

async fn open_empty_partial(
    archive: &RuntimeArchive,
    url: &Url,
    part_path: &Path,
    sidecar_path: &Path,
) -> Result<PartialFile, RuntimeDownloadError> {
    let mut partial = open_partial(archive, url, part_path, sidecar_path).await?;
    partial.file.set_len(0).await?;
    partial.file.seek(io::SeekFrom::Start(0)).await?;
    partial.metadata = empty_metadata(archive, url);
    partial.received_bytes = 0;
    partial.checkpoint_bytes = 0;
    Ok(partial)
}

fn empty_metadata(archive: &RuntimeArchive, url: &Url) -> RuntimePartialMetadata {
    RuntimePartialMetadata {
        schema_version: 1,
        provider: PROVIDER.to_string(),
        repository: REPOSITORY.to_string(),
        commit: COMMIT.to_string(),
        target: archive.target.clone(),
        filename: archive.filename.clone(),
        url: url.as_str().to_string(),
        expected_size: archive.size_bytes,
        expected_sha256: archive.sha256.clone(),
        strong_etag: String::new(),
        total_bytes: archive.size_bytes,
        committed_bytes: 0,
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

async fn persist_or_discard_partial(partial: &mut PartialFile) -> Result<(), RuntimeDownloadError> {
    if is_strong_etag(&partial.metadata.strong_etag) {
        partial.metadata.committed_bytes = partial.received_bytes;
        partial.metadata.updated_at = chrono::Utc::now().to_rfc3339();
        save_checkpoint(partial).await
    } else {
        partial.file.set_len(0).await?;
        partial.file.sync_data().await?;
        remove_sidecar(&partial.sidecar_path).await
    }
}

async fn save_checkpoint(partial: &mut PartialFile) -> Result<(), RuntimeDownloadError> {
    partial.file.flush().await?;
    partial.file.sync_data().await?;
    let payload =
        serde_json::to_vec(&partial.metadata).map_err(|_| RuntimeDownloadError::InvalidResponse)?;
    if payload.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(RuntimeDownloadError::InvalidResponse);
    }
    let path = partial.sidecar_path.clone();
    tokio::task::spawn_blocking(move || replace_sidecar(&path, &payload))
        .await
        .map_err(io::Error::other)??;
    partial.checkpoint_bytes = partial.metadata.committed_bytes;
    Ok(())
}

fn replace_sidecar(path: &Path, payload: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "断点侧车缺少父目录"))?;
    let mut staging = NamedTempFile::new_in(parent)?;
    staging.write_all(payload)?;
    staging.as_file().sync_all()?;
    drop(staging.persist(path).map_err(|failure| failure.error)?);
    Ok(())
}

async fn read_checkpoint(
    path: &Path,
) -> Result<Option<RuntimePartialMetadata>, RuntimeDownloadError> {
    let file = match File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut payload = Vec::new();
    file.take(MAX_SIDECAR_BYTES + 1)
        .read_to_end(&mut payload)
        .await?;
    if payload.len() as u64 > MAX_SIDECAR_BYTES {
        return Ok(None);
    }
    Ok(serde_json::from_slice(&payload).ok())
}

async fn verify_existing_archive(
    archive: &RuntimeArchive,
    path: &Path,
) -> Result<Option<VerifiedArchive>, RuntimeDownloadError> {
    let metadata = match fs::metadata(path).await {
        Ok(metadata) if metadata.is_file() && metadata.len() == archive.size_bytes => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let sha256 = hash_file(path).await?;
    if sha256 != archive.sha256 {
        return Ok(None);
    }
    Ok(Some(VerifiedArchive {
        path: path.to_path_buf(),
        size_bytes: metadata.len(),
        sha256,
    }))
}

async fn hash_file(path: &Path) -> Result<String, RuntimeDownloadError> {
    let mut file = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

async fn replace_verified_file(
    part_path: &Path,
    final_path: &Path,
) -> Result<(), RuntimeDownloadError> {
    let source = part_path.to_path_buf();
    let destination = final_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> io::Result<()> {
        if destination.exists() {
            std::fs::remove_file(&destination)?;
        }
        std::fs::rename(source, destination)
    })
    .await
    .map_err(io::Error::other)??;
    Ok(())
}

async fn reset_partial(part_path: &Path, sidecar_path: &Path) -> Result<(), RuntimeDownloadError> {
    match fs::remove_file(part_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    remove_sidecar(sidecar_path).await
}

async fn remove_sidecar(path: &Path) -> Result<(), RuntimeDownloadError> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_download_directory(path: &Path) -> Result<(), RuntimeDownloadError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(RuntimeDownloadError::InvalidDownloadDirectory);
    }
    Ok(())
}

fn is_safe_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 255
        && Path::new(filename).components().count() == 1
        && !filename.contains(['/', '\\', '\0'])
        && filename
            .bytes()
            .all(|byte| !byte.is_ascii_control() && byte != b':')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_strong_etag(value: &str) -> bool {
    value
        .strip_prefix('"')
        .and_then(|tag| tag.strip_suffix('"'))
        .is_some_and(|tag| {
            !tag.is_empty()
                && tag
                    .bytes()
                    .all(|byte| matches!(byte, 0x21 | 0x23..=0x7e | 0x80..=0xff))
        })
}

fn content_length_matches(
    headers: &HeaderMap,
    expected: u64,
) -> Result<bool, RuntimeDownloadError> {
    let Some(value) = headers.get(CONTENT_LENGTH) else {
        return Ok(true);
    };
    let value = value
        .to_str()
        .map_err(|_| RuntimeDownloadError::InvalidResponse)?;
    let length = value
        .parse::<u64>()
        .map_err(|_| RuntimeDownloadError::InvalidResponse)?;
    Ok(length == expected)
}

fn publish_progress(
    progress: &watch::Sender<RuntimeDownloadProgress>,
    completed_bytes: u64,
    total_bytes: u64,
    bytes_per_second: u64,
) {
    progress.send_replace(RuntimeDownloadProgress {
        completed_bytes,
        total_bytes,
        bytes_per_second,
    });
}

fn parse_header_u64(
    response: &Response,
    header: &HeaderName,
) -> Result<Option<u64>, RuntimeDownloadError> {
    let Some(value) = response.headers().get(header) else {
        return Ok(None);
    };
    value
        .to_str()
        .map_err(|_| RuntimeDownloadError::InvalidResponse)?
        .parse::<u64>()
        .map(Some)
        .map_err(|_| RuntimeDownloadError::InvalidResponse)
}

#[derive(Debug, PartialEq, Eq)]
enum ParsedContentRange {
    Partial { start: u64, end: u64, total: u64 },
}

fn parse_content_range(value: &str) -> Option<ParsedContentRange> {
    let (unit, range) = value.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (positions, total) = range.split_once('/')?;
    let total = total.parse::<u64>().ok()?;
    let (start, end) = positions.split_once('-')?;
    Some(ParsedContentRange::Partial {
        start: start.parse().ok()?,
        end: end.parse().ok()?,
        total,
    })
}

fn average_speed(bytes: u64, elapsed: Duration) -> u64 {
    (bytes as f64 / elapsed.as_secs_f64().max(0.001)).min(u64::MAX as f64) as u64
}

fn is_retryable(error: &RuntimeDownloadError) -> bool {
    matches!(
        error,
        RuntimeDownloadError::Transport(_)
            | RuntimeDownloadError::ServerStatus(_)
            | RuntimeDownloadError::NoProgressTimeout
            | RuntimeDownloadError::SizeMismatch
            | RuntimeDownloadError::ChecksumMismatch
            | RuntimeDownloadError::RetriesExhausted
    )
}

#[cfg(test)]
#[path = "runtime_http_tests.rs"]
mod tests;

//! Node 模型下载与摘要校验接口。

use axum::{
    extract::{rejection::JsonRejection, Json, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use burncloud_download::{DownloadError, DownloadManager};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncReadExt;

use crate::AppState;

/// 构建公开的 Node 模型下载接口。
pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/node/models/download", post(download_model))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum DownloadSource {
    #[serde(rename = "modelscope")]
    ModelScope,
    #[serde(rename = "hf-mirror")]
    HfMirror,
    #[serde(rename = "huggingface")]
    HuggingFace,
}

impl DownloadSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ModelScope => "modelscope",
            Self::HfMirror => "hf-mirror",
            Self::HuggingFace => "huggingface",
        }
    }

    fn host(&self) -> &'static str {
        match self {
            Self::ModelScope => "modelscope.cn",
            Self::HfMirror => "hf-mirror.com",
            Self::HuggingFace => "huggingface.co",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DownloadModelRequest {
    model: String,
    source: DownloadSource,
    revision: String,
    variant_id: String,
    artifact_uri: String,
    checksum: String,
}

#[derive(Clone, Debug)]
struct DownloadRequestIdentity {
    model: String,
    source: DownloadSource,
    revision: String,
    variant_id: String,
    artifact_uri: String,
    checksum: String,
    storage_directory: String,
    filename: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum DownloadTaskState {
    Queued,
    Downloading,
    Verifying,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TaskError {
    code: String,
    message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct DownloadTaskData {
    task_id: String,
    status: DownloadTaskState,
    model: String,
    source: String,
    revision: String,
    variant_id: String,
    completed_bytes: u64,
    total_bytes: Option<u64>,
    download_speed: u64,
    error: Option<TaskError>,
    local_path: Option<String>,
    actual_sha256: Option<String>,
}

type ValidatedDownload = DownloadRequestIdentity;

#[derive(Clone, Debug)]
struct DownloadTaskSnapshot {
    task_id: i64,
    gid: String,
    identity: DownloadRequestIdentity,
    status: DownloadTaskState,
    completed_bytes: u64,
    total_bytes: Option<u64>,
    download_speed: u64,
    error: Option<TaskError>,
    final_path: Option<String>,
    actual_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
struct DownloadModelResponse {
    success: bool,
    data: DownloadModelData,
}

#[derive(Debug, Serialize)]
struct DownloadModelData {
    status: &'static str,
    model: String,
    source: &'static str,
    revision: String,
    variant_id: String,
    local_path: String,
    actual_sha256: String,
}

#[derive(Debug, Serialize)]
struct DownloadModelErrorResponse {
    success: bool,
    error: DownloadModelErrorBody,
}

#[derive(Debug, Serialize)]
struct DownloadModelErrorBody {
    code: &'static str,
    message: &'static str,
}

#[expect(
    clippy::cognitive_complexity,
    clippy::large_futures,
    reason = "下载接口需要在同一边界完成请求校验、懒加载、下载、摘要读取和统一响应"
)]
async fn download_model(
    State(state): State<AppState>,
    payload: Result<Json<DownloadModelRequest>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => match validate_request(request) {
            Ok(request) => request,
            Err(message) => {
                return error_response(StatusCode::BAD_REQUEST, "invalid_download_request", message)
            }
        },
        Err(error) => {
            tracing::debug!(%error, "模型下载请求体无效");
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_download_request",
                "模型下载请求无效",
            );
        }
    };

    let manager = match state
        .download_manager
        .get_or_try_init(|| async { DownloadManager::new().await })
        .await
    {
        Ok(manager) => manager,
        Err(error) => {
            tracing::error!(%error, "初始化模型下载管理器失败");
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "download_service_unavailable",
                "模型下载服务暂时不可用",
            );
        }
    };

    let directory = storage_directory(&request);
    let path = match manager.download(&request.artifact_uri, &directory).await {
        Ok(path) => path,
        Err(error) => return download_error_response(error),
    };
    let actual_sha256 = match sha256_file(&path).await {
        Ok(checksum) => checksum,
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "读取下载文件并计算摘要失败");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "download_storage_failed",
                "下载文件校验失败",
            );
        }
    };
    if !actual_sha256.eq_ignore_ascii_case(&request.checksum) {
        tracing::warn!(
            path = %path.display(),
            expected = %request.checksum,
            actual = %actual_sha256,
            "下载文件摘要不匹配"
        );
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "checksum_mismatch",
            "下载文件摘要与请求摘要不一致",
        );
    }

    success_response(DownloadModelData {
        status: "verified",
        model: request.model,
        source: request.source.as_str(),
        revision: request.revision,
        variant_id: request.variant_id,
        local_path: path.to_string_lossy().into_owned(),
        actual_sha256,
    })
}

fn validate_request(request: DownloadModelRequest) -> Result<ValidatedDownload, &'static str> {
    let model = request.model.trim().to_owned();
    let revision = request.revision.trim().to_owned();
    let variant_id = request.variant_id.trim().to_owned();
    let artifact_uri = request.artifact_uri.trim().to_owned();
    let checksum = normalize_checksum(&request.checksum).ok_or("模型摘要格式无效")?;

    if !valid_model_id(&model) || !valid_segment(&revision) {
        return Err("模型名称或修订号无效");
    }
    if !valid_variant_id(&variant_id) {
        return Err("量化版本标识无效");
    }

    let url = Url::parse(&artifact_uri).map_err(|_| "工件地址无效")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(request.source.host()))
        || !artifact_path_matches(&url, &request.source, &model, &revision, &variant_id)
    {
        return Err("工件地址必须是支持仓库的 HTTPS GGUF resolve 地址");
    }

    let filename = variant_id
        .rsplit('/')
        .next()
        .map(safe_path_segment)
        .ok_or("量化版本文件名无效")?;
    let identity = DownloadRequestIdentity {
        model,
        source: request.source,
        revision,
        variant_id,
        artifact_uri,
        checksum,
        storage_directory: String::new(),
        filename,
    };
    let storage_directory = storage_directory(&identity);
    Ok(DownloadRequestIdentity {
        storage_directory,
        ..identity
    })
}

fn valid_model_id(value: &str) -> bool {
    value
        .split_once('/')
        .is_some_and(|(owner, name)| valid_segment(owner) && valid_segment(name))
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

fn valid_variant_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.to_ascii_lowercase().ends_with(".gguf")
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.contains('\\')
                && !segment.chars().any(char::is_control)
        })
}

fn artifact_path_matches(
    url: &Url,
    source: &DownloadSource,
    model: &str,
    revision: &str,
    variant_id: &str,
) -> bool {
    let Some((owner, name)) = model.split_once('/') else {
        return false;
    };
    let mut expected = Vec::new();
    if matches!(source, DownloadSource::ModelScope) {
        expected.push("models");
    }
    expected.extend([owner, name, "resolve", revision]);
    expected.extend(variant_id.split('/'));
    url.path_segments()
        .is_some_and(|segments| segments.eq(expected))
}

fn normalize_checksum(value: &str) -> Option<String> {
    let digest = value.trim().strip_prefix("sha256:")?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("sha256:{}", digest.to_ascii_lowercase()))
}

fn storage_directory(request: &DownloadRequestIdentity) -> String {
    let checksum = request
        .checksum
        .strip_prefix("sha256:")
        .unwrap_or(&request.checksum);
    format!(
        "models/{}/{}/{}/{}",
        request.source.as_str(),
        safe_path_segment(&request.model.replace('/', "_")),
        safe_path_segment(&request.revision),
        checksum
    )
}

fn parse_task_id(value: &str) -> Result<i64, &'static str> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("任务 ID 必须是十进制数字");
    }
    value.parse::<i64>().map_err(|_| "任务 ID 超出有效范围")
}

fn safe_path_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|character| character == '.') {
        "_".into()
    } else {
        cleaned
    }
}

async fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

#[expect(
    clippy::cognitive_complexity,
    reason = "下载器错误需要按稳定公开错误码分组，避免向客户端泄露内部错误"
)]
fn download_error_response(error: DownloadError) -> Response {
    match error {
        DownloadError::InvalidUrl(_) | DownloadError::InvalidDirectory(_) => {
            tracing::debug!(%error, "模型下载请求未通过下载器校验");
            error_response(
                StatusCode::BAD_REQUEST,
                "invalid_download_request",
                "下载地址或目录无效",
            )
        }
        DownloadError::InvalidFilename => {
            tracing::debug!(%error, "模型下载文件名无效");
            error_response(
                StatusCode::BAD_REQUEST,
                "invalid_download_request",
                "下载文件名无效",
            )
        }
        error => {
            tracing::warn!(%error, "模型文件下载失败");
            error_response(
                StatusCode::BAD_GATEWAY,
                "download_failed",
                "模型文件下载失败",
            )
        }
    }
}

fn success_response(data: DownloadModelData) -> Response {
    no_store(
        Json(DownloadModelResponse {
            success: true,
            data,
        })
        .into_response(),
    )
}

fn error_response(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    no_store(
        (
            status,
            Json(DownloadModelErrorResponse {
                success: false,
                error: DownloadModelErrorBody { code, message },
            }),
        )
            .into_response(),
    )
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
#[path = "model_download_tests.rs"]
mod tests;

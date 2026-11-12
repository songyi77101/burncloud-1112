//! 主服务模型选择 HTTP 边界；保留独立的工件下载与校验能力。

use super::model::{
    valid_digest, ChatError, ErrorKind, Gpu, Hardware, InitializationRequest, ModelResolution,
    ModelVariant, Operation, Platform, PreparationEvent, PreparationStage,
};
use reqwest::{Client, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(45);

/// 模块内部 HTTP 客户端；生产请求主服务，测试可注入本地响应。
#[derive(Clone)]
pub(super) struct CatalogClient {
    client: Client,
    base: Url,
}

/// 每行均为累计快照，最后一行必须携带完成类型。
#[derive(Clone, Debug, Deserialize)]
pub(super) struct CatalogPage<T> {
    pub(super) items: Vec<T>,
    #[serde(rename = "type")]
    kind: Option<String>,
    pub(super) done: bool,
}

/// 预置项和搜索结果均直接绑定资源，不再二次解析仓库。
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(super) struct CatalogModel {
    pub(super) id: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) preset: bool,
    pub(super) model: String,
    pub(super) source: String,
    pub(super) revision: String,
}

/// 已确认的仓库快照；页面地址不是可部署的模型文件。
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(super) struct CatalogVersion {
    pub(super) name: String,
    pub(super) model: String,
    pub(super) revision: String,
    pub(super) source: String,
    pub(super) resource_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ResolveEnvelope {
    success: bool,
    data: Option<ResolveData>,
    error: Option<RemoteHardwareError>,
}

#[derive(Clone, Debug, Deserialize)]
struct ResolveData {
    status: String,
    model: String,
    hardware: RemoteHardwareSnapshot,
    candidates: Vec<RemoteModelCandidate>,
    reason: Option<RemoteResolveReason>,
}

#[derive(Clone, Debug, Deserialize)]
struct RemoteModelCandidate {
    model_name: String,
    variant_id: String,
    quantization: String,
    min_ram_bytes: u64,
    min_vram_bytes: u64,
    disk_size_bytes: u64,
    runtime: String,
    artifact_uri: String,
    checksum: String,
}

#[derive(Clone, Debug, Deserialize)]
struct RemoteResolveReason {
    code: String,
    message: String,
}

#[derive(Clone, Debug, Deserialize)]
struct HardwareEnvelope {
    success: bool,
    data: Option<RemoteHardwareSnapshot>,
    error: Option<RemoteHardwareError>,
}

#[derive(Clone, Debug, Deserialize)]
struct RemoteHardwareSnapshot {
    platform: String,
    #[serde(rename = "arch")]
    _arch: String,
    cpu: String,
    memory_bytes: u64,
    disk_available_bytes: u64,
    accelerators: Vec<RemoteAccelerator>,
}

#[derive(Clone, Debug, Deserialize)]
struct RemoteAccelerator {
    kind: String,
    name: String,
    memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
struct RemoteHardwareError {
    code: String,
    message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CatalogSelection {
    pub(super) name: String,
    pub(super) preset: Option<String>,
    pub(super) version: CatalogVersion,
}
impl CatalogSelection {
    /// 来源、仓库和修订号共同决定身份。
    ///
    /// # Examples
    /// 同一仓库的 `main` 与 `master` 是不同选择。
    ///
    /// # Notes
    /// 长度前缀避免分隔符歧义，不使用展示名称作为运行标识。
    pub(super) fn id(&self) -> String {
        let v = &self.version;
        format!(
            "catalog:{}:{}:{}:{}:{}",
            v.source,
            v.model.len(),
            v.model,
            v.revision.len(),
            v.revision
        )
    }
}

/// 目录状态不能替代本客户端经过校验的运行凭据。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum VersionStatus {
    Available,
    Preparing,
    Developing,
    Downloadable,
}

impl CatalogClient {
    /// 请求统一主服务，默认跟随 HOST/PORT，可用 BURNCLOUD_SERVER_URL 指向远程主服务。
    ///
    /// # Examples
    /// `BURNCLOUD_SERVER_URL=http://127.0.0.1:3000`。
    ///
    /// # Notes
    /// LiveView 请求发生在 Rust 服务端，桌面请求发生在本机；不管理额外进程。
    pub(super) fn from_env() -> Result<Self, ChatError> {
        let read = |name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(failure(
                ErrorKind::InvalidParameter,
                Operation::Search,
                error.to_string(),
            )),
        };
        let base = match read("BURNCLOUD_SERVER_URL")? {
            Some(value) => value,
            None => {
                let host = read("HOST")?.unwrap_or_else(|| "127.0.0.1".into());
                // 主程序对无效 PORT 使用默认端口，这里保持相同规则，避免空端口变成 HTTP 80。
                let port = read("PORT")?
                    .unwrap_or_else(|| burncloud_common::constants::DEFAULT_PORT.to_string())
                    .parse::<u16>()
                    .unwrap_or(burncloud_common::constants::DEFAULT_PORT);
                server_base(&host, port)
            }
        };
        Self::new(&base, Duration::from_secs(45))
    }

    /// 构造目录客户端，拒绝非 HTTP 地址、带认证信息的地址和零超时。
    ///
    /// # Examples
    /// 测试可以注入本机随机端口和短超时。
    ///
    /// # Notes
    /// 错误配置明确返回错误，不退回旧版独立服务。
    pub(super) fn new(base: &str, timeout: Duration) -> Result<Self, ChatError> {
        let base = Url::parse(base).map_err(|error| {
            failure(
                ErrorKind::InvalidParameter,
                Operation::Search,
                error.to_string(),
            )
        })?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || timeout.is_zero()
        {
            return Err(failure(
                ErrorKind::InvalidParameter,
                Operation::Search,
                "主服务地址或超时配置无效".into(),
            ));
        }
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| transport(error, Operation::Search))?;
        Ok(Self { client, base })
    }

    /// 空查询加载预置，非空查询逐行发布累计搜索快照。
    ///
    /// # Examples
    /// 查询 `qwen` 时，先到达的站点结果无需等待其他站点。
    ///
    /// # Notes
    /// EOF 不能代替最终完成行；异常及截断响应明确报错。
    pub(super) async fn models(
        &self,
        query: &str,
        mut publish: impl FnMut(CatalogPage<CatalogModel>),
    ) -> Result<(), ChatError> {
        let query = query.trim();
        if query.is_empty() {
            let items: Vec<CatalogModel> = self
                .get("/api/models/presets", &[], Operation::Search)
                .await?;
            for (index, model) in items.iter().enumerate() {
                model.validate()?;
                if !model.preset
                    || items[..index]
                        .iter()
                        .any(|previous| previous.id == model.id)
                {
                    return Err(invalid_search("预置目录包含非预置条目或重复标识"));
                }
            }
            publish(CatalogPage {
                items,
                kind: Some("models".into()),
                done: true,
            });
            return Ok(());
        }
        let mut response = self
            .request(
                "/api/models/search",
                &[("q", query.to_string())],
                Operation::Search,
            )
            .await?;
        let mut pending = Vec::new();
        let mut done = false;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| transport(error, Operation::Search))?
        {
            pending.extend_from_slice(&chunk);
            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                let bytes: Vec<_> = pending.drain(..=end).collect();
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                if done {
                    return Err(invalid_search("完成行后仍包含搜索结果"));
                }
                let snapshot = parse_snapshot(&bytes)?;
                done = snapshot.done;
                publish(snapshot);
            }
        }
        if pending.iter().any(|byte| !byte.is_ascii_whitespace()) {
            if done {
                return Err(invalid_search("完成行后仍包含搜索结果"));
            }
            let snapshot = parse_snapshot(&pending)?;
            done = snapshot.done;
            publish(snapshot);
        }
        if !done {
            return Err(invalid_search("模型搜索响应被截断，缺少完成行"));
        }
        Ok(())
    }

    /// 一次状态查询返回资源与状态，预置项只发送其绑定标识。
    ///
    /// # Examples
    /// `preset=qwen3-8b` 不触发客户端搜索或资源页面探测。
    ///
    /// # Notes
    /// 响应资源必须与选中项一致；下载状态仍不代表存在可部署工件。
    pub(super) async fn inspect(
        &self,
        model: &CatalogModel,
    ) -> Result<(Option<CatalogVersion>, VersionStatus), ChatError> {
        model.validate()?;
        #[derive(Deserialize)]
        struct Status {
            status: VersionStatus,
            model: String,
            source: String,
            revision: String,
            resource_url: String,
        }
        let params = if model.preset {
            vec![("preset", model.id.clone())]
        } else {
            vec![
                ("model", model.model.clone()),
                ("source", model.source.clone()),
                ("revision", model.revision.clone()),
            ]
        };
        let result: Status = self
            .get("/api/models/status", &params, Operation::Status)
            .await?;
        let version = CatalogVersion {
            name: model.name.clone(),
            model: result.model,
            source: result.source,
            revision: result.revision,
            resource_url: result.resource_url,
        };
        version.validate()?;
        if version.model != model.model
            || version.source != model.source
            || version.revision != model.revision
        {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Status,
                "状态响应与选中资源不一致".into(),
            ));
        }
        Ok((Some(version), result.status))
    }

    /// 在一次主服务请求中获取硬件快照和全部兼容 Variant。
    pub(super) async fn resolve_model(
        &self,
        selection: &CatalogSelection,
    ) -> Result<ModelResolution, ChatError> {
        selection.version.validate()?;
        let model = selection.version.model.as_str();
        let url = self
            .base
            .join("/api/node/models/resolve")
            .map_err(|error| {
                failure(
                    ErrorKind::InvalidParameter,
                    Operation::Resolve,
                    error.to_string(),
                )
            })?;
        let response = self
            .client
            .post(url)
            .header("Accept", "application/json")
            .json(&ResolveModelRequest {
                model: selection.version.model.clone(),
                source: selection.version.source.clone(),
                revision: selection.version.revision.clone(),
            })
            .send()
            .await
            .map_err(|error| transport(error, Operation::Resolve))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| transport(error, Operation::Resolve))?;
        let envelope: ResolveEnvelope = serde_json::from_slice(&body).map_err(|error| {
            failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                format!("模型解析接口响应格式无效：{error}"),
            )
        })?;
        if !status.is_success() || !envelope.success {
            let error = envelope.error.ok_or_else(|| {
                failure(
                    ErrorKind::InvalidResult,
                    Operation::Resolve,
                    "模型解析失败响应缺少错误信息".into(),
                )
            })?;
            return Err(remote_error(error, Operation::Resolve));
        }
        let data = envelope.data.ok_or_else(|| {
            failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                "模型解析成功响应缺少数据".into(),
            )
        })?;
        if data.model != model || !matches!(data.status.as_str(), "resolved" | "unsupported") {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                "模型解析响应与请求不一致".into(),
            ));
        }
        let hardware = data.hardware.into_hardware()?;
        let model_name = data.model.clone();
        let variants = data
            .candidates
            .into_iter()
            .map(|candidate| {
                if candidate.model_name != model_name {
                    return Err(failure(
                        ErrorKind::InvalidResult,
                        Operation::Resolve,
                        "模型解析候选版本与请求模型不一致".into(),
                    ));
                }
                candidate.into_variant()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let reason = data
            .reason
            .map(|reason| format!("{}: {}", reason.code, reason.message));
        if data.status == "resolved" && variants.is_empty() {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                "模型解析成功响应没有候选版本".into(),
            ));
        }
        if data.status == "resolved" && reason.is_some() {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                "模型解析成功响应包含不支持原因".into(),
            ));
        }
        if data.status == "unsupported" && !variants.is_empty() {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                "模型解析不支持响应包含候选版本".into(),
            ));
        }
        if data.status == "unsupported" && reason.is_none() {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                "模型解析不支持响应缺少原因".into(),
            ));
        }
        Ok(ModelResolution {
            hardware,
            variants,
            reason,
        })
    }

    /// 获取主服务 Node 的一次性硬件快照。
    pub(super) async fn inspect_hardware(&self) -> Result<Hardware, ChatError> {
        let response = self
            .request_raw("/api/node/hardware", &[], Operation::Hardware)
            .await?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| transport(error, Operation::Hardware))?;
        let response: HardwareEnvelope = serde_json::from_slice(&body).map_err(|error| {
            failure(
                ErrorKind::InvalidResult,
                Operation::Hardware,
                format!("硬件接口响应格式无效：{error}"),
            )
        })?;
        if !status.is_success() || !response.success {
            let error = response.error.ok_or_else(|| {
                failure(
                    ErrorKind::InvalidResult,
                    Operation::Hardware,
                    "硬件接口失败响应缺少错误信息".into(),
                )
            })?;
            return Err(failure(
                ErrorKind::ExecutionFailed,
                Operation::Hardware,
                format!("{}: {}", error.code, error.message),
            ));
        }

        let snapshot = response.data.ok_or_else(|| {
            failure(
                ErrorKind::InvalidResult,
                Operation::Hardware,
                "硬件接口成功响应缺少数据".into(),
            )
        })?;
        snapshot.into_hardware()
    }

    async fn request(
        &self,
        path: &str,
        query: &[(&str, String)],
        operation: Operation,
    ) -> Result<reqwest::Response, ChatError> {
        self.request_raw(path, query, operation)
            .await?
            .error_for_status()
            .map_err(|error| transport(error, operation))
    }

    async fn request_raw(
        &self,
        path: &str,
        query: &[(&str, String)],
        operation: Operation,
    ) -> Result<reqwest::Response, ChatError> {
        let url = self
            .base
            .join(path)
            .map_err(|error| failure(ErrorKind::InvalidParameter, operation, error.to_string()))?;
        self.client
            .get(url)
            .query(query)
            .header("Accept", "application/json, application/x-ndjson")
            .send()
            .await
            .map_err(|error| transport(error, operation))
    }
    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        operation: Operation,
    ) -> Result<T, ChatError> {
        self.request(path, query, operation)
            .await?
            .json()
            .await
            .map_err(|error| transport(error, operation))
    }
}

#[derive(Debug, Serialize)]
struct ResolveModelRequest {
    model: String,
    source: String,
    revision: String,
}

impl RemoteHardwareSnapshot {
    fn into_hardware(self) -> Result<Hardware, ChatError> {
        let platform = match self.platform.as_str() {
            "linux" => Some(Platform::Linux),
            "macos" => Some(Platform::MacOs),
            "windows" => Some(Platform::Windows),
            "other" => Some(Platform::Other),
            _ => {
                return Err(failure(
                    ErrorKind::InvalidResult,
                    Operation::Hardware,
                    "硬件接口返回了未知平台".into(),
                ));
            }
        };
        if self.cpu.trim().is_empty() || self._arch.trim().is_empty() {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Hardware,
                "硬件接口返回了空的平台或 CPU 信息".into(),
            ));
        }

        let gpus = self
            .accelerators
            .into_iter()
            .map(RemoteAccelerator::into_gpu)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Hardware {
            platform,
            cpu: Some(self.cpu),
            gpus: Some(gpus),
            ram_bytes: Some(self.memory_bytes),
            disk_bytes: Some(self.disk_available_bytes),
        })
    }
}

impl RemoteModelCandidate {
    fn into_variant(self) -> Result<ModelVariant, ChatError> {
        if self.model_name.trim().is_empty()
            || self.variant_id.trim().is_empty()
            || self.quantization.trim().is_empty()
            || self.runtime.trim().is_empty()
            || self.artifact_uri.trim().is_empty()
            || !valid_digest(&self.checksum)
        {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Resolve,
                "模型解析返回了无效候选版本".into(),
            ));
        }
        Ok(ModelVariant {
            id: self.variant_id,
            quantization: self.quantization,
            runtime: self.runtime,
            artifact_source: self.artifact_uri,
            expected_sha256: self.checksum,
            ram_bytes: self.min_ram_bytes,
            vram_bytes: self.min_vram_bytes,
            disk_bytes: self.disk_size_bytes,
        })
    }
}

impl RemoteAccelerator {
    fn into_gpu(self) -> Result<Gpu, ChatError> {
        let kind = match self.kind.as_str() {
            "nvidia" | "amd" | "intel" | "apple" | "other" => self.kind,
            _ => {
                return Err(failure(
                    ErrorKind::InvalidResult,
                    Operation::Hardware,
                    "硬件接口返回了未知加速器类型".into(),
                ));
            }
        };
        if self.name.trim().is_empty() {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Hardware,
                "硬件接口返回了空加速器名称".into(),
            ));
        }
        Ok(Gpu {
            name: format!("{kind}: {}", self.name),
            available_bytes: self.memory_bytes,
        })
    }
}

fn server_base(host: &str, port: u16) -> String {
    let host = match host {
        "0.0.0.0" => "127.0.0.1".to_string(),
        "::" | "[::]" => "[::1]".to_string(),
        value if value.contains(':') && !value.starts_with('[') => format!("[{value}]"),
        value => value.to_string(),
    };
    format!("http://{host}:{port}")
}

fn remote_error(error: RemoteHardwareError, operation: Operation) -> ChatError {
    let kind = match error.code.as_str() {
        "invalid_model" => ErrorKind::InvalidParameter,
        "model_repository_not_found" => ErrorKind::NotFound,
        "model_repository_unavailable" => ErrorKind::Timeout,
        "unsupported_platform" | "hardware_probe_failed" => ErrorKind::ExecutionFailed,
        "manifest_invalid" | "model_resolution_failed" => ErrorKind::InvalidResult,
        _ => ErrorKind::InvalidResult,
    };
    failure(
        kind,
        operation,
        format!("{}: {}", error.code, error.message),
    )
}

fn invalid_search(reason: &str) -> ChatError {
    failure(ErrorKind::InvalidResult, Operation::Search, reason.into())
}

fn parse_snapshot(bytes: &[u8]) -> Result<CatalogPage<CatalogModel>, ChatError> {
    let snapshot: CatalogPage<CatalogModel> =
        serde_json::from_slice(bytes).map_err(|error| invalid_search(&error.to_string()))?;
    let valid = match (snapshot.done, snapshot.kind.as_deref()) {
        (false, None) => true,
        (true, Some("models")) => !snapshot.items.is_empty(),
        (true, Some("failed" | "unavailable")) => snapshot.items.is_empty(),
        _ => false,
    };
    if !valid {
        return Err(invalid_search("模型搜索快照类型无效"));
    }
    for (index, model) in snapshot.items.iter().enumerate() {
        model.validate()?;
        if model.preset
            || snapshot.items[..index]
                .iter()
                .any(|previous| previous.id == model.id)
        {
            return Err(invalid_search("模型搜索快照含预置项或重复标识"));
        }
    }
    Ok(snapshot)
}

fn valid_resource(model: &str, source: &str, revision: &str) -> bool {
    let safe = |part: &str| {
        !part.is_empty()
            && part.len() <= 128
            && !matches!(part, "." | "..")
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    matches!(source, "modelscope" | "hf-mirror" | "huggingface")
        && safe(revision)
        && model
            .split_once('/')
            .is_some_and(|(org, name)| safe(org) && safe(name))
}

impl CatalogModel {
    /// 在 HTTP 响应边界验证展示项及其资源绑定。
    ///
    /// # Examples
    /// 无来源或含路径跳转的仓库必须拒收。
    ///
    /// # Notes
    /// 不把缺失字段补成虚构仓库。
    pub(super) fn validate(&self) -> Result<(), ChatError> {
        if self.name.trim().is_empty()
            || !valid_resource(&self.model, &self.source, &self.revision)
            || self
                .model
                .rsplit('/')
                .next()
                .is_none_or(|name| name.to_ascii_lowercase() != self.id)
        {
            return Err(invalid_search("模型目录条目无效或资源不完整"));
        }
        Ok(())
    }
}

impl CatalogVersion {
    /// 验证仓库快照可交接，且资源页面为 HTTP(S) 地址。
    ///
    /// # Examples
    /// 无效来源、空修订号或非 HTTP 页面均拒绝交接。
    ///
    /// # Notes
    /// 校验页面并不意味着它是模型文件。
    pub(super) fn validate(&self) -> Result<(), ChatError> {
        let url_ok = Url::parse(&self.resource_url).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        });
        if self.name.trim().is_empty()
            || !valid_resource(&self.model, &self.source, &self.revision)
            || !url_ok
        {
            return Err(failure(
                ErrorKind::InvalidResult,
                Operation::Status,
                "目录资源信息无效或不完整".into(),
            ));
        }
        Ok(())
    }
}

impl<T> CatalogPage<T> {
    /// 仅完成且没有结果时显示空列表或失败说明。
    ///
    /// # Examples
    /// `unavailable` 显示没有匹配模型，而非模拟预置。
    ///
    /// # Notes
    /// 部分来源失败不掩盖已成功返回的结果。
    pub(super) fn feedback(&self) -> Option<String> {
        if !self.done || !self.items.is_empty() {
            return None;
        }
        match self.kind.as_deref() {
            Some("failed") => Some("模型搜索失败，请重试".into()),
            Some("unavailable") => Some("没有找到匹配模型".into()),
            _ => None,
        }
    }
}

/// 生产环境的工件保存根目录：`<本地数据目录>/BurnCloud/models`。
pub(super) fn download_root() -> Result<PathBuf, ChatError> {
    dirs::data_local_dir()
        .map(|dir| dir.join("BurnCloud").join("models"))
        .ok_or_else(|| {
            failure(
                ErrorKind::NotConnected,
                Operation::Initialize(PreparationStage::Downloading),
                "无法确定本地数据目录".into(),
            )
        })
}

/// 流式下载已确认的目录工件并校验 SHA-256，成功时返回本地路径。
///
/// # Notes
/// 先写入 `.part`，字节数与摘要一致后才重命名；失败时删除临时文件。
/// 依次回调下载进度（最多每 1% 一次）、`Downloaded` 与 `Verified`，不执行部署。
pub(super) async fn download(
    root: &Path,
    request: &InitializationRequest,
    mut callback: impl FnMut(PreparationEvent),
) -> Result<String, ChatError> {
    let downloading = Operation::Initialize(PreparationStage::Downloading);
    let variant = &request.variant;
    let url = Url::parse(&variant.artifact_source)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    let (Some(selection), Some(url)) = (request.model.catalog_selection.as_ref(), url) else {
        return Err(failure(
            ErrorKind::InvalidParameter,
            downloading,
            "缺少目录选择快照或工件地址无效".into(),
        ));
    };
    let file_name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .unwrap_or("model");
    let v = &selection.version;
    let dir = root
        .join(segment(&v.source))
        .join(segment(&v.model))
        .join(segment(&v.revision));
    let target = dir.join(segment(&format!("{}-{file_name}", variant.id)));
    let mut part = target.clone().into_os_string();
    part.push(".part");
    let part = PathBuf::from(part);

    if let Err(error) = fetch(&url, &dir, &part, variant, &mut callback).await {
        remove_part(&part).await;
        return Err(error);
    }
    let io = |error: std::io::Error| {
        failure(
            ErrorKind::InvalidResult,
            downloading,
            format!("保存模型文件失败：{error}"),
        )
    };
    let replaced = match tokio::fs::remove_file(&target).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(io(error)),
        _ => tokio::fs::rename(&part, &target).await.map_err(io),
    };
    if let Err(error) = replaced {
        remove_part(&part).await;
        return Err(error);
    }
    let path = target.to_string_lossy().into_owned();
    // 状态机严格比较摘要字符串，因此实际摘要沿用预期摘要的原始写法。
    callback(PreparationEvent::Verified {
        expected: variant.expected_sha256.clone(),
        actual: variant.expected_sha256.clone(),
        path: path.clone(),
    });
    Ok(path)
}

/// 下载与校验已完成、但部署尚未接入时的终止原因。
pub(super) fn deployment_missing(path: &str) -> ChatError {
    failure(
        ErrorKind::NotConnected,
        Operation::Initialize(PreparationStage::LoadingPackage),
        format!("模型已下载并通过摘要校验（{path}），尚未接入部署"),
    )
}

async fn fetch(
    url: &Url,
    dir: &Path,
    part: &Path,
    variant: &ModelVariant,
    callback: &mut impl FnMut(PreparationEvent),
) -> Result<(), ChatError> {
    let downloading = Operation::Initialize(PreparationStage::Downloading);
    let io = |error: std::io::Error| {
        failure(
            ErrorKind::InvalidResult,
            downloading,
            format!("写入模型文件失败：{error}"),
        )
    };
    let size_mismatch = || {
        failure(
            ErrorKind::VerificationFailed,
            Operation::Initialize(PreparationStage::Verifying),
            "校验失败：下载字节数与声明大小不一致".into(),
        )
    };
    tokio::fs::create_dir_all(dir).await.map_err(io)?;
    let mut file = tokio::fs::File::create(part).await.map_err(io)?;
    let client = Client::builder()
        .connect_timeout(DOWNLOAD_TIMEOUT)
        .read_timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(download_failure)?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(download_failure)?;
    let total = variant.disk_bytes;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut last_percent = None;
    while let Some(chunk) = response.chunk().await.map_err(download_failure)? {
        received = received.saturating_add(chunk.len() as u64);
        // 超过声明大小立即中止，防止异常来源写满磁盘。
        if received > total {
            return Err(size_mismatch());
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(io)?;
        // 高频回调会反复刷新界面，只在百分比变化时上报。
        let percent = u128::from(received) * 100 / u128::from(total);
        if last_percent != Some(percent) {
            last_percent = Some(percent);
            callback(PreparationEvent::DownloadProgress {
                completed: received,
                total: Some(total),
            });
        }
    }
    file.flush().await.map_err(io)?;
    drop(file);
    if received != total {
        return Err(size_mismatch());
    }
    callback(PreparationEvent::Downloaded);
    let actual = format!("{:x}", hasher.finalize());
    let expected = variant
        .expected_sha256
        .strip_prefix("sha256:")
        .unwrap_or(&variant.expected_sha256);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(failure(
            ErrorKind::VerificationFailed,
            Operation::Initialize(PreparationStage::Verifying),
            "校验失败：SHA-256 摘要不一致".into(),
        ));
    }
    Ok(())
}

async fn remove_part(part: &Path) {
    if let Err(error) = tokio::fs::remove_file(part).await {
        // 清理失败不覆盖原始错误；残留的 .part 会在下次下载时被截断覆盖。
        tracing::warn!("删除临时模型文件失败：{error}");
    }
}

/// 路径段只保留 `[A-Za-z0-9._-]`，并拒绝纯点号段，防止路径穿越。
fn segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.chars().all(|c| c == '.') {
        "_".into()
    } else {
        cleaned
    }
}

fn download_failure(error: reqwest::Error) -> ChatError {
    let reason = format!("模型文件下载失败，请重试：{error}");
    let mut result = transport(error, Operation::Initialize(PreparationStage::Downloading));
    result.reason = reason;
    result
}

fn failure(kind: ErrorKind, operation: Operation, reason: String) -> ChatError {
    ChatError {
        kind,
        operation,
        reason,
    }
}
fn transport(error: reqwest::Error, operation: Operation) -> ChatError {
    let kind = if error.is_timeout() {
        ErrorKind::Timeout
    } else if error.is_decode() {
        ErrorKind::InvalidResult
    } else {
        ErrorKind::NotConnected
    };
    failure(
        kind,
        operation,
        format!("模型服务请求失败，请确认服务已启动后重试：{error}"),
    )
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

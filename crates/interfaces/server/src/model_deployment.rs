use serde::{de::DeserializeOwned, Deserialize, Serialize};
use uuid::Uuid;

/// 模型来源。
///
/// 该枚举只表示已在 HTTP 边界完成校验的受支持来源，不包含任意 URL。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ModelSource {
    #[serde(rename = "modelscope")]
    ModelScope,
    #[serde(rename = "hf-mirror")]
    HfMirror,
    #[serde(rename = "huggingface")]
    HuggingFace,
}

impl ModelSource {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ModelScope => "modelscope",
            Self::HfMirror => "hf-mirror",
            Self::HuggingFace => "huggingface",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "modelscope" => Some(Self::ModelScope),
            "hf-mirror" => Some(Self::HfMirror),
            "huggingface" => Some(Self::HuggingFace),
            _ => None,
        }
    }
}

/// 客户端请求的后端选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RequestedBackend {
    Auto,
    Cpu,
    Cuda,
}

impl RequestedBackend {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "cpu" => Some(Self::Cpu),
            "cuda" => Some(Self::Cuda),
            _ => None,
        }
    }
}

/// 运行时实际选择的后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EffectiveBackend {
    Cpu,
    Metal,
    Cuda,
}

impl EffectiveBackend {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Metal => "metal",
            Self::Cuda => "cuda",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "cpu" => Some(Self::Cpu),
            "metal" => Some(Self::Metal),
            "cuda" => Some(Self::Cuda),
            _ => None,
        }
    }
}

/// 部署任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TaskStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl TaskStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    pub(super) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

/// 部署流程阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeploymentStage {
    Preflight,
    PreparingRuntime,
    DownloadingModel,
    Verifying,
    Starting,
    HealthChecking,
    AttachingRoute,
}

impl DeploymentStage {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::PreparingRuntime => "preparing_runtime",
            Self::DownloadingModel => "downloading_model",
            Self::Verifying => "verifying",
            Self::Starting => "starting",
            Self::HealthChecking => "health_checking",
            Self::AttachingRoute => "attaching_route",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "preflight" => Some(Self::Preflight),
            "preparing_runtime" => Some(Self::PreparingRuntime),
            "downloading_model" => Some(Self::DownloadingModel),
            "verifying" => Some(Self::Verifying),
            "starting" => Some(Self::Starting),
            "health_checking" => Some(Self::HealthChecking),
            "attaching_route" => Some(Self::AttachingRoute),
            _ => None,
        }
    }
}

/// 受控部署错误；只保存契约规定的错误码和用户可见消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskError {
    pub(super) code: String,
    pub(super) message: String,
}

impl TaskError {
    pub(super) fn storage(message: impl Into<String>) -> Self {
        let reason = message.into();
        tracing::error!(reason = %reason, "本机模型部署状态存储失败");
        Self {
            code: "deployment_storage_failed".to_string(),
            message: "本机部署状态暂时无法可靠读取或保存，请稍后重试".to_string(),
        }
    }

    pub(super) fn service_restarted() -> Self {
        Self {
            code: "service_restarted".to_string(),
            message: "服务重启，未完成部署已中断".to_string(),
        }
    }
}

/// 已在请求边界规范化的部署命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeploymentCommand {
    pub(super) model: String,
    pub(super) source: ModelSource,
    pub(super) revision: String,
    pub(super) variant_id: String,
    pub(super) checksum: String,
    pub(super) backend: RequestedBackend,
}

/// 模型准备分支的持久化快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparationProgress {
    pub(super) status: TaskStatus,
    pub(super) stage: Option<DeploymentStage>,
    pub(super) detail: Option<String>,
    pub(super) task_id: Option<String>,
    pub(super) completed_bytes: u64,
    pub(super) total_bytes: Option<u64>,
    pub(super) download_speed: u64,
    pub(super) actual_sha256: Option<String>,
    pub(super) error: Option<TaskError>,
}

/// 运行时准备分支的持久化快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeProgress {
    pub(super) status: TaskStatus,
    pub(super) stage: Option<DeploymentStage>,
    pub(super) detail: Option<String>,
    pub(super) task_id: Option<String>,
    pub(super) completed_bytes: u64,
    pub(super) total_bytes: Option<u64>,
    pub(super) download_speed: u64,
    pub(super) actual_sha256: Option<String>,
    pub(super) error: Option<TaskError>,
    pub(super) origin: Option<String>,
    pub(super) version: Option<String>,
}

/// 部署前预检所得到的下载、磁盘和依赖预算。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentEstimate {
    pub(super) checked_at: String,
    pub(super) model_download_bytes: Option<u64>,
    pub(super) runtime_download_bytes: Option<u64>,
    pub(super) runtime_packages: Vec<RuntimePackage>,
    pub(super) disk_checks: Vec<DiskCheck>,
    pub(super) requirements: Vec<String>,
}

/// 受管运行时固定资源清单项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimePackage {
    pub(super) name: String,
    pub(super) size_bytes: u64,
    pub(super) checksum: String,
    pub(super) origin: String,
    pub(super) download_required: bool,
}

/// 统一模型和运行时文件系统的磁盘预检结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiskCheck {
    pub(super) storage: String,
    pub(super) required_bytes: u64,
    pub(super) available_bytes: Option<u64>,
    pub(super) sufficient: Option<bool>,
}

/// 已启动部署的归属证明，不包含裸 PID 或路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentOwnership {
    pub(super) deployment_key: String,
    pub(super) model_id: String,
    pub(super) generation: Option<u64>,
    pub(super) route_attachment_id: Option<i32>,
    pub(super) created_by_task: bool,
    pub(super) originating_session: Uuid,
    pub(super) runtime_fingerprint: String,
}

/// 查询时使用的临时部署视图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentView {
    pub(super) model_id: String,
    pub(super) generation: Option<u64>,
    pub(super) route_attachment_id: Option<i32>,
    pub(super) process_ready: bool,
    pub(super) route_status: Option<String>,
    pub(super) observed_at: Option<String>,
}

/// 持久化的本机模型部署任务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeploymentTask {
    pub(super) task_id: Uuid,
    pub(super) owner_id: Uuid,
    pub(super) session_id: Uuid,
    pub(super) request_key: String,
    pub(super) command: DeploymentCommand,
    pub(super) status: TaskStatus,
    pub(super) stage: DeploymentStage,
    pub(super) effective_backend: Option<EffectiveBackend>,
    pub(super) backend_reason: Option<String>,
    pub(super) cancel_requested: bool,
    pub(super) model_download: PreparationProgress,
    pub(super) runtime: RuntimeProgress,
    pub(super) estimate: Option<DeploymentEstimate>,
    pub(super) deployment: Option<DeploymentOwnership>,
    pub(super) error: Option<TaskError>,
    pub(super) cleanup_error: Option<TaskError>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) finished_at: Option<String>,
    pub(super) row_version: i64,
}

impl DeploymentTask {
    pub(super) fn deployment_view(&self) -> Option<DeploymentView> {
        self.deployment.as_ref().map(|ownership| DeploymentView {
            model_id: ownership.model_id.clone(),
            generation: ownership.generation,
            route_attachment_id: ownership.route_attachment_id,
            process_ready: false,
            route_status: None,
            observed_at: None,
        })
    }
}

pub(super) fn decode_json<T>(value: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    serde_json::from_str(value).map_err(|error| error.to_string())
}

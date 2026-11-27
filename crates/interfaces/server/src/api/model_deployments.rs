//! 本机目录模型异步部署任务 HTTP 接口。

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use burncloud_download::DownloadManager;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path as FsPath;

use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::{
    model_deployment::{
        DeploymentCommand, DeploymentStage, DeploymentTask, EffectiveBackend, ModelSource,
        RequestedBackend, TaskError, TaskStatus,
    },
    model_deployment_store::DeploymentStore,
    node_deployment::DeploymentSnapshot,
    AppState,
};

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const LOCAL_OWNER_ID: Uuid = Uuid::from_u128(1);

/// 恢复服务上次会话遗留的部署任务。
pub(crate) async fn recover_incomplete_deployments(
    db: std::sync::Arc<burncloud_database::Database>,
    session_id: Uuid,
) -> Result<(), String> {
    let store = DeploymentStore::new(db);
    let interrupted = store
        .interrupt_incomplete(LOCAL_OWNER_ID, session_id)
        .await
        .map_err(|error| error.message)?;
    if !interrupted.is_empty() {
        tracing::warn!(count = interrupted.len(), "服务重启后中断未完成部署任务");
    }
    Ok(())
}

/// 构建本机模型异步部署路由。
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/node/models/deployments", post(submit_deployment))
        .route(
            "/api/node/models/deployments/{task_id}",
            get(get_deployment).delete(cancel_deployment),
        )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitRequest {
    model: String,
    source: String,
    revision: String,
    variant_id: String,
    checksum: String,
    #[serde(default = "default_backend")]
    backend: String,
}

#[derive(Debug, Serialize)]
struct Envelope<T> {
    success: bool,
    data: T,
}

#[derive(Debug, Serialize)]
struct SubmitEnvelope<T> {
    success: bool,
    data: T,
    reused: bool,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    success: bool,
    error: ApiError,
}

#[derive(Debug, Serialize)]
struct ApiError {
    code: &'static str,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct DeploymentData {
    task_id: String,
    status: String,
    model: String,
    source: String,
    revision: String,
    variant_id: String,
    backend: String,
    stage: String,
    cancel_requested: bool,
    model_download: BranchData,
    runtime: BranchData,
    deployment: Option<DeploymentDataView>,
    ready_to_chat: bool,
    error: Option<TaskErrorData>,
    cleanup_error: Option<TaskErrorData>,
    created_at: String,
    updated_at: String,
    finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct BranchData {
    status: String,
    stage: Option<String>,
    detail: Option<String>,
    task_id: Option<String>,
    completed_bytes: u64,
    total_bytes: Option<u64>,
    download_speed: u64,
    actual_sha256: Option<String>,
    error: Option<TaskErrorData>,
}

#[derive(Debug, Serialize)]
struct DeploymentDataView {
    model_id: String,
    generation: Option<u64>,
    route_attachment_id: Option<i32>,
    process_ready: bool,
    route_status: Option<String>,
    observed_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct TaskErrorData {
    code: String,
    message: String,
}

#[expect(
    clippy::cognitive_complexity,
    reason = "提交边界必须在一次原子流程中完成严格解析、幂等登记和后台调度"
)]
async fn submit_deployment(State(state): State<AppState>, body: Bytes) -> Response {
    if body.len() > MAX_REQUEST_BYTES {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "部署请求体超过 16 KiB",
        );
    }
    let request = match serde_json::from_slice::<SubmitRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            tracing::debug!(%error, "本机部署请求体无效");
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_deployment_request",
                "部署请求字段无效",
            );
        }
    };
    let command = match validate_request(request) {
        Ok(command) => command,
        Err(message) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_deployment_request",
                message,
            )
        }
    };
    if let Some((code, message)) = unsupported_backend_preflight(command.backend) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, code, message);
    }
    let store = DeploymentStore::new(state.db.clone());
    let (task, reused) = match store
        .insert_or_join(LOCAL_OWNER_ID, state.deployment_session_id, &command)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(code = %error.code, "登记本机部署任务失败");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "deployment_storage_failed",
                "部署任务状态暂时无法保存",
            );
        }
    };
    if !reused {
        spawn_deployment_worker(state.clone(), task.clone());
    }
    let mut response = no_store(
        (
            StatusCode::ACCEPTED,
            Json(SubmitEnvelope {
                success: true,
                data: task_data(&state, &task).await,
                reused,
            }),
        )
            .into_response(),
    );
    if let Ok(value) =
        HeaderValue::from_str(&format!("/api/node/models/deployments/{}", task.task_id))
    {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

async fn get_deployment(State(state): State<AppState>, Path(task_id): Path<String>) -> Response {
    let task_id = match Uuid::parse_str(task_id.trim()) {
        Ok(task_id) if !task_id.is_nil() => task_id,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_deployment_task_id",
                "任务 ID 必须是 UUID",
            )
        }
    };
    let store = DeploymentStore::new(state.db.clone());
    let task = match store.load(LOCAL_OWNER_ID, task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "deployment_task_not_found",
                "部署任务不存在",
            )
        }
        Err(error) => {
            tracing::error!(code = %error.code, "读取本机部署任务失败");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "deployment_storage_failed",
                "部署任务状态暂时无法读取",
            );
        }
    };
    let mut response = no_store(
        Json(Envelope {
            success: true,
            data: task_data(&state, &task).await,
        })
        .into_response(),
    );
    if !task.status.is_terminal() {
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static("1"));
    }
    response
}

#[expect(
    clippy::cognitive_complexity,
    reason = "取消边界必须在一次 CAS 流程中区分所有权、终态和并发冲突"
)]
async fn cancel_deployment(State(state): State<AppState>, Path(task_id): Path<String>) -> Response {
    let task_id = match Uuid::parse_str(task_id.trim()) {
        Ok(task_id) if !task_id.is_nil() => task_id,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_deployment_task_id",
                "任务 ID 必须是 UUID",
            )
        }
    };
    let store = DeploymentStore::new(state.db.clone());
    let current = match store.load(LOCAL_OWNER_ID, task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "deployment_task_not_found",
                "部署任务不存在",
            )
        }
        Err(error) => {
            tracing::error!(code = %error.code, "读取待取消部署任务失败");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "deployment_storage_failed",
                "部署任务状态暂时无法读取",
            );
        }
    };
    if current.status == TaskStatus::Cancelled {
        return no_store(
            Json(Envelope {
                success: true,
                data: task_data(&state, &current).await,
            })
            .into_response(),
        );
    }
    if current.status.is_terminal() {
        return error_response(
            StatusCode::CONFLICT,
            "deployment_task_not_cancellable",
            "部署任务已经结束",
        );
    }
    let mut next = current.clone();
    next.cancel_requested = true;
    next.updated_at = now();
    next.row_version += 1;
    match store.compare_update(&current, &next).await {
        Ok(Some(updated)) => {
            let status = StatusCode::ACCEPTED;
            no_store(
                (
                    status,
                    Json(Envelope {
                        success: true,
                        data: task_data(&state, &updated).await,
                    }),
                )
                    .into_response(),
            )
        }
        Ok(None) => error_response(
            StatusCode::CONFLICT,
            "deployment_task_not_cancellable",
            "部署任务状态已变化，请重新查询",
        ),
        Err(error) => {
            tracing::error!(code = %error.code, "保存部署取消意图失败");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "deployment_storage_failed",
                "部署取消状态暂时无法保存",
            )
        }
    }
}

fn default_backend() -> String {
    "auto".to_string()
}

fn runtime_failure_details(error: &str) -> Option<(&'static str, &'static str)> {
    let details = error.strip_prefix("runtime preparation failed: ")?;
    let (code, _) = details.split_once(": ")?;
    match code {
        "unsupported_platform" => Some((
            "unsupported_platform",
            "当前平台没有受支持的 llama.cpp 运行时",
        )),
        "unsupported_backend" => Some((
            "unsupported_backend",
            "当前固定运行时来源不支持请求的 backend",
        )),
        "runtime_configuration_invalid" => Some((
            "runtime_configuration_invalid",
            "运行时请求或服务端配置无效",
        )),
        "runtime_manifest_invalid" => {
            Some(("runtime_manifest_invalid", "服务端固定运行时清单无效"))
        }
        "runtime_cache_invalid" => Some(("runtime_cache_invalid", "运行时缓存不可用")),
        "runtime_download_failed" => Some(("runtime_download_failed", "运行时归档下载失败")),
        "runtime_source_unavailable" => {
            Some(("runtime_source_unavailable", "固定 Gitee 运行时资源不可用"))
        }
        "runtime_size_mismatch" => Some(("runtime_size_mismatch", "运行时归档长度校验失败")),
        "runtime_checksum_mismatch" => {
            Some(("runtime_checksum_mismatch", "运行时归档 SHA-256 校验失败"))
        }
        "runtime_archive_invalid" => {
            Some(("runtime_archive_invalid", "运行时归档不满足安全解压要求"))
        }
        "runtime_archive_io" => Some(("runtime_archive_io", "运行时归档处理失败")),
        "runtime_probe_failed" => Some(("runtime_probe_failed", "llama-server 运行时探测失败")),
        "runtime_device_unavailable" => {
            Some(("runtime_device_unavailable", "请求的运行时设备不可用"))
        }
        "runtime_install_interrupted" => {
            Some(("runtime_install_interrupted", "运行时准备作业已中断"))
        }
        "cancelled" => Some(("cancelled", "运行时准备已取消")),
        _ => None,
    }
}

fn unsupported_backend_preflight(
    backend: RequestedBackend,
) -> Option<(&'static str, &'static str)> {
    match backend {
        RequestedBackend::Cuda => Some((
            "unsupported_backend",
            "当前固定 Gitee 运行时来源没有 CUDA 资源",
        )),
        RequestedBackend::Auto | RequestedBackend::Cpu => None,
    }
}

fn validate_request(request: SubmitRequest) -> Result<DeploymentCommand, &'static str> {
    let source = ModelSource::parse(request.source.trim()).ok_or("模型来源不受支持")?;
    let backend = RequestedBackend::parse(request.backend.trim()).ok_or("部署后端不受支持")?;
    let model = request.model.trim().to_owned();
    let revision = request.revision.trim().to_owned();
    let variant_id = request.variant_id.trim().to_owned();
    let checksum = normalize_checksum(&request.checksum).ok_or("模型摘要格式无效")?;
    if !valid_model_id(&model) || !valid_segment(&revision) || !valid_variant_id(&variant_id) {
        return Err("模型、修订号或量化版本标识无效");
    }
    Ok(DeploymentCommand {
        model,
        source,
        revision,
        variant_id,
        checksum,
        backend,
    })
}

#[allow(
    clippy::large_futures,
    reason = "后台部署任务需要串联下载、摘要校验和本机运行时生命周期"
)]
fn spawn_deployment_worker(state: AppState, task: DeploymentTask) {
    tokio::spawn(async move {
        let store = DeploymentStore::new(state.db.clone());
        if let Err(error) = run_deployment(&state, &store, task.clone()).await {
            tracing::error!(%error, task_id = %task.task_id, "本机部署后台任务失败");
            let (code, message) = runtime_failure_details(&error).unwrap_or((
                "deployment_failed",
                "自动部署执行失败，请检查服务日志后重试",
            ));
            if let Err(finalize_error) =
                finalize_unhandled_failure(&store, task.task_id, code, message).await
            {
                tracing::error!(
                    %finalize_error,
                    task_id = %task.task_id,
                    "本机部署失败后未能持久化终态"
                );
            }
        }
    });
}

#[expect(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "部署执行器必须按下载、校验、启动、路由和终态提交顺序保留单一 CAS 流程"
)]
async fn run_deployment(
    state: &AppState,
    store: &DeploymentStore,
    task: DeploymentTask,
) -> Result<(), String> {
    let current = store
        .load(LOCAL_OWNER_ID, task.task_id)
        .await
        .map_err(|error| error.message.clone())?
        .ok_or_else(|| "部署任务不存在".to_string())?;
    let mut running = current.clone();
    running.status = TaskStatus::Running;
    running.stage = DeploymentStage::DownloadingModel;
    running.model_download.status = TaskStatus::Running;
    running.model_download.stage = Some(DeploymentStage::DownloadingModel);
    running.updated_at = now();
    running.row_version += 1;
    let Some(mut current) = store
        .compare_update(&current, &running)
        .await
        .map_err(|error| error.message.clone())?
    else {
        return Ok(());
    };

    let manager = DownloadManager::with_database(state.db.clone())
        .await
        .map_err(|error| error.to_string())?;
    let uri = artifact_url(
        current.command.source,
        &current.command.model,
        &current.command.revision,
        &current.command.variant_id,
    )?;
    let directory = storage_directory(&current.command);
    let handle = manager
        .start_download(&uri, &directory)
        .await
        .map_err(|error| error.to_string())?;
    manager
        .set_task_identity(
            handle.task_id,
            &current.command.model,
            current.command.source.as_str(),
            &current.command.revision,
            &current.command.variant_id,
            &uri,
            &current.command.checksum,
        )
        .await
        .map_err(|error| error.to_string())?;
    let path = manager
        .wait_for_task(handle.task_id)
        .await
        .map_err(|error| error.to_string())?;
    let actual = Box::pin(sha256_file(&path))
        .await
        .map_err(|error| error.to_string())?;
    if format!("sha256:{actual}") != current.command.checksum {
        return fail_task(
            store,
            &current,
            "checksum_mismatch",
            "模型文件摘要与目录凭据不一致",
        )
        .await;
    }
    let mut verified = current.clone();
    verified.model_download.status = TaskStatus::Succeeded;
    verified.model_download.stage = Some(DeploymentStage::Verifying);
    verified.model_download.task_id = Some(handle.task_id.to_string());
    verified.model_download.actual_sha256 = Some(actual);
    verified.model_download.detail = Some("模型文件已下载并完成校验".to_string());
    verified.stage = DeploymentStage::Starting;
    mark_runtime_preparing(&mut verified);
    current = store
        .compare_update(&current, &verified)
        .await
        .map_err(|error| error.message.clone())?
        .ok_or_else(|| "部署任务状态竞争失败".to_string())?;
    if current.cancel_requested {
        return cancel_task(store, &current).await;
    }

    let snapshot = match state.node_deployment.start_local(path).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let (code, message) = runtime_failure_details(&error)
                .unwrap_or(("process_start_failed", "模型运行时启动失败"));
            return fail_task(store, &current, code, message)
                .await
                .map_err(|_| error);
        }
    };
    let Some(current) = store
        .load(LOCAL_OWNER_ID, current.task_id)
        .await
        .map_err(|error| error.message.clone())?
    else {
        stop_runtime(state, &snapshot.model_id).await;
        return Err("部署任务在运行时启动后消失".to_string());
    };
    if current.cancel_requested {
        stop_runtime(state, &snapshot.model_id).await;
        return cancel_task(store, &current).await;
    }

    let mut attaching = current.clone();
    attaching.stage = DeploymentStage::AttachingRoute;
    attaching.runtime.status = TaskStatus::Succeeded;
    attaching.runtime.stage = Some(DeploymentStage::PreparingRuntime);
    attaching.effective_backend = Some(EffectiveBackend::Cpu);
    attaching.runtime.detail = Some("模型运行时已启动并通过健康检查，正在接入路由".to_string());
    attaching.deployment = Some(crate::model_deployment::DeploymentOwnership {
        deployment_key: current.request_key.clone(),
        model_id: snapshot.model_id.clone(),
        generation: Some(snapshot.generation),
        route_attachment_id: snapshot.route_attachment_id,
        created_by_task: true,
        originating_session: current.session_id,
        runtime_fingerprint: current.request_key.clone(),
    });
    attaching.updated_at = now();
    attaching.row_version += 1;
    let Some(mut current) = store
        .compare_update(&current, &attaching)
        .await
        .map_err(|error| error.message.clone())?
    else {
        stop_runtime(state, &snapshot.model_id).await;
        let latest = store
            .load(LOCAL_OWNER_ID, attaching.task_id)
            .await
            .map_err(|error| error.message.clone())?;
        if let Some(latest) =
            latest.filter(|task| task.cancel_requested && !task.status.is_terminal())
        {
            return cancel_task(store, &latest).await;
        }
        return Err("部署任务状态竞争失败".to_string());
    };

    let routable = match wait_for_routable(state, &snapshot.model_id).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let cleanup_error = stop_runtime(state, &snapshot.model_id).await;
            return fail_task_with_cleanup(
                store,
                &current,
                "route_attachment_failed",
                &error,
                cleanup_error,
            )
            .await;
        }
    };
    if let Some(deployment) = current.deployment.as_mut() {
        deployment.route_attachment_id = routable.route_attachment_id;
    }
    current.status = TaskStatus::Succeeded;
    current.finished_at = Some(now());
    current.updated_at = current.finished_at.clone().unwrap_or_else(now);
    current.row_version += 1;
    match store.compare_update(&attaching, &current).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => {
            stop_runtime(state, &snapshot.model_id).await;
            let latest = store
                .load(LOCAL_OWNER_ID, attaching.task_id)
                .await
                .map_err(|error| error.message.clone())?;
            if let Some(latest) =
                latest.filter(|task| task.cancel_requested && !task.status.is_terminal())
            {
                cancel_task(store, &latest).await
            } else {
                Err("部署任务状态竞争失败".to_string())
            }
        }
        Err(error) => Err(error.message.clone()),
    }
}

async fn wait_for_routable(state: &AppState, model_id: &str) -> Result<DeploymentSnapshot, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(snapshot) = state.node_deployment.status(model_id).await {
            if snapshot.route_status == "routable"
                && snapshot.route_attachment_id.is_some()
                && matches!(snapshot.status.as_str(), "ready" | "routable")
            {
                return Ok(snapshot);
            }
            if matches!(snapshot.status.as_str(), "failed" | "stopped")
                || matches!(
                    snapshot.route_status.as_str(),
                    "failed" | "detached" | "lost"
                )
            {
                return Err("模型运行时未能进入可路由状态".to_string());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("模型运行时在 30 秒内未能进入可路由状态".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn stop_runtime(state: &AppState, model_id: &str) -> Option<TaskError> {
    match state.node_deployment.stop(model_id).await {
        Ok(_) => None,
        Err(error) => {
            tracing::error!(model = %model_id, %error, "清理本机模型运行时失败");
            Some(TaskError {
                code: "cleanup_failed".to_string(),
                message: "本机模型运行时清理失败".to_string(),
            })
        }
    }
}

fn mark_runtime_preparing(task: &mut DeploymentTask) {
    task.runtime.status = TaskStatus::Running;
    task.runtime.stage = Some(DeploymentStage::PreparingRuntime);
    task.runtime.detail = Some("正在准备 llama.cpp 运行时".to_string());
    task.updated_at = now();
    task.row_version += 1;
}

async fn finalize_unhandled_failure(
    store: &DeploymentStore,
    task_id: Uuid,
    code: &str,
    message: &str,
) -> Result<(), String> {
    let Some(current) = store
        .load(LOCAL_OWNER_ID, task_id)
        .await
        .map_err(|error| error.message.clone())?
    else {
        return Err("部署任务不存在，无法收束后台错误".to_string());
    };
    if current.status.is_terminal() {
        return Ok(());
    }
    if current.cancel_requested {
        return cancel_task(store, &current).await;
    }
    fail_task(store, &current, code, message).await
}

async fn fail_task(
    store: &DeploymentStore,
    current: &DeploymentTask,
    code: &str,
    message: &str,
) -> Result<(), String> {
    fail_task_with_cleanup(store, current, code, message, None).await
}

async fn fail_task_with_cleanup(
    store: &DeploymentStore,
    current: &DeploymentTask,
    code: &str,
    message: &str,
    cleanup_error: Option<TaskError>,
) -> Result<(), String> {
    let mut failed = current.clone();
    failed.status = TaskStatus::Failed;
    failed.error = Some(TaskError {
        code: code.to_string(),
        message: message.to_string(),
    });
    if !failed.model_download.status.is_terminal() {
        failed.model_download.status = TaskStatus::Failed;
        if failed.model_download.stage.is_none() {
            failed.model_download.stage = Some(DeploymentStage::DownloadingModel);
        }
        failed.model_download.detail = Some(message.to_string());
        failed.model_download.error = Some(TaskError {
            code: code.to_string(),
            message: message.to_string(),
        });
    }
    if !failed.runtime.status.is_terminal() {
        failed.runtime.status = TaskStatus::Failed;
        if failed.runtime.stage.is_none() {
            failed.runtime.stage = Some(DeploymentStage::PreparingRuntime);
        }
        failed.runtime.detail = Some(message.to_string());
        failed.runtime.error = Some(TaskError {
            code: code.to_string(),
            message: message.to_string(),
        });
    }
    failed.cleanup_error = cleanup_error;
    failed.finished_at = Some(now());
    failed.updated_at = failed.finished_at.clone().unwrap_or_else(now);
    failed.row_version += 1;
    match store.compare_update(current, &failed).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => {
            let latest = store
                .load(LOCAL_OWNER_ID, current.task_id)
                .await
                .map_err(|error| error.message.clone())?;
            if let Some(latest) =
                latest.filter(|task| task.cancel_requested && !task.status.is_terminal())
            {
                cancel_task(store, &latest).await
            } else {
                Err("部署任务状态竞争失败".to_string())
            }
        }
        Err(error) => Err(error.message.clone()),
    }
}

async fn cancel_task(store: &DeploymentStore, current: &DeploymentTask) -> Result<(), String> {
    let mut cancelled = current.clone();
    cancelled.status = TaskStatus::Cancelled;
    cancelled.stage = current.stage;
    if !cancelled.model_download.status.is_terminal() {
        cancelled.model_download.status = TaskStatus::Cancelled;
        cancelled.model_download.detail = Some("部署任务已取消".to_string());
    }
    if !cancelled.runtime.status.is_terminal() {
        cancelled.runtime.status = TaskStatus::Cancelled;
        cancelled.runtime.detail = Some("部署任务已取消".to_string());
    }
    cancelled.deployment = None;
    cancelled.effective_backend = None;
    cancelled.backend_reason = None;
    cancelled.finished_at = Some(now());
    cancelled.updated_at = cancelled.finished_at.clone().unwrap_or_else(now);
    cancelled.row_version += 1;
    store
        .compare_update(current, &cancelled)
        .await
        .map_err(|error| error.message.clone())?
        .ok_or_else(|| "部署取消状态竞争失败".to_string())?;
    Ok(())
}

async fn sha256_file(path: &FsPath) -> Result<String, std::io::Error> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn task_data(state: &AppState, task: &DeploymentTask) -> DeploymentData {
    let deployment = if let Some(deployment) = task.deployment.as_ref() {
        let snapshot = state.node_deployment.status(&deployment.model_id).await;
        let process_ready = snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.status == "ready" || snapshot.status == "routable");
        Some(DeploymentDataView {
            model_id: deployment.model_id.clone(),
            generation: deployment.generation,
            route_attachment_id: deployment.route_attachment_id,
            process_ready,
            route_status: snapshot
                .as_ref()
                .map(|snapshot| snapshot.route_status.clone()),
            observed_at: snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.last_heartbeat_unix_ms)
                .map(|value| value.to_string()),
        })
    } else {
        None
    };
    DeploymentData {
        task_id: task.task_id.to_string(),
        status: task.status.as_str().to_string(),
        model: task.command.model.clone(),
        source: task.command.source.as_str().to_string(),
        revision: task.command.revision.clone(),
        variant_id: task.command.variant_id.clone(),
        backend: task.command.backend.as_str().to_string(),
        stage: task.stage.as_str().to_string(),
        cancel_requested: task.cancel_requested,
        model_download: branch_data(&task.model_download),
        runtime: runtime_data(&task.runtime),
        ready_to_chat: task.status == TaskStatus::Succeeded
            && deployment.as_ref().is_some_and(|deployment| {
                deployment.process_ready && deployment.route_status.as_deref() == Some("routable")
            }),
        deployment,
        error: task.error.as_ref().map(error_data),
        cleanup_error: task.cleanup_error.as_ref().map(error_data),
        created_at: task.created_at.clone(),
        updated_at: task.updated_at.clone(),
        finished_at: task.finished_at.clone(),
    }
}

fn branch_data(progress: &crate::model_deployment::PreparationProgress) -> BranchData {
    BranchData {
        status: progress.status.as_str().to_string(),
        stage: progress.stage.map(|stage| stage.as_str().to_string()),
        detail: progress.detail.clone(),
        task_id: progress.task_id.clone(),
        completed_bytes: progress.completed_bytes,
        total_bytes: progress.total_bytes,
        download_speed: progress.download_speed,
        actual_sha256: progress.actual_sha256.clone(),
        error: progress.error.as_ref().map(error_data),
    }
}
fn runtime_data(progress: &crate::model_deployment::RuntimeProgress) -> BranchData {
    BranchData {
        status: progress.status.as_str().to_string(),
        stage: progress.stage.map(|stage| stage.as_str().to_string()),
        detail: progress.detail.clone(),
        task_id: progress.task_id.clone(),
        completed_bytes: progress.completed_bytes,
        total_bytes: progress.total_bytes,
        download_speed: progress.download_speed,
        actual_sha256: progress.actual_sha256.clone(),
        error: progress.error.as_ref().map(error_data),
    }
}
fn error_data(error: &TaskError) -> TaskErrorData {
    TaskErrorData {
        code: error.code.clone(),
        message: error.message.clone(),
    }
}

fn artifact_url(
    source: ModelSource,
    model: &str,
    revision: &str,
    variant: &str,
) -> Result<String, String> {
    let (owner, name) = model
        .split_once('/')
        .ok_or_else(|| "模型名称无效".to_string())?;
    let base = match source {
        ModelSource::ModelScope => "https://modelscope.cn",
        ModelSource::HfMirror => "https://hf-mirror.com",
        ModelSource::HuggingFace => "https://huggingface.co",
    };
    let prefix = if source == ModelSource::ModelScope {
        "/models"
    } else {
        ""
    };
    Ok(format!(
        "{base}{prefix}/{owner}/{name}/resolve/{revision}/{variant}"
    ))
}
fn storage_directory(command: &DeploymentCommand) -> String {
    format!(
        "models/{}/{}/{}/{}",
        command.source.as_str(),
        command.model.replace('/', "_"),
        command.revision,
        command.checksum.trim_start_matches("sha256:")
    )
}
fn normalize_checksum(value: &str) -> Option<String> {
    let digest = value.trim().strip_prefix("sha256:")?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("sha256:{}", digest.to_ascii_lowercase()))
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
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
fn error_response(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    no_store(
        (
            status,
            Json(ErrorEnvelope {
                success: false,
                error: ApiError { code, message },
            }),
        )
            .into_response(),
    )
}

#[cfg(test)]
#[path = "model_deployments_tests.rs"]
mod tests;

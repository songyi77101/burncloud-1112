#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "测试只使用固定的本地序列化输入"
)]

use super::*;
use burncloud_database::{create_database_with_url, sqlite_url};
use serde_json::json;
use std::sync::Arc;
use tempfile::NamedTempFile;
use uuid::Uuid;

#[test]
fn submit_response_keeps_reused_at_the_top_level() {
    let value = serde_json::to_value(SubmitEnvelope {
        success: true,
        data: json!({"task_id": "task-1"}),
        reused: true,
    })
    .expect("固定响应应可序列化");

    assert_eq!(value["success"], true);
    assert_eq!(value["reused"], true);
    assert_eq!(value["data"]["task_id"], "task-1");
}

#[test]
fn missing_backend_defaults_to_auto() {
    let request: SubmitRequest = serde_json::from_value(json!({
        "model": "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
        "source": "hf-mirror",
        "revision": "main",
        "variant_id": "qwen2.5-0.5b-instruct-q4_k_m.gguf",
        "checksum": "sha256:74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db"
    }))
    .expect("backend 缺省时应使用 auto");

    let command = validate_request(request).expect("固定凭据应通过校验");
    assert_eq!(command.backend, RequestedBackend::Auto);
}

#[test]
fn cuda_backend_preflight_rejects_request_without_falling_back_to_cpu() {
    assert_eq!(
        unsupported_backend_preflight(RequestedBackend::Cuda),
        Some((
            "unsupported_backend",
            "当前固定 Gitee 运行时来源没有 CUDA 资源",
        ))
    );
    assert_eq!(unsupported_backend_preflight(RequestedBackend::Auto), None);
    assert_eq!(unsupported_backend_preflight(RequestedBackend::Cpu), None);
}

#[test]
fn runtime_preparation_errors_keep_stable_codes_without_exposing_diagnostics() {
    assert_eq!(
        runtime_failure_details(
            "runtime preparation failed: runtime_checksum_mismatch: local path secret",
        ),
        Some(("runtime_checksum_mismatch", "运行时归档 SHA-256 校验失败",))
    );
    assert_eq!(
        runtime_failure_details("runtime preparation failed: runtime_archive_io: disk error"),
        Some(("runtime_archive_io", "运行时归档处理失败"))
    );
    assert_eq!(
        runtime_failure_details("runtime preparation failed: unknown: detail"),
        None
    );
}

#[tokio::test]
async fn runtime_preparation_failure_is_persisted_as_a_terminal_deployment() {
    let database_file = NamedTempFile::new().expect("创建临时数据库文件");
    let database_url = sqlite_url(&database_file.path().to_string_lossy(), cfg!(windows), true);
    let database = Arc::new(
        create_database_with_url(&database_url)
            .await
            .expect("初始化部署测试数据库"),
    );
    let store = DeploymentStore::new(database);
    let command = DeploymentCommand {
        model: "Qwen/Qwen2.5-1.5B-Instruct".to_string(),
        source: ModelSource::HuggingFace,
        revision: "main".to_string(),
        variant_id: "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf".to_string(),
        checksum: format!("sha256:{}", "a".repeat(64)),
        backend: RequestedBackend::Auto,
    };
    let (queued, _) = store
        .insert_or_join(LOCAL_OWNER_ID, Uuid::new_v4(), &command)
        .await
        .expect("登记部署任务");

    let mut downloading = queued.clone();
    downloading.status = TaskStatus::Running;
    downloading.stage = DeploymentStage::DownloadingModel;
    downloading.model_download.status = TaskStatus::Running;
    downloading.model_download.stage = Some(DeploymentStage::DownloadingModel);
    downloading.updated_at = now();
    downloading.row_version += 1;
    let downloading = store
        .compare_update(&queued, &downloading)
        .await
        .expect("写入模型下载状态")
        .expect("模型下载状态未发生竞争");

    let mut starting = downloading.clone();
    starting.model_download.status = TaskStatus::Succeeded;
    starting.model_download.stage = Some(DeploymentStage::Verifying);
    starting.model_download.detail = Some("模型文件已下载并完成校验".to_string());
    starting.stage = DeploymentStage::Starting;
    starting.updated_at = now();
    starting.row_version += 1;
    let starting = store
        .compare_update(&downloading, &starting)
        .await
        .expect("写入运行时启动状态")
        .expect("运行时启动状态未发生竞争");

    let mut preparing_runtime = starting.clone();
    mark_runtime_preparing(&mut preparing_runtime);
    let starting = store
        .compare_update(&starting, &preparing_runtime)
        .await
        .expect("写入运行时准备状态")
        .expect("运行时准备状态未发生竞争");
    assert_eq!(starting.runtime.status, TaskStatus::Running);
    assert_eq!(
        starting.runtime.stage,
        Some(DeploymentStage::PreparingRuntime)
    );

    fail_task(&store, &starting, "runtime_probe_failed", "运行时探测失败")
        .await
        .expect("运行时准备失败应能持久化终态");

    let failed = store
        .load(LOCAL_OWNER_ID, starting.task_id)
        .await
        .expect("读取部署终态")
        .expect("部署记录仍存在");
    assert_eq!(failed.status, TaskStatus::Failed);
    assert_eq!(failed.runtime.status, TaskStatus::Failed);
    assert_eq!(
        failed
            .runtime
            .error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("runtime_probe_failed")
    );
    assert!(failed.finished_at.is_some());
}

#[tokio::test]
async fn startup_recovery_interrupts_previous_session_tasks() {
    let database_file = NamedTempFile::new().expect("创建临时数据库文件");
    let database_url = sqlite_url(&database_file.path().to_string_lossy(), cfg!(windows), true);
    let database = Arc::new(
        create_database_with_url(&database_url)
            .await
            .expect("初始化部署测试数据库"),
    );
    let store = DeploymentStore::new(database.clone());
    let previous_session = Uuid::new_v4();
    let next_session = Uuid::new_v4();
    let command = DeploymentCommand {
        model: "Qwen/Qwen2.5-1.5B-Instruct".to_string(),
        source: ModelSource::HuggingFace,
        revision: "main".to_string(),
        variant_id: "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf".to_string(),
        checksum: format!("sha256:{}", "b".repeat(64)),
        backend: RequestedBackend::Auto,
    };
    let (queued, _) = store
        .insert_or_join(LOCAL_OWNER_ID, previous_session, &command)
        .await
        .expect("登记上次会话部署任务");
    let mut running = queued.clone();
    running.status = TaskStatus::Running;
    running.stage = DeploymentStage::Starting;
    running.model_download.status = TaskStatus::Succeeded;
    running.model_download.stage = Some(DeploymentStage::Verifying);
    running.runtime.status = TaskStatus::Running;
    running.runtime.stage = Some(DeploymentStage::PreparingRuntime);
    running.updated_at = now();
    running.row_version += 1;
    store
        .compare_update(&queued, &running)
        .await
        .expect("写入上次会话运行态")
        .expect("上次会话运行态未发生竞争");

    recover_incomplete_deployments(database, next_session)
        .await
        .expect("启动恢复应成功");

    let recovered = store
        .load(LOCAL_OWNER_ID, queued.task_id)
        .await
        .expect("读取恢复后的任务")
        .expect("恢复任务仍存在");
    assert_eq!(recovered.status, TaskStatus::Interrupted);
    assert_eq!(recovered.runtime.status, TaskStatus::Interrupted);
    assert_eq!(
        recovered.error.as_ref().map(|error| error.code.as_str()),
        Some("service_restarted")
    );
    assert!(recovered.finished_at.is_some());
}

#[tokio::test]
async fn early_deployment_failure_closes_all_nonterminal_branches() {
    let database_file = NamedTempFile::new().expect("创建临时数据库文件");
    let database_url = sqlite_url(&database_file.path().to_string_lossy(), cfg!(windows), true);
    let database = Arc::new(
        create_database_with_url(&database_url)
            .await
            .expect("初始化部署测试数据库"),
    );
    let store = DeploymentStore::new(database);
    let command = DeploymentCommand {
        model: "Qwen/Qwen2.5-1.5B-Instruct".to_string(),
        source: ModelSource::HuggingFace,
        revision: "main".to_string(),
        variant_id: "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf".to_string(),
        checksum: format!("sha256:{}", "c".repeat(64)),
        backend: RequestedBackend::Auto,
    };
    let (queued, _) = store
        .insert_or_join(LOCAL_OWNER_ID, Uuid::new_v4(), &command)
        .await
        .expect("登记部署任务");

    let mut running = queued.clone();
    running.status = TaskStatus::Running;
    running.stage = DeploymentStage::DownloadingModel;
    running.model_download.status = TaskStatus::Running;
    running.model_download.stage = Some(DeploymentStage::DownloadingModel);
    running.updated_at = now();
    running.row_version += 1;
    let running = store
        .compare_update(&queued, &running)
        .await
        .expect("写入模型下载状态")
        .expect("模型下载状态未发生竞争");

    finalize_unhandled_failure(
        &store,
        running.task_id,
        "deployment_failed",
        "模拟后台准备失败",
    )
    .await
    .expect("后台失败应能写入终态");

    let failed = store
        .load(LOCAL_OWNER_ID, running.task_id)
        .await
        .expect("读取部署终态")
        .expect("部署记录仍存在");
    assert_eq!(failed.status, TaskStatus::Failed);
    assert_eq!(failed.model_download.status, TaskStatus::Failed);
    assert_eq!(failed.runtime.status, TaskStatus::Failed);
    assert!(failed.finished_at.is_some());
}

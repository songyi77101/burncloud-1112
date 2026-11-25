#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "仓储测试使用临时数据库；测试前提失败直接终止能保留最短诊断路径。"
)]

use super::*;
use burncloud_database::{create_database_with_url, sqlite_url};
use std::sync::Arc;
use tempfile::NamedTempFile;

async fn database() -> (NamedTempFile, Arc<Database>) {
    let file = NamedTempFile::new().expect("创建临时数据库文件");
    let url = sqlite_url(&file.path().to_string_lossy(), cfg!(windows), true);
    let database = create_database_with_url(&url)
        .await
        .expect("初始化临时数据库");
    (file, Arc::new(database))
}

fn command() -> DeploymentCommand {
    DeploymentCommand {
        model: "Qwen/Qwen2.5-1.5B-Instruct".to_string(),
        source: ModelSource::HuggingFace,
        revision: "main".to_string(),
        variant_id: "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf".to_string(),
        checksum: format!("sha256:{}", "a".repeat(64)),
        backend: RequestedBackend::Auto,
    }
}

fn running_task(mut task: DeploymentTask) -> DeploymentTask {
    task.status = TaskStatus::Running;
    task.stage = DeploymentStage::PreparingRuntime;
    task.model_download.status = TaskStatus::Running;
    task.model_download.stage = Some(DeploymentStage::DownloadingModel);
    task.runtime.status = TaskStatus::Running;
    task.runtime.stage = Some(DeploymentStage::PreparingRuntime);
    task.updated_at = now();
    task.row_version += 1;
    task
}

fn succeeded_task(mut task: DeploymentTask) -> DeploymentTask {
    task.status = TaskStatus::Succeeded;
    task.stage = DeploymentStage::AttachingRoute;
    task.effective_backend = Some(EffectiveBackend::Cpu);
    task.deployment = Some(ownership(task.session_id));
    task.model_download.status = TaskStatus::Succeeded;
    task.model_download.stage = Some(DeploymentStage::Verifying);
    task.runtime.status = TaskStatus::Succeeded;
    task.runtime.stage = Some(DeploymentStage::PreparingRuntime);
    task.updated_at = now();
    task.finished_at = Some(task.updated_at.clone());
    task.row_version += 1;
    task
}

fn ownership(session: Uuid) -> DeploymentOwnership {
    DeploymentOwnership {
        deployment_key: "b".repeat(64),
        model_id: format!("bc{}", "1".repeat(32)),
        generation: Some(4),
        route_attachment_id: Some(9),
        created_by_task: true,
        originating_session: session,
        runtime_fingerprint: "c".repeat(64),
    }
}

#[tokio::test]
async fn concurrent_insert_or_join_returns_the_unique_active_winner() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let request = command();
    let (left, right) = tokio::join!(
        store.insert_or_join(owner, Uuid::new_v4(), &request),
        store.insert_or_join(owner, Uuid::new_v4(), &request),
    );
    let (left, left_reused) = left.expect("并发登记左请求");
    let (right, right_reused) = right.expect("并发登记右请求");

    assert_eq!(left.task_id, right.task_id);
    assert_ne!(left_reused, right_reused);
}

#[tokio::test]
async fn accepted_cancellation_wins_against_a_late_success() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let (queued, _) = store
        .insert_or_join(owner, Uuid::new_v4(), &command())
        .await
        .expect("插入任务");
    let running = running_task(queued.clone());
    assert!(store
        .compare_update(&queued, &running)
        .await
        .expect("开始任务")
        .is_some());

    let mut cancelling = running.clone();
    cancelling.cancel_requested = true;
    cancelling.updated_at = now();
    cancelling.row_version += 1;
    assert!(store
        .compare_update(&running, &cancelling)
        .await
        .expect("持久化取消意图")
        .is_some());

    let mut late_success = succeeded_task(cancelling.clone());
    late_success.cancel_requested = false;
    assert!(store
        .compare_update(&cancelling, &late_success)
        .await
        .expect("成功 CAS 遇到已接受取消应返回冲突")
        .is_none());

    let mut cancelled = cancelling.clone();
    cancelled.status = TaskStatus::Cancelled;
    cancelled.stage = DeploymentStage::PreparingRuntime;
    cancelled.model_download.status = TaskStatus::Cancelled;
    cancelled.runtime.status = TaskStatus::Cancelled;
    cancelled.error = None;
    cancelled.cleanup_error = None;
    cancelled.finished_at = Some(now());
    cancelled.updated_at = cancelled.finished_at.clone().expect("终态时间已设置");
    cancelled.row_version += 1;
    assert!(store
        .compare_update(&cancelling, &cancelled)
        .await
        .expect("提交取消终态")
        .is_some());
    assert_eq!(
        store
            .load(owner, queued.task_id)
            .await
            .expect("读取任务")
            .expect("任务存在")
            .status,
        TaskStatus::Cancelled
    );
}

#[tokio::test]
async fn corrupted_enum_and_integer_are_storage_errors() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let (task, _) = store
        .insert_or_join(owner, Uuid::new_v4(), &command())
        .await
        .expect("插入任务");
    let pool = database.get_connection().expect("数据库连接").pool();
    let mut connection = pool.acquire().await.expect("取得 SQLite 连接");
    burncloud_database::sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .expect("仅为损坏数据测试关闭 CHECK");
    burncloud_database::sqlx::query(
        "UPDATE sys_model_deployments SET source = ?, cancel_requested = ? WHERE owner_id = ? AND task_id = ?",
    )
    .bind("not-a-source")
    .bind(7_i32)
    .bind(owner.to_string())
    .bind(task.task_id.to_string())
    .execute(&mut *connection)
    .await
    .expect("写入损坏枚举和整数");
    drop(connection);

    let error = store
        .load(owner, task.task_id)
        .await
        .expect_err("损坏来源必须被拒绝");
    assert_eq!(error.code, "deployment_storage_failed");
    burncloud_database::sqlx::query(
        "UPDATE sys_model_deployments SET source = ? WHERE owner_id = ? AND task_id = ?",
    )
    .bind("huggingface")
    .bind(owner.to_string())
    .bind(task.task_id.to_string())
    .execute(pool)
    .await
    .expect("修复来源以覆盖布尔整数校验");
    let error = store
        .load(owner, task.task_id)
        .await
        .expect_err("非 0/1 取消位必须被拒绝");
    assert_eq!(error.code, "deployment_storage_failed");
}

#[tokio::test]
#[ignore = "需要 BURNCLOUD_TEST_POSTGRES_URL 和建库权限；必须显式运行以提供 PostgreSQL 仓储证据"]
async fn postgres_store_insert_join_cas_and_owner_isolation() {
    use burncloud_database::sqlx::{postgres::PgConnectOptions, ConnectOptions, Executor};
    use std::{str::FromStr, time::SystemTime};

    let server_url =
        std::env::var("BURNCLOUD_TEST_POSTGRES_URL").expect("显式提供 PostgreSQL 测试服务器 URL");
    let options = PgConnectOptions::from_str(&server_url).expect("PostgreSQL 测试 URL 有效");
    let database_name = format!(
        "bc_store_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("系统时间晚于 Unix epoch")
            .as_nanos()
    );
    let mut admin = options
        .clone()
        .database("postgres")
        .connect()
        .await
        .expect("连接 PostgreSQL 管理数据库");
    admin
        .execute(format!("CREATE DATABASE {database_name}").as_str())
        .await
        .expect("创建独立 PostgreSQL 测试数据库");
    let target_url = options.database(&database_name).to_url_lossy().to_string();
    let result = async {
        let database = create_database_with_url(&target_url)
            .await
            .expect("运行 PostgreSQL 迁移");
        let database = Arc::new(database);
        let store = DeploymentStore::new(database.clone());
        let owner = Uuid::new_v4();
        let other_owner = Uuid::new_v4();
        let (task, reused) = store
            .insert_or_join(owner, Uuid::new_v4(), &command())
            .await
            .expect("PostgreSQL 插入任务");
        assert!(!reused);
        let (joined, reused) = store
            .insert_or_join(owner, Uuid::new_v4(), &command())
            .await
            .expect("PostgreSQL 加入在途任务");
        assert!(reused);
        assert_eq!(task.task_id, joined.task_id);
        assert!(store
            .load(other_owner, task.task_id)
            .await
            .expect("PostgreSQL owner 隔离读取")
            .is_none());
        let running = running_task(task.clone());
        assert!(store
            .compare_update(&task, &running)
            .await
            .expect("PostgreSQL CAS 更新")
            .is_some());
        drop(store);
        drop(database);
        Ok::<(), String>(())
    }
    .await;
    let drop_result = admin
        .execute(format!("DROP DATABASE {database_name} WITH (FORCE)").as_str())
        .await;
    assert!(
        drop_result.is_ok(),
        "清理独立 PostgreSQL 测试库失败: {drop_result:?}"
    );
    assert!(result.is_ok(), "PostgreSQL 部署仓储测试失败: {result:?}");
}

#[tokio::test]
async fn insert_or_join_is_idempotent_only_while_task_is_active() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let session = Uuid::new_v4();

    let (first, reused) = store
        .insert_or_join(owner, session, &command())
        .await
        .expect("首次插入任务");
    assert!(!reused);
    let (joined, reused) = store
        .insert_or_join(owner, Uuid::new_v4(), &command())
        .await
        .expect("加入在途任务");
    assert!(reused);
    assert_eq!(joined.task_id, first.task_id);

    let terminal = succeeded_task(first.clone());
    assert!(store
        .compare_update(&first, &terminal)
        .await
        .expect("写入成功终态")
        .is_some());
    let (retry, reused) = store
        .insert_or_join(owner, session, &command())
        .await
        .expect("终态后新建任务");
    assert!(!reused);
    assert_ne!(retry.task_id, first.task_id);
}

#[tokio::test]
async fn load_and_join_are_isolated_by_owner() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let other_owner = Uuid::new_v4();
    let (task, _) = store
        .insert_or_join(owner, Uuid::new_v4(), &command())
        .await
        .expect("插入任务");

    assert!(store
        .load(other_owner, task.task_id)
        .await
        .expect("隔离读取")
        .is_none());
    let (other, reused) = store
        .insert_or_join(other_owner, Uuid::new_v4(), &command())
        .await
        .expect("其他所有者插入同请求");
    assert!(!reused);
    assert_ne!(other.task_id, task.task_id);
}

#[tokio::test]
async fn compare_update_rejects_stale_snapshot_and_preserves_newer_row() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let (queued, _) = store
        .insert_or_join(owner, Uuid::new_v4(), &command())
        .await
        .expect("插入任务");
    let running = running_task(queued.clone());
    assert!(store
        .compare_update(&queued, &running)
        .await
        .expect("CAS 更新")
        .is_some());

    let mut stale_next = queued.clone();
    stale_next.status = TaskStatus::Failed;
    stale_next.error = Some(TaskError {
        code: "model_download_failed".to_string(),
        message: "下载失败".to_string(),
    });
    stale_next.updated_at = now();
    stale_next.finished_at = Some(stale_next.updated_at.clone());
    stale_next.model_download.status = TaskStatus::Failed;
    stale_next.runtime.status = TaskStatus::Cancelled;
    stale_next.row_version += 1;

    assert!(store
        .compare_update(&queued, &stale_next)
        .await
        .expect("过期 CAS 不应是存储错误")
        .is_none());
    let loaded = store
        .load(owner, queued.task_id)
        .await
        .expect("读取最新任务")
        .expect("任务必须存在");
    assert_eq!(loaded.status, TaskStatus::Running);
    assert_eq!(loaded.row_version, running.row_version);
}

#[tokio::test]
async fn malformed_json_is_a_storage_error() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let (task, _) = store
        .insert_or_join(owner, Uuid::new_v4(), &command())
        .await
        .expect("插入任务");
    burncloud_database::sqlx::query(
        "UPDATE sys_model_deployments SET model_progress_json = ? WHERE owner_id = ? AND task_id = ?",
    )
    .bind("not-json")
    .bind(owner.to_string())
    .bind(task.task_id.to_string())
    .execute(database.get_connection().expect("数据库连接").pool())
    .await
    .expect("写入损坏 JSON");

    let error = store
        .load(owner, task.task_id)
        .await
        .expect_err("损坏 JSON 必须失败");
    assert_eq!(error.code, "deployment_storage_failed");
}

#[tokio::test]
async fn interrupt_incomplete_only_marks_old_owner_session_and_returns_proof() {
    let (_file, database) = database().await;
    let store = DeploymentStore::new(database.clone());
    let owner = Uuid::new_v4();
    let other_owner = Uuid::new_v4();
    let old_session = Uuid::new_v4();
    let new_session = Uuid::new_v4();
    let (old_task, _) = store
        .insert_or_join(owner, old_session, &command())
        .await
        .expect("插入旧任务");
    let mut running = running_task(old_task.clone());
    running.deployment = Some(ownership(old_session));
    assert!(store
        .compare_update(&old_task, &running)
        .await
        .expect("保存运行归属")
        .is_some());
    let (other_task, _) = store
        .insert_or_join(other_owner, old_session, &command())
        .await
        .expect("插入其他所有者任务");

    let proofs = store
        .interrupt_incomplete(owner, new_session)
        .await
        .expect("恢复旧任务");
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0].originating_session, old_session);
    let interrupted = store
        .load(owner, old_task.task_id)
        .await
        .expect("读取中断任务")
        .expect("中断任务存在");
    assert_eq!(interrupted.status, TaskStatus::Interrupted);
    assert_eq!(interrupted.model_download.status, TaskStatus::Interrupted);
    assert_eq!(interrupted.runtime.status, TaskStatus::Interrupted);
    assert_eq!(
        interrupted.error.as_ref().map(|error| error.code.as_str()),
        Some("service_restarted")
    );
    let other = store
        .load(other_owner, other_task.task_id)
        .await
        .expect("读取其他所有者任务")
        .expect("其他任务存在");
    assert_eq!(other.status, TaskStatus::Queued);
}

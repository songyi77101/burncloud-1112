use crate::model_deployment::{
    decode_json, DeploymentCommand, DeploymentEstimate, DeploymentOwnership, DeploymentStage,
    DeploymentTask, EffectiveBackend, ModelSource, PreparationProgress, RequestedBackend,
    RuntimeProgress, TaskError, TaskStatus,
};
use burncloud_database::sqlx::{any::AnyRow, Row};
use burncloud_database::{adapt_sql, Database};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

const SELECT_COLUMNS: &str = "task_id, owner_id, session_id, request_key, source, model, revision, variant_id, checksum, requested_backend, effective_backend, backend_reason, status, stage, cancel_requested, model_progress_json, runtime_progress_json, estimate_json, ownership_json, error_json, cleanup_error_json, row_version, created_at, updated_at, finished_at";

/// 已注入的部署任务仓储。
///
/// 仓储只借用应用已经初始化的数据库连接，不会根据配置重新打开数据库。
pub(crate) struct DeploymentStore {
    db: Arc<Database>,
}

#[derive(Serialize)]
struct RequestKeyMaterial<'a> {
    source: &'a str,
    model: &'a str,
    revision: &'a str,
    variant_id: &'a str,
    checksum: &'a str,
    requested_backend: &'a str,
}

impl DeploymentStore {
    /// 创建一个只使用现有共享连接的部署仓储。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let store = DeploymentStore::new(database);
    /// ```
    ///
    /// `database` 必须由应用组合根完成初始化；本方法不会创建第二个连接池。
    pub(super) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// 原子登记或加入当前所有者上相同请求的在途任务。
    ///
    /// # Errors
    ///
    /// 数据库不可用、任务命令不满足存储不变量或持久化快照损坏时返回
    /// `deployment_storage_failed`。
    pub(super) async fn insert_or_join(
        &self,
        owner: Uuid,
        session: Uuid,
        command: &DeploymentCommand,
    ) -> Result<(DeploymentTask, bool), TaskError> {
        validate_owner_session(owner, session)?;
        validate_command(command)?;

        let request_key = request_key(command)?;
        let task = new_task(owner, session, request_key.clone(), command.clone());
        let postgres = self.db.kind() == "postgres";
        let pool = self
            .db
            .get_connection()
            .map_err(|error| TaskError::storage(format!("无法获取部署数据库连接: {error}")))?
            .pool();
        let mut transaction = pool
            .begin()
            .await
            .map_err(|error| TaskError::storage(format!("无法开启部署事务: {error}")))?;

        let insert_sql = adapt_sql(
            postgres,
            "INSERT INTO sys_model_deployments (task_id, owner_id, session_id, request_key, source, model, revision, variant_id, checksum, requested_backend, effective_backend, backend_reason, status, stage, cancel_requested, model_progress_json, runtime_progress_json, estimate_json, ownership_json, error_json, cleanup_error_json, row_version, created_at, updated_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        );
        let model_progress_json = encode_json(&task.model_download)?;
        let runtime_progress_json = encode_json(&task.runtime)?;
        let result = burncloud_database::sqlx::query(&insert_sql)
            .bind(task.task_id.to_string())
            .bind(task.owner_id.to_string())
            .bind(task.session_id.to_string())
            .bind(&task.request_key)
            .bind(task.command.source.as_str())
            .bind(&task.command.model)
            .bind(&task.command.revision)
            .bind(&task.command.variant_id)
            .bind(&task.command.checksum)
            .bind(task.command.backend.as_str())
            .bind(None::<&str>)
            .bind(None::<&str>)
            .bind(task.status.as_str())
            .bind(task.stage.as_str())
            .bind(bool_to_i32(task.cancel_requested))
            .bind(model_progress_json)
            .bind(runtime_progress_json)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(task.row_version)
            .bind(&task.created_at)
            .bind(&task.updated_at)
            .bind(None::<&str>)
            .execute(&mut *transaction)
            .await
            .map_err(|error| TaskError::storage(format!("无法登记部署任务: {error}")))?;

        let (row, reused) = if result.rows_affected() == 1 {
            let select_sql = adapt_sql(
                postgres,
                &format!("SELECT {SELECT_COLUMNS} FROM sys_model_deployments WHERE owner_id = ? AND task_id = ?"),
            );
            let row = burncloud_database::sqlx::query(&select_sql)
                .bind(owner.to_string())
                .bind(task.task_id.to_string())
                .fetch_one(&mut *transaction)
                .await
                .map_err(|error| {
                    TaskError::storage(format!("无法读取刚登记的部署任务: {error}"))
                })?;
            (row, false)
        } else {
            let select_sql = adapt_sql(
                postgres,
                &format!("SELECT {SELECT_COLUMNS} FROM sys_model_deployments WHERE owner_id = ? AND request_key = ? AND status IN (?, ?)"),
            );
            let row = burncloud_database::sqlx::query(&select_sql)
                .bind(owner.to_string())
                .bind(&request_key)
                .bind(TaskStatus::Queued.as_str())
                .bind(TaskStatus::Running.as_str())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| TaskError::storage(format!("无法读取竞争中的部署任务: {error}")))?
                .ok_or_else(|| TaskError::storage("部署请求竞争后未找到在途任务"))?;
            (row, true)
        };

        let decoded = decode_task(&row)?;
        transaction
            .commit()
            .await
            .map_err(|error| TaskError::storage(format!("无法提交部署登记事务: {error}")))?;
        Ok((decoded, reused))
    }

    /// 按本机所有者和任务 UUID 读取一条部署记录。
    ///
    /// # Errors
    ///
    /// 任何查询错误或持久化字段无法严格解码时返回
    /// `deployment_storage_failed`；其他所有者的同 UUID 记录不会被返回。
    pub(super) async fn load(
        &self,
        owner: Uuid,
        task_id: Uuid,
    ) -> Result<Option<DeploymentTask>, TaskError> {
        if owner.is_nil() || task_id.is_nil() {
            return Err(TaskError::storage("部署所有者和任务 ID 不能为 nil UUID"));
        }
        let postgres = self.db.kind() == "postgres";
        let sql = adapt_sql(
            postgres,
            &format!("SELECT {SELECT_COLUMNS} FROM sys_model_deployments WHERE owner_id = ? AND task_id = ?"),
        );
        let row = burncloud_database::sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(task_id.to_string())
            .fetch_optional(
                self.db
                    .get_connection()
                    .map_err(|error| {
                        TaskError::storage(format!("无法获取部署数据库连接: {error}"))
                    })?
                    .pool(),
            )
            .await
            .map_err(|error| TaskError::storage(format!("无法读取部署任务: {error}")))?;
        row.as_ref().map(decode_task).transpose()
    }

    /// 以所有者、任务、请求键和行版本为条件提交完整快照。
    ///
    /// 返回 `None` 表示 CAS 冲突，调用方必须重新读取后再决定是否继续；该结果不是
    /// 存储错误，也不会覆盖已经进入终态的记录。
    pub(super) async fn compare_update(
        &self,
        expected: &DeploymentTask,
        next: &DeploymentTask,
    ) -> Result<Option<DeploymentTask>, TaskError> {
        validate_transition(expected, next)?;
        let postgres = self.db.kind() == "postgres";
        let model_progress_json = encode_json(&next.model_download)?;
        let runtime_progress_json = encode_json(&next.runtime)?;
        let estimate_json = encode_optional_json(next.estimate.as_ref())?;
        let ownership_json = encode_optional_json(next.deployment.as_ref())?;
        let error_json = encode_optional_json(next.error.as_ref())?;
        let cleanup_error_json = encode_optional_json(next.cleanup_error.as_ref())?;
        let success_cancel_guard = if next.status == TaskStatus::Succeeded {
            " AND cancel_requested = 0"
        } else {
            ""
        };
        let update_sql = format!(
            "UPDATE sys_model_deployments SET session_id = ?, source = ?, model = ?, revision = ?, variant_id = ?, checksum = ?, requested_backend = ?, effective_backend = ?, backend_reason = ?, status = ?, stage = ?, cancel_requested = ?, model_progress_json = ?, runtime_progress_json = ?, estimate_json = ?, ownership_json = ?, error_json = ?, cleanup_error_json = ?, row_version = ?, updated_at = ?, finished_at = ? WHERE owner_id = ? AND task_id = ? AND request_key = ? AND row_version = ? AND status = ?{success_cancel_guard}"
        );
        let sql = adapt_sql(postgres, &update_sql);
        let pool = self
            .db
            .get_connection()
            .map_err(|error| TaskError::storage(format!("无法获取部署数据库连接: {error}")))?
            .pool();
        let mut transaction = pool
            .begin()
            .await
            .map_err(|error| TaskError::storage(format!("无法开启部署更新事务: {error}")))?;
        let result = burncloud_database::sqlx::query(&sql)
            .bind(next.session_id.to_string())
            .bind(next.command.source.as_str())
            .bind(&next.command.model)
            .bind(&next.command.revision)
            .bind(&next.command.variant_id)
            .bind(&next.command.checksum)
            .bind(next.command.backend.as_str())
            .bind(next.effective_backend.map(EffectiveBackend::as_str))
            .bind(next.backend_reason.as_deref())
            .bind(next.status.as_str())
            .bind(next.stage.as_str())
            .bind(bool_to_i32(next.cancel_requested))
            .bind(model_progress_json)
            .bind(runtime_progress_json)
            .bind(estimate_json)
            .bind(ownership_json)
            .bind(error_json)
            .bind(cleanup_error_json)
            .bind(next.row_version)
            .bind(&next.updated_at)
            .bind(next.finished_at.as_deref())
            .bind(next.owner_id.to_string())
            .bind(next.task_id.to_string())
            .bind(&next.request_key)
            .bind(expected.row_version)
            .bind(expected.status.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(|error| TaskError::storage(format!("无法比较更新部署任务: {error}")))?;
        if result.rows_affected() == 0 {
            transaction.rollback().await.map_err(|error| {
                TaskError::storage(format!("无法回滚冲突的部署更新事务: {error}"))
            })?;
            return Ok(None);
        }
        transaction
            .commit()
            .await
            .map_err(|error| TaskError::storage(format!("无法提交部署更新事务: {error}")))?;
        Ok(Some(next.clone()))
    }

    /// 将当前所有者在旧会话中仍未完成的任务标记为服务重启中断。
    ///
    /// 只有能够从任务快照证明归属的部署才会出现在返回值中；不会根据 PID 或进程名
    /// 猜测需要清理的外部进程。
    pub(super) async fn interrupt_incomplete(
        &self,
        owner: Uuid,
        session: Uuid,
    ) -> Result<Vec<DeploymentOwnership>, TaskError> {
        validate_owner_session(owner, session)?;
        let postgres = self.db.kind() == "postgres";
        let sql = adapt_sql(
            postgres,
            &format!("SELECT {SELECT_COLUMNS} FROM sys_model_deployments WHERE owner_id = ? AND session_id <> ? AND status IN (?, ?) ORDER BY created_at"),
        );
        let rows = burncloud_database::sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session.to_string())
            .bind(TaskStatus::Queued.as_str())
            .bind(TaskStatus::Running.as_str())
            .fetch_all(
                self.db
                    .get_connection()
                    .map_err(|error| {
                        TaskError::storage(format!("无法获取部署数据库连接: {error}"))
                    })?
                    .pool(),
            )
            .await
            .map_err(|error| TaskError::storage(format!("无法读取待恢复部署任务: {error}")))?;

        let mut ownerships = Vec::new();
        for row in rows {
            let task = decode_task(&row)?;
            loop {
                let Some(current) = self.load(owner, task.task_id).await? else {
                    break;
                };
                if current.session_id == session || current.status.is_terminal() {
                    break;
                }
                let next = interrupted_task(&current);
                match self.compare_update(&current, &next).await? {
                    Some(committed) => {
                        if let Some(ownership) = committed.deployment {
                            ownerships.push(ownership);
                        }
                        break;
                    }
                    None => continue,
                }
            }
        }
        Ok(ownerships)
    }
}

fn validate_owner_session(owner: Uuid, session: Uuid) -> Result<(), TaskError> {
    if owner.is_nil() || session.is_nil() {
        return Err(TaskError::storage("部署所有者和会话 ID 不能为 nil UUID"));
    }
    Ok(())
}

fn validate_command(command: &DeploymentCommand) -> Result<(), TaskError> {
    if command.model.trim().is_empty()
        || command.revision.trim().is_empty()
        || command.variant_id.trim().is_empty()
        || command.model != command.model.trim()
        || command.revision != command.revision.trim()
        || command.variant_id != command.variant_id.trim()
    {
        return Err(TaskError::storage("部署命令包含空白或空身份字段"));
    }
    if !is_sha256(&command.checksum) {
        return Err(TaskError::storage("部署命令摘要不是规范的 sha256 摘要"));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        && hex == hex.to_ascii_lowercase()
}

fn is_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value == value.to_ascii_lowercase()
}

fn request_key(command: &DeploymentCommand) -> Result<String, TaskError> {
    let material = RequestKeyMaterial {
        source: command.source.as_str(),
        model: &command.model,
        revision: &command.revision,
        variant_id: &command.variant_id,
        checksum: &command.checksum,
        requested_backend: command.backend.as_str(),
    };
    let bytes = serde_json::to_vec(&material)
        .map_err(|error| TaskError::storage(format!("无法生成部署请求键: {error}")))?;
    Ok(hex_digest(Sha256::digest(bytes)))
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn new_task(
    owner_id: Uuid,
    session_id: Uuid,
    request_key: String,
    command: DeploymentCommand,
) -> DeploymentTask {
    let timestamp = now();
    DeploymentTask {
        task_id: Uuid::new_v4(),
        owner_id,
        session_id,
        request_key,
        command,
        status: TaskStatus::Queued,
        stage: DeploymentStage::Preflight,
        effective_backend: None,
        backend_reason: None,
        cancel_requested: false,
        model_download: PreparationProgress {
            status: TaskStatus::Queued,
            stage: None,
            detail: None,
            task_id: None,
            completed_bytes: 0,
            total_bytes: None,
            download_speed: 0,
            actual_sha256: None,
            error: None,
        },
        runtime: RuntimeProgress {
            status: TaskStatus::Queued,
            stage: None,
            detail: None,
            task_id: None,
            completed_bytes: 0,
            total_bytes: None,
            download_speed: 0,
            actual_sha256: None,
            error: None,
            origin: None,
            version: None,
        },
        estimate: None,
        deployment: None,
        error: None,
        cleanup_error: None,
        created_at: timestamp.clone(),
        updated_at: timestamp,
        finished_at: None,
        row_version: 0,
    }
}

fn interrupted_task(task: &DeploymentTask) -> DeploymentTask {
    let mut next = task.clone();
    let error = TaskError::service_restarted();
    next.status = TaskStatus::Interrupted;
    next.cancel_requested = false;
    next.error = Some(error.clone());
    next.cleanup_error = None;
    next.finished_at = Some(now());
    next.updated_at = next.finished_at.clone().unwrap_or_else(now);
    next.row_version = next.row_version.saturating_add(1);
    interrupt_progress(&mut next.model_download, &error);
    interrupt_runtime(&mut next.runtime, &error);
    next
}

fn interrupt_progress(progress: &mut PreparationProgress, error: &TaskError) {
    if !progress.status.is_terminal() {
        progress.status = TaskStatus::Interrupted;
        progress.detail = Some(error.message.clone());
        progress.error = Some(error.clone());
    }
}

fn interrupt_runtime(progress: &mut RuntimeProgress, error: &TaskError) {
    if !progress.status.is_terminal() {
        progress.status = TaskStatus::Interrupted;
        progress.detail = Some(error.message.clone());
        progress.error = Some(error.clone());
    }
}

fn validate_transition(expected: &DeploymentTask, next: &DeploymentTask) -> Result<(), TaskError> {
    if expected.owner_id.is_nil()
        || expected.session_id.is_nil()
        || expected.task_id.is_nil()
        || expected.owner_id != next.owner_id
        || expected.task_id != next.task_id
        || expected.session_id != next.session_id
        || expected.request_key != next.request_key
        || expected.command != next.command
        || expected.created_at != next.created_at
        || expected.status.is_terminal()
        || expected.row_version < 0
        || expected.row_version == i64::MAX
        || next.row_version != expected.row_version.saturating_add(1)
        || next.updated_at.trim().is_empty()
    {
        return Err(TaskError::storage("部署 CAS 快照身份或版本不合法"));
    }
    if !is_valid_transition(expected.status, next.status) {
        return Err(TaskError::storage("部署状态流转不合法"));
    }
    validate_task_snapshot(next)
}

fn is_valid_transition(from: TaskStatus, to: TaskStatus) -> bool {
    match from {
        TaskStatus::Queued => matches!(
            to,
            TaskStatus::Queued
                | TaskStatus::Running
                | TaskStatus::Succeeded
                | TaskStatus::Failed
                | TaskStatus::Cancelled
                | TaskStatus::Interrupted
        ),
        TaskStatus::Running => matches!(
            to,
            TaskStatus::Running
                | TaskStatus::Succeeded
                | TaskStatus::Failed
                | TaskStatus::Cancelled
                | TaskStatus::Interrupted
        ),
        _ => false,
    }
}

fn validate_task_snapshot(task: &DeploymentTask) -> Result<(), TaskError> {
    validate_command(&task.command)?;
    if task.owner_id.is_nil()
        || task.session_id.is_nil()
        || task.task_id.is_nil()
        || task.row_version < 0
        || !is_hex_digest(&task.request_key)
        || task.request_key != request_key(&task.command)?
        || !valid_timestamp(&task.created_at)
        || !valid_timestamp(&task.updated_at)
    {
        return Err(TaskError::storage("部署快照身份、时间或版本不合法"));
    }
    if task.status == TaskStatus::Queued
        && (task.stage != DeploymentStage::Preflight
            || task.model_download.status != TaskStatus::Queued
            || task.runtime.status != TaskStatus::Queued)
    {
        return Err(TaskError::storage("排队部署快照阶段或子状态不合法"));
    }
    if task.status.is_terminal() {
        if task
            .finished_at
            .as_deref()
            .is_none_or(|value| !valid_timestamp(value))
            || !task.model_download.status.is_terminal()
            || !task.runtime.status.is_terminal()
        {
            return Err(TaskError::storage("终态部署缺少结束时间或准备分支未收束"));
        }
    } else if task.finished_at.is_some() {
        return Err(TaskError::storage("未完成部署不能带 finished_at"));
    }
    if task.status == TaskStatus::Succeeded && task.cancel_requested {
        return Err(TaskError::storage("成功部署不能保留取消意图"));
    }
    match task.status {
        TaskStatus::Succeeded
            if task.error.is_some()
                || task.cleanup_error.is_some()
                || task.model_download.status != TaskStatus::Succeeded
                || task.runtime.status != TaskStatus::Succeeded
                || task.effective_backend.is_none()
                || task.deployment.is_none() =>
        {
            return Err(TaskError::storage(
                "成功部署的后端、归属、准备分支或错误字段不一致",
            ));
        }
        TaskStatus::Failed if task.error.is_none() => {
            return Err(TaskError::storage("失败部署必须保留阶段或清理错误"));
        }
        TaskStatus::Failed
            if task
                .cleanup_error
                .as_ref()
                .is_some_and(|error| error.code != "cleanup_failed") =>
        {
            return Err(TaskError::storage("清理错误必须使用 cleanup_failed 错误码"));
        }
        TaskStatus::Cancelled
            if task.error.is_some()
                || task.cleanup_error.is_some()
                || task.model_download.status == TaskStatus::Failed
                || task.runtime.status == TaskStatus::Failed =>
        {
            return Err(TaskError::storage("正常取消不能携带失败分支或伪造错误"));
        }
        TaskStatus::Interrupted
            if task
                .error
                .as_ref()
                .is_none_or(|error| error.code != "service_restarted")
                || task.cleanup_error.is_some()
                || task.cancel_requested =>
        {
            return Err(TaskError::storage(
                "中断部署必须保留 service_restarted 原因",
            ));
        }
        _ => {}
    }
    validate_task_error(task.error.as_ref())?;
    validate_task_error(task.cleanup_error.as_ref())?;
    validate_progress(&task.model_download, false)?;
    validate_runtime_progress(&task.runtime)?;
    if let Some(estimate) = task.estimate.as_ref() {
        validate_estimate(estimate)?;
    }
    if let Some(ownership) = task.deployment.as_ref() {
        validate_ownership(ownership)?;
    }
    Ok(())
}

fn validate_task_error(error: Option<&TaskError>) -> Result<(), TaskError> {
    if error.is_some_and(|error| error.code.trim().is_empty() || error.message.trim().is_empty()) {
        return Err(TaskError::storage("部署错误快照数据损坏"));
    }
    Ok(())
}

fn validate_estimate(estimate: &DeploymentEstimate) -> Result<(), TaskError> {
    if !valid_timestamp(&estimate.checked_at)
        || estimate.runtime_packages.iter().any(|package| {
            package.name.trim().is_empty()
                || !is_sha256(&package.checksum)
                || !matches!(package.origin.as_str(), "bundled" | "download")
        })
        || estimate.disk_checks.iter().any(|check| {
            !matches!(
                check.storage.as_str(),
                "model" | "runtime" | "model_and_runtime"
            ) || check.sufficient.is_some_and(|sufficient| {
                check
                    .available_bytes
                    .is_none_or(|available| sufficient != (available >= check.required_bytes))
            }) || check.sufficient.is_some() && check.available_bytes.is_none()
        })
    {
        return Err(TaskError::storage("部署预检估算快照数据损坏"));
    }
    Ok(())
}

fn validate_ownership(ownership: &DeploymentOwnership) -> Result<(), TaskError> {
    let id_suffix = ownership.model_id.strip_prefix("bc").unwrap_or_default();
    if !is_hex_digest(&ownership.deployment_key)
        || !is_hex_digest(&ownership.runtime_fingerprint)
        || id_suffix.len() != 32
        || !id_suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        || id_suffix != id_suffix.to_ascii_lowercase()
        || ownership.originating_session.is_nil()
    {
        return Err(TaskError::storage("部署归属证明损坏"));
    }
    Ok(())
}

fn valid_timestamp(value: &str) -> bool {
    value.ends_with('Z') && chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

fn encode_json<T: Serialize>(value: &T) -> Result<String, TaskError> {
    serde_json::to_string(value)
        .map_err(|error| TaskError::storage(format!("无法编码部署 JSON: {error}")))
}

fn encode_optional_json<T: Serialize>(value: Option<&T>) -> Result<Option<String>, TaskError> {
    value.map(encode_json).transpose()
}

fn bool_to_i32(value: bool) -> i32 {
    i32::from(value)
}

fn decode_task(row: &AnyRow) -> Result<DeploymentTask, TaskError> {
    let task_id = parse_uuid(&row_text(row, "task_id")?, "task_id")?;
    let owner_id = parse_uuid(&row_text(row, "owner_id")?, "owner_id")?;
    let session_id = parse_uuid(&row_text(row, "session_id")?, "session_id")?;
    let request_key = row_text(row, "request_key")?;
    let source_text = row_text(row, "source")?;
    let model = row_text(row, "model")?;
    let revision = row_text(row, "revision")?;
    let variant_id = row_text(row, "variant_id")?;
    let checksum = row_text(row, "checksum")?;
    let requested_backend_text = row_text(row, "requested_backend")?;
    let effective_backend = row_optional_text(row, "effective_backend")?
        .map(|value| parse_effective_backend(&value))
        .transpose()?;
    let backend_reason = row_optional_text(row, "backend_reason")?;
    let status = parse_status(&row_text(row, "status")?)?;
    let stage = parse_stage(&row_text(row, "stage")?)?;
    let cancel_requested = parse_bool(row_i32(row, "cancel_requested")?, "cancel_requested")?;
    let model_download = decode_json::<PreparationProgress>(&row_text(row, "model_progress_json")?)
        .map_err(|error| TaskError::storage(format!("模型进度 JSON 损坏: {error}")))?;
    let runtime = decode_json::<RuntimeProgress>(&row_text(row, "runtime_progress_json")?)
        .map_err(|error| TaskError::storage(format!("运行时进度 JSON 损坏: {error}")))?;
    let estimate = decode_optional_json_column::<DeploymentEstimate>(row, "estimate_json")?;
    let deployment = decode_optional_json_column::<DeploymentOwnership>(row, "ownership_json")?;
    let error = decode_optional_json_column::<TaskError>(row, "error_json")?;
    let cleanup_error = decode_optional_json_column::<TaskError>(row, "cleanup_error_json")?;
    let row_version = row_i64(row, "row_version")?;
    let created_at = row_text(row, "created_at")?;
    let updated_at = row_text(row, "updated_at")?;
    let finished_at = row_optional_text(row, "finished_at")?;
    let command = DeploymentCommand {
        model,
        source: parse_source(&source_text)?,
        revision,
        variant_id,
        checksum,
        backend: parse_requested_backend(&requested_backend_text)?,
    };
    let task = DeploymentTask {
        task_id,
        owner_id,
        session_id,
        request_key,
        command,
        status,
        stage,
        effective_backend,
        backend_reason,
        cancel_requested,
        model_download,
        runtime,
        estimate,
        deployment,
        error,
        cleanup_error,
        created_at,
        updated_at,
        finished_at,
        row_version,
    };
    validate_decoded_task(&task)?;
    Ok(task)
}

fn validate_decoded_task(task: &DeploymentTask) -> Result<(), TaskError> {
    validate_task_snapshot(task)
}

fn validate_progress(progress: &PreparationProgress, runtime: bool) -> Result<(), TaskError> {
    if progress.completed_bytes > progress.total_bytes.unwrap_or(u64::MAX)
        || progress
            .actual_sha256
            .as_deref()
            .is_some_and(|value| !is_hex_digest(value))
        || progress
            .error
            .as_ref()
            .is_some_and(|error| error.code.trim().is_empty() || error.message.trim().is_empty())
    {
        return Err(TaskError::storage("部署进度数据损坏"));
    }
    match progress.stage {
        Some(DeploymentStage::DownloadingModel | DeploymentStage::Verifying) if !runtime => {}
        Some(DeploymentStage::PreparingRuntime) if runtime => {}
        None => {}
        _ => return Err(TaskError::storage("部署子任务阶段不匹配")),
    }
    if runtime && progress.actual_sha256.is_some() {
        return Err(TaskError::storage("运行时进度不能伪造单文件摘要"));
    }
    if runtime && progress.task_id.is_some() {
        return Err(TaskError::storage("运行时进度不能暴露独立下载任务 ID"));
    }
    Ok(())
}

fn validate_runtime_progress(progress: &RuntimeProgress) -> Result<(), TaskError> {
    validate_progress(
        &PreparationProgress {
            status: progress.status,
            stage: progress.stage,
            detail: progress.detail.clone(),
            task_id: progress.task_id.clone(),
            completed_bytes: progress.completed_bytes,
            total_bytes: progress.total_bytes,
            download_speed: progress.download_speed,
            actual_sha256: progress.actual_sha256.clone(),
            error: progress.error.clone(),
        },
        true,
    )?;
    if progress
        .origin
        .as_deref()
        .is_some_and(|origin| !matches!(origin, "explicit" | "cache" | "bundled" | "downloaded"))
    {
        return Err(TaskError::storage("运行时来源枚举损坏"));
    }
    Ok(())
}

fn row_text(row: &AnyRow, column: &str) -> Result<String, TaskError> {
    row.try_get(column)
        .map_err(|error| TaskError::storage(format!("无法读取部署文本字段 {column}: {error}")))
}

fn row_optional_text(row: &AnyRow, column: &str) -> Result<Option<String>, TaskError> {
    row.try_get::<Option<String>, _>(column)
        .map_err(|error| TaskError::storage(format!("无法读取部署可空文本字段 {column}: {error}")))
}

fn row_i64(row: &AnyRow, column: &str) -> Result<i64, TaskError> {
    row.try_get::<i64, _>(column)
        .map_err(|error| TaskError::storage(format!("无法读取部署整数字段 {column}: {error}")))
}

fn row_i32(row: &AnyRow, column: &str) -> Result<i32, TaskError> {
    row.try_get::<i32, _>(column)
        .map_err(|error| TaskError::storage(format!("无法读取部署布尔字段 {column}: {error}")))
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, TaskError> {
    Uuid::parse_str(value)
        .map_err(|error| TaskError::storage(format!("部署 UUID 字段 {field} 损坏: {error}")))
}

fn parse_source(value: &str) -> Result<ModelSource, TaskError> {
    ModelSource::parse(value).ok_or_else(|| TaskError::storage("部署来源枚举损坏"))
}

fn parse_requested_backend(value: &str) -> Result<RequestedBackend, TaskError> {
    RequestedBackend::parse(value).ok_or_else(|| TaskError::storage("请求后端枚举损坏"))
}

fn parse_effective_backend(value: &str) -> Result<EffectiveBackend, TaskError> {
    EffectiveBackend::parse(value).ok_or_else(|| TaskError::storage("实际后端枚举损坏"))
}

fn parse_status(value: &str) -> Result<TaskStatus, TaskError> {
    TaskStatus::parse(value).ok_or_else(|| TaskError::storage("部署状态枚举损坏"))
}

fn parse_stage(value: &str) -> Result<DeploymentStage, TaskError> {
    DeploymentStage::parse(value).ok_or_else(|| TaskError::storage("部署阶段枚举损坏"))
}

fn parse_bool(value: i32, field: &str) -> Result<bool, TaskError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(TaskError::storage(format!("部署布尔字段 {field} 损坏"))),
    }
}

fn decode_optional_json_column<T>(row: &AnyRow, column: &str) -> Result<Option<T>, TaskError>
where
    T: serde::de::DeserializeOwned,
{
    let value = row_optional_text(row, column)?;
    value
        .as_deref()
        .map(decode_json)
        .transpose()
        .map_err(|error| TaskError::storage(format!("部署 JSON 字段 {column} 损坏: {error}")))
}

#[cfg(test)]
#[path = "model_deployment_store_tests.rs"]
mod tests;

-- JSON 使用 TEXT，避免两种数据库经过 sqlx Any 时出现不同的解码行为。
CREATE TABLE IF NOT EXISTS sys_model_deployments (
    task_id TEXT NOT NULL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    request_key TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('modelscope', 'hf-mirror', 'huggingface')),
    model TEXT NOT NULL,
    revision TEXT NOT NULL,
    variant_id TEXT NOT NULL,
    checksum TEXT NOT NULL,
    requested_backend TEXT NOT NULL CHECK (requested_backend IN ('auto', 'cpu', 'cuda')),
    effective_backend TEXT CHECK (effective_backend IN ('cpu', 'metal', 'cuda')),
    backend_reason TEXT,
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted')),
    stage TEXT NOT NULL CHECK (stage IN ('preflight', 'preparing_runtime', 'downloading_model', 'verifying', 'starting', 'health_checking', 'attaching_route')),
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    model_progress_json TEXT NOT NULL,
    runtime_progress_json TEXT NOT NULL,
    estimate_json TEXT,
    ownership_json TEXT,
    error_json TEXT,
    cleanup_error_json TEXT,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT
);

-- 清理中的任务仍占用在途键，直到取消或失败真正结算。
CREATE UNIQUE INDEX IF NOT EXISTS idx_sys_model_deployments_active_request
    ON sys_model_deployments (owner_id, request_key)
    WHERE status IN ('queued', 'running');

-- 恢复按本机所有者隔离，避免共享 PostgreSQL 时扫描或中断其他主机。
CREATE INDEX IF NOT EXISTS idx_sys_model_deployments_owner_status
    ON sys_model_deployments (owner_id, status);

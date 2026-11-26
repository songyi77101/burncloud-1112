//! 部署任务迁移的真实数据库回归，不向生产模块暴露测试入口。

use super::{sqlite_0017, sqlite_0019, MigrationRunner, MIGRATIONS_POSTGRES, MIGRATIONS_SQLITE};
use crate::{adapt_sql, create_infrastructure_database_with_url, sqlite_url};
use sqlx::{any::AnyPoolOptions, AnyPool, ConnectOptions, Executor, Row};
use std::{error::Error, io, str::FromStr, time::SystemTime};

const VERSION: &str = "0020_local_model_deployments";
const COLUMN_NAMES: &[&str] = &[
    "task_id",
    "owner_id",
    "session_id",
    "request_key",
    "source",
    "model",
    "revision",
    "variant_id",
    "checksum",
    "requested_backend",
    "effective_backend",
    "backend_reason",
    "status",
    "stage",
    "cancel_requested",
    "model_progress_json",
    "runtime_progress_json",
    "estimate_json",
    "ownership_json",
    "error_json",
    "cleanup_error_json",
    "row_version",
    "created_at",
    "updated_at",
    "finished_at",
];

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn check(condition: bool, message: &str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message).into())
    }
}

async fn prepare_0019(pool: &AnyPool, kind: &str) -> TestResult {
    sqlx::query(
        "CREATE TABLE _schema_migrations (version TEXT PRIMARY KEY, applied_at INTEGER NOT NULL)",
    )
    .execute(pool)
    .await?;
    let migrations = if kind == "postgres" {
        MIGRATIONS_POSTGRES
    } else {
        MIGRATIONS_SQLITE
    };
    for migration in migrations
        .iter()
        .take_while(|migration| !migration.version.starts_with("0020"))
    {
        match (kind, migration.version) {
            ("sqlite", "0017_fix_bool_columns") => sqlite_0017::apply(pool).await?,
            ("sqlite", "0019_download_stable_identity") => sqlite_0019::apply(pool).await?,
            _ => MigrationRunner::apply_sql(pool, migration).await?,
        }
        MigrationRunner::mark_applied(pool, migration.version, kind).await?;
    }
    // 保存升级前的实际下载记录，防止新表迁移意外改写既有下载任务。
    sqlx::query(
        "INSERT INTO sys_downloads (gid, uris, status, total_length, completed_length) \
         VALUES ('legacy-0019', '[]', 'paused', 4096, 2048)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_task(
    pool: &AnyPool,
    kind: &str,
    task_id: &str,
    owner: &str,
    key: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    let sql = adapt_sql(
        kind == "postgres",
        "INSERT INTO sys_model_deployments (task_id, owner_id, session_id, request_key, \
         source, model, revision, variant_id, checksum, requested_backend, status, stage, \
         model_progress_json, runtime_progress_json, created_at, updated_at) \
         VALUES (?, ?, 'session', ?, 'modelscope', 'owner/model-GGUF', 'master', \
         'model-Q4_K_M.gguf', 'sha256:fixed-test-digest', 'cpu', ?, 'preflight', \
         '{}', '{}', '2026-11-24T00:00:00Z', '2026-11-24T00:00:00Z')",
    );
    sqlx::query(&sql)
        .bind(task_id)
        .bind(owner)
        .bind(key)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

async fn assert_columns(pool: &AnyPool, kind: &str) -> TestResult {
    let rows = if kind == "postgres" {
        sqlx::query(
            "SELECT column_name::TEXT AS name, data_type::TEXT AS type, is_nullable::TEXT AS is_nullable, \
             COALESCE(column_default, '') AS default_value FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'sys_model_deployments' \
             ORDER BY ordinal_position",
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query("PRAGMA table_info(sys_model_deployments)")
            .fetch_all(pool)
            .await?
    };
    check(
        rows.len() == COLUMN_NAMES.len(),
        "部署表必须恰好含设计中的25个字段",
    )?;
    let nullable = [
        "effective_backend",
        "backend_reason",
        "estimate_json",
        "ownership_json",
        "error_json",
        "cleanup_error_json",
        "finished_at",
    ];
    for (row, expected_name) in rows.iter().zip(COLUMN_NAMES) {
        let name: String = row.try_get("name")?;
        let field_type: String = row.try_get("type")?;
        let expected_type = match *expected_name {
            "cancel_requested" => "INTEGER",
            "row_version" => "BIGINT",
            _ => "TEXT",
        };
        check(name == *expected_name, "字段名称或顺序与设计不一致")?;
        check(
            field_type.eq_ignore_ascii_case(expected_type),
            "字段类型与设计不一致",
        )?;
        let (is_nullable, default) = if kind == "postgres" {
            (
                row.try_get::<String, _>("is_nullable")? == "YES",
                row.try_get::<String, _>("default_value")?,
            )
        } else {
            (
                row.try_get::<i64, _>("notnull")? == 0,
                row.try_get::<Option<String>, _>("dflt_value")?
                    .unwrap_or_default(),
            )
        };
        check(
            is_nullable == nullable.contains(expected_name),
            "字段空值约束与设计不一致",
        )?;
        check(
            if matches!(*expected_name, "row_version" | "cancel_requested") {
                default == "0"
            } else {
                default.is_empty()
            },
            "字段默认值与设计不一致",
        )?;
    }
    Ok(())
}

async fn assert_indexes_and_constraints(pool: &AnyPool, kind: &str) -> TestResult {
    let index_rows = if kind == "postgres" {
        sqlx::query(
            "SELECT indexname::TEXT AS name, indexdef AS definition FROM pg_indexes \
            WHERE schemaname = 'public' AND tablename = 'sys_model_deployments'",
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT name, sql AS definition FROM sqlite_master \
            WHERE type = 'index' AND tbl_name = 'sys_model_deployments' AND sql IS NOT NULL",
        )
        .fetch_all(pool)
        .await?
    };
    for expected in [
        "idx_sys_model_deployments_active_request",
        "idx_sys_model_deployments_owner_status",
    ] {
        let row = index_rows
            .iter()
            .find(|row| {
                row.try_get::<String, _>("name")
                    .is_ok_and(|name| name == expected)
            })
            .ok_or_else(|| io::Error::other(format!("缺少索引：{expected}")))?;
        let definition: String = row.try_get("definition")?;
        if expected.ends_with("active_request") {
            check(
                definition.contains("UNIQUE")
                    && definition.contains("owner_id")
                    && definition.contains("request_key")
                    && definition.contains("queued")
                    && definition.contains("running")
                    && definition.contains("WHERE"),
                "在途去重索引错误",
            )?;
        } else {
            check(definition.contains("owner_id, status"), "恢复索引列错误")?;
        }
    }
    insert_task(
        pool,
        kind,
        "constraint-base",
        "owner-constraints",
        "constraint-key",
        "queued",
    )
    .await?;
    let sql = "SELECT cancel_requested, row_version FROM sys_model_deployments WHERE task_id = 'constraint-base'";
    let row = sqlx::query(sql).fetch_one(pool).await?;
    check(
        row.try_get::<i32, _>("cancel_requested")? == 0
            && row.try_get::<i64, _>("row_version")? == 0,
        "默认取消位和行版本必须为0",
    )?;
    // 只使用测试内固定SQL片段；绝不把外部输入拼入字段名或表达式。
    for assignment in [
        "status = 'ready'",
        "stage = 'done'",
        "row_version = -1",
        "cancel_requested = 2",
        "source = 'arbitrary'",
        "requested_backend = 'metal'",
        "effective_backend = 'auto'",
        "task_id = NULL",
        "owner_id = NULL",
    ] {
        let result = sqlx::query(&format!(
            "UPDATE sys_model_deployments SET {assignment} WHERE task_id = 'constraint-base'"
        ))
        .execute(pool)
        .await;
        check(
            matches!(result, Err(sqlx::Error::Database(_))),
            "非法字段值必须由数据库约束拒绝",
        )?;
    }
    for status in [
        "queued",
        "running",
        "succeeded",
        "failed",
        "cancelled",
        "interrupted",
    ] {
        let sql = adapt_sql(
            kind == "postgres",
            "UPDATE sys_model_deployments SET status = ? WHERE task_id = 'constraint-base'",
        );
        sqlx::query(&sql).bind(status).execute(pool).await?;
    }
    for stage in [
        "preflight",
        "preparing_runtime",
        "downloading_model",
        "verifying",
        "starting",
        "health_checking",
        "attaching_route",
    ] {
        let sql = adapt_sql(
            kind == "postgres",
            "UPDATE sys_model_deployments SET stage = ? WHERE task_id = 'constraint-base'",
        );
        sqlx::query(&sql).bind(stage).execute(pool).await?;
    }
    Ok(())
}

async fn assert_inflight_uniqueness(pool: &AnyPool, kind: &str) -> TestResult {
    let (first, second) = tokio::join!(
        insert_task(pool, kind, "race-a", "same-owner", "same-request", "queued"),
        insert_task(
            pool,
            kind,
            "race-b",
            "same-owner",
            "same-request",
            "running"
        )
    );
    match (first, second) {
        (Ok(()), Err(error)) | (Err(error), Ok(())) => {
            check(
                error
                    .as_database_error()
                    .is_some_and(|error| error.is_unique_violation()),
                "并发失败必须为唯一键冲突，而非连接或锁故障",
            )?;
        }
        _ => return Err(io::Error::other("同owner/request的并发在途插入必须仅一条成功").into()),
    }
    insert_task(
        pool,
        kind,
        "other-owner",
        "different-owner",
        "same-request",
        "running",
    )
    .await?;
    // 取消意图不能提前释放在途键，真正终态才可重试。
    sqlx::query(
        "UPDATE sys_model_deployments SET cancel_requested = 1 WHERE owner_id = 'same-owner'",
    )
    .execute(pool)
    .await?;
    let blocked = insert_task(
        pool,
        kind,
        "cancel-race",
        "same-owner",
        "same-request",
        "queued",
    )
    .await;
    check(
        blocked
            .as_ref()
            .err()
            .and_then(sqlx::Error::as_database_error)
            .is_some_and(|error| error.is_unique_violation()),
        "清理中的取消必须仍占用在途键",
    )?;
    sqlx::query("UPDATE sys_model_deployments SET status = 'cancelled', finished_at = '2026-11-24T00:01:00Z' WHERE owner_id = 'same-owner'")
        .execute(pool).await?;
    insert_task(pool, kind, "retry", "same-owner", "same-request", "queued").await?;
    // 所有终态均不占用在途键，保留完整历史不阻塞后续重试。
    for status in ["succeeded", "failed", "cancelled", "interrupted"] {
        insert_task(pool, kind, status, "terminal-owner", "terminal-key", status).await?;
    }
    insert_task(
        pool,
        kind,
        "after-terminals",
        "terminal-owner",
        "terminal-key",
        "queued",
    )
    .await?;
    Ok(())
}

async fn exercise_upgrade(url: &str, kind: &str) -> TestResult {
    sqlx::any::install_default_drivers();
    let baseline = AnyPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await?;
    prepare_0019(&baseline, kind).await?;
    let versions_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _schema_migrations")
        .fetch_one(&baseline)
        .await?;
    check(versions_before == 19, "测试必须从真实0019基线升级")?;
    let before: (i64, String, String, String, i64, i64) = sqlx::query_as(
        "SELECT id, gid, uris, status, CAST(total_length AS BIGINT), CAST(completed_length AS BIGINT) \
         FROM sys_downloads WHERE gid = 'legacy-0019'",
    ).fetch_one(&baseline).await?;
    baseline.close().await;

    let database = create_infrastructure_database_with_url(url).await?;
    let result: TestResult = async {
        let pool = database.get_connection()?.pool();
        assert_columns(pool, kind).await?;
        assert_indexes_and_constraints(pool, kind).await?;
        assert_inflight_uniqueness(pool, kind).await?;
        let after: (i64, String, String, String, i64, i64) = sqlx::query_as(
            "SELECT id, gid, uris, status, CAST(total_length AS BIGINT), CAST(completed_length AS BIGINT) \
             FROM sys_downloads WHERE gid = 'legacy-0019'",
        ).fetch_one(pool).await?;
        check(before == after, "部署迁移不能改写旧下载记录")?;
        let sql = adapt_sql(kind == "postgres", "SELECT COUNT(*) FROM _schema_migrations WHERE version = ?");
        let recorded: i64 = sqlx::query_scalar(&sql).bind(VERSION).fetch_one(pool).await?;
        check(recorded == 1, "新迁移必须实际登记一次")?;
        MigrationRunner::run(&database).await?;
        let recorded_again: i64 = sqlx::query_scalar(&sql).bind(VERSION).fetch_one(pool).await?;
        check(recorded_again == 1, "重复迁移不得重复登记")?;
        check(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sys_model_deployments")
            .fetch_one(pool).await? > 0, "重复迁移不得删除部署历史")?;
        Ok(())
    }.await;
    database.close().await?;
    result
}

#[test]
fn both_catalogues_register_the_same_deployment_schema() -> TestResult {
    for migrations in [MIGRATIONS_SQLITE, MIGRATIONS_POSTGRES] {
        check(
            migrations
                .iter()
                .filter(|migration| migration.version == VERSION)
                .count()
                == 1,
            "每个数据库只能登记一次部署迁移",
        )?;
        check(
            migrations
                .windows(2)
                .all(|pair| pair[0].version < pair[1].version),
            "完整迁移版本名必须严格递增",
        )?;
    }
    check(
        include_str!("../../migrations/sqlite/0020_local_model_deployments.sql")
            == include_str!("../../migrations/postgres/0020_local_model_deployments.sql"),
        "两种数据库的公共DDL必须保持一致",
    )
}

#[tokio::test]
async fn sqlite_upgrades_from_0019_with_constraints_and_preserved_downloads() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("deployments.sqlite");
    let url = sqlite_url(
        &path.to_string_lossy().replace('\\', "/"),
        cfg!(windows),
        true,
    );
    exercise_upgrade(&url, "sqlite").await
}

#[tokio::test]
#[ignore = "需要显式BURNCLOUD_TEST_POSTGRES_URL；创建并删除独立测试库，不允许伪报PostgreSQL已验证"]
async fn postgres_upgrades_from_0019_with_constraints_and_preserved_downloads() -> TestResult {
    let url = std::env::var("BURNCLOUD_TEST_POSTGRES_URL")?;
    let options = sqlx::postgres::PgConnectOptions::from_str(&url)?;
    let name = format!(
        "bc_deploy_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos()
    );
    let mut admin = options.connect().await?;
    // 名称仅由进程号和时间生成，不插入环境变量中的标识符或修改现有数据库。
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await?;
    let target = options.database(&name).to_url_lossy().to_string();
    let result = exercise_upgrade(&target, "postgres").await;
    let cleanup = admin
        .execute(format!("DROP DATABASE {name} WITH (FORCE)").as_str())
        .await;
    match (result, cleanup) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(error), Ok(_)) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Err(error), Err(cleanup)) => Err(io::Error::other(format!(
            "迁移测试失败：{error}；独立测试库清理失败：{cleanup}"
        ))
        .into()),
    }
}

# 015 实现部署任务数据与本机仓储 CAS

## 修改的信息

- 新增部署状态、阶段、来源、后端及严格 JSON DTO；任务错误只含契约 `code` / `message`，部署所有权快照不含路径、PID 或可执行句柄。
- 新增只包装注入的 `Arc<Database>` 的仓储。请求去重键由规范字段按固定 JSON 顺序计算；`insert_or_join` 依赖数据库部分唯一索引原子处理并发，不做先查后插。
- `load` 使用 `owner_id + task_id` 限定读取，并严格校验 UUID、枚举、布尔整数、摘要、JSON、时间、终态和归属数据。存储错误统一返回 `deployment_storage_failed`，详细原因只进入受控日志。
- `compare_update` 校验任务身份不变、版本严格递增和合法状态流转；更新使用 owner/task/request_key/row_version/活动状态条件，成功额外要求数据库中的 `cancel_requested = 0`。CAS 冲突返回 `None`。
- `interrupt_incomplete` 只读取当前 owner 的旧会话 queued/running 任务，以 CAS 写入 `interrupted` / `service_restarted`，收束未完成子项并只返回可证明的部署归属；不按 PID 或进程名猜测清理。
- 添加 SQLite 并发去重、owner 隔离、终态后重试、CAS 丢更新、取消先于成功、坏 JSON/枚举/整数、旧会话恢复测试。另提供显式 PostgreSQL 独立数据库测试入口。

## 修改的文件与文件内位置

- `crates/interfaces/server/src/lib.rs`：登记私有 `model_deployment` 与 `model_deployment_store` 模块。
- `crates/interfaces/server/src/model_deployment.rs`：本机部署内部 DTO、枚举、严格序列化边界与受控错误。
- `crates/interfaces/server/src/model_deployment_store.rs`：注入式仓储、请求摘要、owner 限定读写、CAS、启动恢复及持久化快照校验。
- `crates/interfaces/server/src/model_deployment_store_tests.rs`：与生产代码分离的独立仓储测试。
- `docs/dev-docs/5/task-report/015-deployment-store-cas.md`：本任务执行报告。

## 检查结果

- `cargo fmt --all`：通过；最终 `cargo fmt --all -- --check`：通过。
- `cargo test -p burncloud-server --no-default-features --lib model_deployment_store::tests -- --nocapture`：最终快照通过，8 passed / 0 failed / 1 ignored。
- `cargo clippy -p burncloud-server --all-targets --no-default-features -- -D warnings`：最终代码通过。
- 提交钩子要求的 `cargo test --workspace --no-default-features`：通过，2062 passed / 0 failed / 149 ignored（unit 962 passed、integration 994 passed、doc 106 passed）。
- 提交钩子要求的 `cargo clippy --workspace --all-targets --no-default-features -- -D warnings`：通过。
- PostgreSQL 实机证据：未执行。当前环境未设置 `BURNCLOUD_TEST_POSTGRES_URL`；本任务 PostgreSQL 仓储测试明确 ignored，未将 SQLite 结果冒充 PostgreSQL 证据。
- `cargo deny check`：在线检查未通过，RustSec advisory-db 的 GitHub fetch 被网络连接重置；`cargo deny --offline check` 使用本机缓存完成 bans/licenses/sources/advisories 检查，但不能证明 advisory 数据是最新。故提交钩子整体未通过，尚未创建任务提交。
- 构建缓存使用专用 D: target 目录以避开 E: 空间不足；未清理或递归删除用户现有 target。提交钩子中的报告路径被现有根 `.gitignore` 忽略，已按任务报告要求强制暂存。

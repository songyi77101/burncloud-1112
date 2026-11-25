```json
{
  "编号": "001",
  "依赖": []
}
```

# 001 新增双数据库部署任务迁移

- 任务：新增双数据库部署任务迁移
- 工作区：database（读写）——crates/platform/storage/database/migrations/{sqlite,postgres}/0020_local_model_deployments.sql；src/migration/mod.rs；独立迁移测试
- 产出：两端 0020 迁移、迁移登记与约束回归测试
- 前置任务：无
- 后置任务：015 实现部署任务数据与本机仓储 CAS
- 设计依据：第四节 sys_model_deployments；第二节 2.1、2.3、2.4
- 验收标准：
  - SQLite 与 PostgreSQL 各自从 0019 升至 0020 后，25 个字段、类型、CHECK、默认值与设计表逐项一致；不改写 sys_downloads 或旧迁移。
  - 相同 owner/request_key 的两个 queued/running 插入仅一条成功；任务终态后可插入新任务，不同 owner 互不冲突。
  - 非法六态/七阶段、负 row_version 或非法取消位被约束或仓储边界拒绝；(owner_id,status) 恢复索引存在。
  - 独立测试保留真实迁移命令/结果；缺 PostgreSQL 环境仅记录未执行，不能据 SQLite 结果宣称双端通过。

> 来源：开发序号5、用户于2026-11-24确认的 `../local-model-auto-deploy-dev-doc.md`、同目录流程图与接口契约；分支 `codex/local-model-auto-deploy`，工作区以 `../dev-context.md` 为准。任务定义不代表业务代码或实机验收已完成；实际执行次序只采用脚本生成的 `execution-order.md`。

## 范围与执行约束

- 本任务只定义并登记表结构，不新增数据库 crate 的部署仓储公共 API，不建立运行时表或历史任务清理 API。
- 只实现本任务摘录的定稿设计；新函数/方法/字段默认私有，跨文件和跨crate可见性不得超过开发文档第六节公开清单，不为测试导出内部实现。
- 注释与rustdoc使用中文；公共边界黑盒校验，内部公有函数白盒契约检查；私有函数不重复校验已保证参数，但新读取的网络/磁盘/归档/进程结果仍须验证。
- 测试写入独立文件；每个公有API/内部公有方法有独立测试，关键公共模块提供可编译# Examples，修复有回归；不使用unsafe、外部输入unwrap/expect或静默丢弃Result。
- 只写上述登记工作区和具体范围；project-root仅登记允许的Cargo.toml/Cargo.lock/README.md/本轮文档。不得覆盖用户target运行时、未跟踪server/docs文件、开发规范或dev-context.md。
- 编码/测试/提交状态由后续任务报告如实记录；提交使用Conventional Commits，提交前后均执行cargo fmt、cargo clippy、cargo test并记录实际feature/target与结果。真实硬件、许可、签名或测试资源缺失不填假证据。

## 实现内容

### 数据结构

- 名称：sys_model_deployments
- 类型：新增数据库表
- 说明：SQLite/PostgreSQL 同步迁移。类型统一为 TEXT/BIGINT/INTEGER，JSON 用 TEXT 保持 sqlx Any 的读写一致；全部 SQL 参数绑定，不拼接身份。以下按字段作为表头列出类型与约束；每个 SQL 字段独立定义，不将 JSON 用作去重主键。
- 结构：

| task_id | owner_id | session_id | request_key | source | model | revision | variant_id | checksum | requested_backend | effective_backend | backend_reason | status | stage | cancel_requested | model_progress_json | runtime_progress_json | estimate_json | ownership_json | error_json | cleanup_error_json | row_version | created_at | updated_at | finished_at |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| TEXT PRIMARY KEY | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NULL | TEXT NULL | TEXT NOT NULL | TEXT NOT NULL | INTEGER NOT NULL DEFAULT 0 | TEXT NOT NULL | TEXT NOT NULL | TEXT NULL | TEXT NULL | TEXT NULL | TEXT NULL | BIGINT NOT NULL DEFAULT 0 | TEXT NOT NULL | TEXT NOT NULL | TEXT NULL |
| 服务生成 UUID | 本机所有者，所有查询/CAS 必须带上 | 创建会话 | 完整请求摘要 | 契约三来源枚举 | 仓库模型 | 用户修订 | GGUF 相对标识 | 规范期望 SHA | auto/cpu/cuda | cpu/metal/cuda，尚未实测为空 | 中文选择说明 | 契约六态 CHECK | 契约七阶段 CHECK | CHECK IN (0,1) | PreparationProgress 序列化，含稳定下载 ID 与实际摘要 | RuntimeProgress，含来源/版本/两个包汇总进度 | 预检预算 | DeploymentOwnership；实时健康/ready_to_chat 不持久化为事实 | 原始错误 | 独立清理错误 | 非负 CAS 版本 | RFC3339 UTC | RFC3339 UTC | 仅终态非空 |

索引：`UNIQUE(owner_id,request_key) WHERE status IN ('queued','running')`；另建 `(owner_id,status)` 恢复索引。事务内插入/读取在途与 CAS 更新，索引约束是最终并发保障。终态约束由仓储转移函数统一实施，JSON 反序列化错误是存储错误，不转成默认成功值。

迁移文件：`migrations/{sqlite,postgres}/0020_local_model_deployments.sql`，同时登记两个迁移目录。保留 0019 和所有 sys_downloads 字段，不重写历史数据。部署记录不级联删除模型/运行时；本轮不建清理历史任务 API，不另建运行时数据库表。

本任务不新增或修改函数实现；仅覆盖上述表结构、固定资源、发行配置或说明文件，不为满足模板虚构函数。

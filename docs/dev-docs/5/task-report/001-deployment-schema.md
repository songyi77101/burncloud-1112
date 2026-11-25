# 001 新增双数据库部署任务迁移：任务报告

- 开发序号：5；分支：`codex/local-model-auto-deploy`。
- 日期：2026-11-24。
- 状态：代码及任务级 SQLite/PostgreSQL 验收通过；提交、推送与提交后复验结果由执行反馈记录。本报告不表示其余24项任务或整体自动部署已完成。

## 修改的信息

1. 两种数据库各增加 `0020_local_model_deployments.sql`，创建25字段部署任务表，保留模型身份、请求/实际后端、两个子快照、错误/清理错误、归属、行版本与时间。
2. 对来源、后端、六种任务状态、七种阶段、取消位和非负行版本增加CHECK；用 `(owner_id,request_key)` 部分唯一索引限制 queued/running，同时登记 `(owner_id,status)` 恢复索引。
3. 在两个迁移目录中分别登记新迁移，不改变旧迁移或下载记录。源码已经存在完整版本名 `0020_download_model_identity`；迁移器主键为完整字符串，因此保留原登记后追加不同的 `0020_local_model_deployments`。测试验证完整版本名严格递增且各只登记一次，没有复用或改写旧迁移版本。
4. 独立测试文件从真实0019迁移基线升级，经生产数据库工厂运行待执行迁移；检查字段/类型/空值/默认值、非法值、并发在途去重、不同owner隔离、取消清理占键、四种终态重试和重复迁移幂等。
5. 没有新增生产公共API、部署仓储或运行时数据库表，没有改变可见性或依赖。JSON仍使用TEXT，跨数据库状态流转的CAS与JSON语义留给任务015。

## 修改的文件与文件内位置

| 文件 | 位置 | 内容 |
| --- | --- | --- |
| `crates/platform/storage/database/migrations/sqlite/0020_local_model_deployments.sql` | CREATE TABLE及两个CREATE INDEX | SQLite部署表和约束 |
| `crates/platform/storage/database/migrations/postgres/0020_local_model_deployments.sql` | CREATE TABLE及两个CREATE INDEX | PostgreSQL同等DDL |
| `crates/platform/storage/database/src/migration/mod.rs` | MIGRATIONS_SQLITE、MIGRATIONS_POSTGRES、末尾cfg(test)模块声明 | 分别登记资源及独立测试 |
| `crates/platform/storage/database/src/migration/local_deployments_tests.rs` | prepare_0019、exercise_upgrade、assert_columns、assert_indexes_and_constraints、assert_inflight_uniqueness及三个测试 | 迁移与约束回归，无测试代码混入生产实现文件 |

## 检查结果

### 提交前命令与实际结果

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过；不改写任务范围外文件 |
| `cargo clippy -p burncloud-database --all-targets --locked` | 通过，无新增lint警告 |
| `cargo test -p burncloud-database --locked` | 106 passed、0 failed、1 ignored；新PostgreSQL用例默认明确忽略，须下述显式命令执行；既有测试自身也有无环境时提前返回的PostgreSQL用例，不把其ok当真实PG证据 |
| `cargo test -p burncloud-database --lib local_deployments_tests --locked -- --nocapture` | 新SQLite与目录一致性测试2 passed；PG用例显式标记ignored |
| `cargo test -p burncloud-database --lib postgres_upgrades_from_0019_with_constraints_and_preserved_downloads --locked -- --ignored --nocapture` | 配置本次独立本地PG实例后1 passed、0 failed、0 ignored；真实执行完整0019升级和约束检查 |

以上Cargo验证范围为database包及其依赖和全仓格式检查，不是完整workspace clippy/test，也不是模型或四平台实机部署验收。

### PostgreSQL实际资源与回归

- 本机Docker引擎未运行，未把无环境的测试标记为通过。改用官方PostgreSQL Windows下载页指向的EDB便携二进制 PostgreSQL 16.6，归档304112325字节，下载后SHA-256为 `6a1bfb6435b13d9563ae481445c70ac2a19846bd8a430b12903b408eec300f9b`。此值是本次下载记录，不冒充已取得上游签名摘要。
- 下载、解压和数据库簇仅在 `target/tmp/deployment-schema-postgres/` 独立随机目录中；没有注册Windows服务、修改PATH、写入.env或碰用户已有llama.cpp。测试实例仅监听127.0.0.1，使用随机临时端口；无业务数据。该资源仅用于开发验收，绝不随BurnCloud发行。
- 测试显式从 `BURNCLOUD_TEST_POSTGRES_URL` 连接本次实例，创建唯一随机测试库，成功/失败均尝试删除本次库；失败与清理双错误均保留。本次提交被工作区清洁度钩子拦截后，已精确停止该独占测试簇；pg_ctl返回server stopped、日志记录database system is shut down，并确认51468端口无监听。不残留后台测试服务。
- 真实PG首次运行发现测试查询中SQLx Any无法解码PostgreSQL NAME系统类型；测试元数据列改为显式TEXT转换后重跑通过。未修改生产连接或弱化断言。
- 临时启动包装器曾遇到Windows子进程继承输出句柄导致等待卡住；数据库日志确认实例就绪后，直接执行上述Cargo命令完成实际测试。该包装器问题不算测试通过证据；停止测试簇后，原包装会话已以TimeoutExpired退出，未将其误报为成功。

### 对照验收标准

| 标准 | 判定与证据 |
| --- | --- |
| 两数据库从0019升级、25字段/类型/CHECK/默认值一致 | 通过；两个真实后端执行相同contract，目录一致性测试校对两端DDL |
| 同owner/request并发仅一条在途、终态可重试、不同owner互不冲突 | 通过；并发失败明确检查为唯一键冲突，不把数据库锁故障当去重成功 |
| 非法状态/阶段/取消位/负行版本拒绝，恢复索引存在 | 通过；逐项非法UPDATE被数据库约束拒绝，六态/七阶段合法值逐项写入，索引定义核对 |
| 保留旧下载与历史迁移，不伪报PG | 通过；升级前后旧行主键/身份/状态/长度一致，新PG用例另行显式执行通过 |

## 尚未覆盖的整体边界

运行时安装、HTTP下载切换、部署接口/客户端、真实模型对话、四目标发行、签名授权和WindowsCUDA均尚未实现或验收；不创建PR留档，不宣称本机自动部署已经可用。

## 提交门禁状态（2026-11-24）

- 已对本任务5个精确文件执行git add和git diff --cached --check；未将前阶段34份设计/任务文档夹带入本任务提交。
- 尝试 `git commit --only -m "feat(database): persist local model deployment task schema" -- <本任务5个路径>`，现有pre-commit在 `cargo run --quiet -- code test --staged` 拒绝提交，原始报错：`Error: Commit blocked: stage or stash tracked changes and untracked files before code test --staged`。未禁用钩子、未amend，也未修改Git配置。
- 工作区存在用户原有未跟踪文件 `crates/interfaces/server/docs/hardware-model-resolution-api.md`。首次拦截时没有擅自处理该文件。用户随后明确同意仅临时stash该文件；已定向备份，并核对备份可按原CRLF恢复到原SHA-256 `fbe5055d38112fa90f37e866ddd7a8ab73483b9d7b68e6353d0010daf0f1b6e7`。不将该文件加入本次提交，提交操作结束后原样恢复。
- 本报告写入时正在重新执行正常提交门禁；commit、push与提交后复验以实际命令结果为准，未成功前不标记任务完成。后续任务002尚未开始修改代码。

### Windows提交工具自锁记录

- 用户批准的第一次定向stash已完成，提交尝试后已从备份恢复原CRLF字节，SHA-256与原文件完全相同；只清理本次stash，没有处理其他用户改动。
- 解除未跟踪文件拦截后，钩子执行 `cargo run --quiet -- code test --staged`，其prepare步骤又执行 `cargo build -p burncloud --bin burncloud --no-default-features`；Windows拒绝替换正在运行的 `target/debug/burncloud.exe`，原始错误：`failed to remove file ... target\debug\burncloud.exe` / `拒绝访问。 (os error 5)`。该次提交仍未成功。
- 不改钩子、不改Git配置、不关闭测试。准备使用仅在本次命令环境有效的Cargo runner：只把正在执行 `code test` 的检查工具复制到独立临时目录运行，让原始产物可以正常重建；其余执行原样转发，保留所有测试参数、环境、工作目录和退出码。临时辅助脚本仅在 `target/tmp/deployment-commit-support/`，不随产品提交。已用 `--plan` 确认runner选中及完整检查命令；plan不算测试通过。

### 全工作区编译资源记录

- 使用临时runner后，钩子的 `cargo build -p burncloud --bin burncloud --no-default-features` 已真实通过（25.30秒），证明原产物自锁已解除，没有跳过prepare。
- 随后的 `cargo test --workspace --no-default-features` 在默认并发编译时失败：`rustc-LLVM ERROR: out of memory`、`页面文件太小，无法完成操作。 (os error 1455)`、`LINK : fatal error LNK1102: 内存不足`；没有完整测试统计，不宣称全工作区测试通过。完整日志由既有工具保留在 `.git/burncloud/checks/1795523235170-40448/02-test.log`。
- 提交失败后已再次恢复用户原文件，SHA-256仍与原始值相同，并清理本次定向stash。下一次仅设置命令环境 `CARGO_BUILD_JOBS=1` 限制编译并发，不缩减测试范围、不改项目Cargo配置。

### 钩子Git环境隔离与工具准备

- 串行全工作区编译已完成（14m49s），不再出现内存错误；测试执行到burncloud-code的Git回归时失败。统计为259 passed、16 failed；未运行完全部工作区，严格Clippy/钩子内deny尚未执行，不记为整体通过。原始错误包括父仓库 `.git/next-index-27824.lock.lock` 冲突及 `invalid object ... .github/hooks/commit-msg`。实际暂存区完整、diff检查通过，没有把临时测试仓库内容带入真实索引。
- 源码核对：Git回归用例创建独立仓库，但Fixture::command未清理钩子导出的GIT_INDEX_FILE；`git commit --only` 的父仓库临时索引因此被继承。没有修改范围外的burncloud-code源码。
- 在无钩子仓库局部环境下独立执行 `cargo test -p burncloud-code --test regression --no-default-features --locked`，22 passed、0 failed。
- 临时runner仅对Cargo测试产物清理 `git rev-parse --local-env-vars` 声明的父仓库局部变量；检查工具本身仍保留本次提交索引，继续按真实5文件选范围和生成receipt。其余环境、参数、工作目录和退出码保留，不过滤测试。注入独占的父索引证据路径运行同一Git回归二进制，22 passed、0 failed，且证据索引路径未被创建。该隔离只在本轮执行环境有效，非产品代码或钩子修改。
- 依据仓库code init说明补装缺失的cargo-deny v0.22.0（`cargo install --locked cargo-deny --jobs 1 --target-dir target/tmp/deployment-commit-support/cargo-deny-build`）；未改Git/Cargo配置或项目依赖。独立执行 `cargo deny check`，advisories、bans、licenses、sources均ok，保留现有配置未命中的warning，不增忽略项。
- 每次失败后均恢复用户文件到原SHA-256并只删除当次stash；现继续正常提交复验，未成功前不标为完成。

### 文档测试资源隔离验证

- 完整门禁中的Git回归22项已全部通过；全工作区单元916 passed/0 failed/3 ignored，集成981 passed/0 failed/141 ignored。进入文档测试后出现内存映射及MSVC PDB LIMIT错误，部分文档用例未能编译；当次工具汇总1936 passed、27 failed、146 ignored，不能报完整门禁通过。完整日志：`.git/burncloud/checks/1795524993124-28280/02-test.log`。
- 失败后恢复用户原文件并校对原SHA-256一致，真实暂存内容和HEAD保持完整，没有commit生成。
- 仅设置 `CARGO_BUILD_JOBS=1` 与 `RUST_TEST_THREADS=1`，独立执行 `cargo test --workspace --doc --no-default-features --locked`，全部选中的工作区文档测试实际通过（退出码0，保留既有忽略项）。没有设置RUSTDOCFLAGS、删除文档示例、增加ignore或修改其他模块源码；串行运行解决本机并发链接资源不足。
- 下一次完整提交沿用相同串行构建/测试资源设置；仍执行所有原有钩子，不因已经单独通过而跳过完整测试、Clippy或依赖审计。

### 最终提交前补充预检

- `cargo clippy --workspace --all-targets --no-default-features -- -D warnings` 在 `CARGO_BUILD_JOBS=1` 下退出码0，严格全工作区检查通过（5m56s），没有降低warning门禁。
- 全工作区文档测试已用串行运行实际通过，依赖审计此前四项均ok；下一次提交仍完整重跑现有钩子，不把分开通过的命令伪称为已提交。

### API测试派生进程回收

- 再次提交在最初cargo run阶段遇到原可执行文件被占用，未进入后续钩子。进程核对为PID30808、父PID9540、21:18:37启动、仅127.0.0.1:52504监听；同父PID的 `burncloud-api-tests-9540.db` 于21:18:38创建。既有 `tests/common/mod.rs` 已明确记录static OnceLock持有Child而退出时不析构的测试资源缺陷。
- 确认父测试进程已退出、子进程路径/命令及数据库创建时间吻合后，只停止该本轮测试服务器，并确认端口关闭。未按进程名全局停止服务，也未更改范围外测试源码。用户文件已恢复原摘要，临时stash已定向清理。
- 临时Cargo runner对api_tests测试产物等待退出后，按其实际子PID及本仓库服务器路径回收遗留服务；Popen句柄在清理前保持持有，清理失败会导致检查失败。不会停止通过E2E_BASE_URL引用的外部服务，也不处理其他进程。
- 直接运行该已编译二进制时首次遗漏Cargo提供的CARGO_MANIFEST_DIR，9项明确失败；补齐与Cargo一致的包目录环境后，完整178项中73 passed、0 failed、105既有ignored，实际清理本次测试派生PID7188；后续查询无本仓库burncloud服务器残留。未将缺环境的首次结果当作通过。

## 当前阻塞：范围外Vertex缓存隔离（2026-11-24）

- 最新完整钩子在串行测试模式执行到router单元测试时失败：`adaptor::vertex::tests::test_overrides` 期望POST /auth一次、实际0次；当次汇总1317 passed、1 failed、3 ignored（后续测试/文档/Clippy/deny未走完）。日志：`.git/burncloud/checks/1795528774967-6360/02-test.log`。没有commit生成，也未push。
- 缩小到现有Vertex四项测试，执行 `target/debug/deps/burncloud_router-75318a1e5fbc3a23.exe adaptor::vertex::tests --test-threads=1`，确定性复现3 passed、1 failed。并非只记录一次偶发失败后盲目重试。
- 本次未修改的 `crates/traffic/router/src/adaptor/vertex.rs` 中，TOKEN_CACHE仅以client_email索引；get_access_token在调用认证地址前直接返回该email缓存。两个既有测试使用相同email、不同auth_url，前一项缓存使override测试不再请求其认证端点。该问题在先前默认并发测试中未暴露，本轮全局串行执行显式暴露。
- 不通过跳过此用例、清空共享缓存后单跑失败项或调整执行顺序把门禁伪装为通过。router不在任务001登记写范围，当前不擅自修复；需用户批准独立最小缓存隔离修复及对应回归范围，再继续当前提交和任务002。
- 用户原文件再次恢复为原CRLF字节，SHA-256一致；当次stash d77faecbac440fc6e21f1b12e480ced90afd148e已定向删除。前阶段34份已暂存设计文档未改内容或夹带提交。PostgreSQL测试簇未重新启动，无本轮API测试服务器残留。
- 任务001业务实现与数据库专项检查已经通过，但提交门禁尚未通过，任务不标为完成。任务002尚未开始改码；整体自动部署功能仍未实现完成，未进入PR阶段，不创建-pr.md占位产物。

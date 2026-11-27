# 031 接入 LlamaCppRuntimePreparer 与现有 DeploymentService

## 修改的信息

- 将固定 Gitee manifest 接入服务端运行时准备器：校验固定 provider、repository、commit、版本、四平台资源、backend、文件名、归档格式、大小、SHA-256、解压路径与资源限制；运行时请求不能选择漂移版本。
- 将 `LlamaCppRuntimePreparer` 改为按显式 `BURNCLOUD_LLAMA_SERVER`、受管缓存、固定 Gitee 下载的优先级准备运行时。显式路径必须为绝对普通文件并通过 `llama-server` 身份、版本输出、依赖和目标 backend 探测；配置无效时 fail-closed。
- 受管路径通过进程内共享作业与跨进程安装锁串行化。锁内复查并探测缓存；探测失败的缓存在锁内隔离。缓存未命中时使用原生运行时下载器、断点元数据和取消信号，完成摘要校验后复制到同文件系统 staging，安全解压、探测并原子提交。
- 运行时下载进度已发布到共享安装作业的进度通道；父任务进度快照、DELETE 对共享订阅的映射按执行顺序由任务 032 接入，本任务不新增 runtime 管理接口。
- 自动部署 POST 对 CUDA 请求在提交任务及模型下载前返回 HTTP 422 `unsupported_backend`，不会以 CPU 资源伪装成功。运行时准备错误按稳定错误码转换为安全的任务错误信息，不向客户端暴露本地路径或原始诊断。
- `node_deployment::start_local` 已按既有流程先调用 `RuntimePreparer::prepare`，随后将结果传给 `DeploymentService::deploy_prepared`；本任务保留该生命周期及启动参数，不改动 `node_deployment.rs`。

## 修改的文件与文件内位置

- `crates/interfaces/server/src/node_runtime.rs`
  - `LlamaCppRuntimePreparer::supported_runtimes`、`prepare`：声明 `llama.cpp` 能力并编排来源优先级。
  - `PreparationFailure`：将 manifest、缓存、下载、解压、探测错误转换为稳定且不泄漏路径的错误。
  - `explicit_runtime_executable`、`prepare_explicit_runtime`：校验管理员显式路径并探测运行时。
  - `runtime_cache_root`、`runtime_cache_identity`：解析受管缓存根和固定缓存身份。
  - `prepare_managed_runtime`、`prepare_managed_runtime_inner`、`forward_download_progress`：共享安装、取消桥接、下载、解压、探测和原子提交。
- `crates/interfaces/server/src/node_runtime/manifest.rs`（新增）：固定 manifest 解析、可信字段校验、目标资源选择及稳定错误码。
- `crates/interfaces/server/src/node_runtime/manifest_tests.rs`（新增）：覆盖四目标清单、身份漂移、backend、版本和字段缺失。
- `crates/interfaces/server/src/node_runtime/cache.rs`：新增 `quarantine_cached_runtime`，供安装锁保护下隔离未通过再次探测的缓存；为任务 032 保留的共享作业进度访问点加注释性 dead-code 豁免。
- `crates/interfaces/server/src/node_runtime/probe.rs`：标记任务 032 接入请求 backend 时使用的解析器。
- `crates/interfaces/server/src/node_runtime/tests.rs`：替换旧便携运行时回退测试，覆盖显式路径绝对性、普通文件、无效配置 fail-closed 和运行时能力声明。
- `crates/interfaces/server/src/api/model_deployments.rs`
  - `submit_deployment` / `unsupported_backend_preflight`：在任务登记和下载前拒绝 CUDA。
  - `runtime_failure_details`、`run_deployment`、`fail_task_with_cleanup`：保留运行时稳定错误码并更新运行时失败分支。
- `crates/interfaces/server/src/api/model_deployments_tests.rs`：覆盖 CUDA 前置拒绝及运行时错误码脱敏映射。
- `crates/interfaces/server/src/node_deployment.rs`：已检查 `NodeDeploymentState::start_local`（约第 604–646 行）；现有 `prepare → deploy_prepared` 生命周期符合本任务要求，未改动。

## 检查结果

- 来源优先级、固定 manifest 校验、缓存复探测、固定来源下载、安全解压、原子提交：已接通。
- 显式路径无效时不回退：已实现并有单元测试。
- 共享准备作业、跨进程安装锁、最后一个订阅释放时取消作业、下载断点保留和 staging 失败清理：复用既有组件并已串接。任务 032 继续负责把自动部署 DELETE 的取消意图映射为订阅释放及持久化 RuntimeProgress。
- CUDA 不降级：通过新增提交前 preflight 返回 `unsupported_backend`，并有单元测试。
- 进程启动、readiness、health、路由、停止及模型 GGUF 下载：未复制实现，继续使用既有职责链。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets`：通过。
- `cargo test -p burncloud-server --lib`：通过，166 passed、0 failed、3 ignored。
- `cargo test -p burncloud-server --doc`：通过，6 passed、0 failed、1 ignored。
- `cargo test --workspace`：未通过。Windows 构建期间出现页面文件不足（`os error 1455`，Rust 无法映射 `core/std` 元数据），随后产生级联 `E0463/E0462`；同时 `burncloud-tests` 的既有集成测试源码出现大量标准库 prelude 缺失编译错误。未修改这些无关构建或测试文件。
- 未进行 Gitee 实际下载、四平台运行或真实模型对话验收；这些证据属于任务 036 的真实部署门禁。

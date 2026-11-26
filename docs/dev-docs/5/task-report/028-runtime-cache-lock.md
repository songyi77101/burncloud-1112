# 028 实现运行时缓存、安装锁和共享准备作业

## 修改的信息

- 新增方案二运行时缓存核心组件：按版本、target、package_kind 和归档 SHA-256 组成隔离 key，并实现 `downloads/`、版本缓存目录、`.staging/`、`.locks/` 与 `.quarantine/` 路径约定。
- 默认缓存根解析到当前用户数据目录下的 `BurnCloud/runtimes`；支持管理员环境变量 `BURNCLOUD_RUNTIME_CACHE_DIR`，要求绝对路径，并在创建目录前后校验规范路径不得与安装目录、项目目录或 `target` 目录重叠。
- 缓存命中必须同时验证固定大小的 `ready` 标记、受限大小的 `runtime.json`、key 身份、普通文件类型、归档大小和 SHA-256、服务端相对路径及解压目录边界；无效缓存移入隔离目录，不覆盖其他版本。
- staging 由 RAII guard 管理；提交前再次校验归档和服务端文件，先同步写入 `runtime.json`，最后写入 `ready`，再通过同文件系统目录改名原子提交。失败时清理本次 staging；未完成 staging 即使含有伪造的 `ready` 也不会被当作最终缓存命中。
- 按缓存 key 使用标准库文件锁实现跨进程互斥等待，并支持取消等待；同一进程内通过 `RuntimeInstallRegistry` 共享安装 worker、状态、进度和取消信号。订阅引用计数保证单个父任务取消不影响其他订阅者，最后一个订阅者取消才通知共享作业停止。
- 缓存组件暂未接入现有 `LlamaCppRuntimePreparer`；按执行顺序由任务 031 完成下载、解压、探测和部署器接入。源文件对此设置了带原因的 `dead_code` lint 说明，接入后应移除。

## 修改的文件与文件内位置

- `crates/interfaces/server/src/node_runtime.rs`：新增私有 `cache` 子模块声明；保留现有运行时准备器和部署生命周期不变。
- `crates/interfaces/server/src/node_runtime/cache.rs`：新增 `RuntimeCacheKey`、`RuntimeCacheSpec`、缓存根与目录布局解析、缓存元数据/完整性校验、损坏缓存隔离、staging guard 与原子提交、key 级文件锁、共享安装 registry/订阅/进度/取消实现。
- `crates/interfaces/server/src/node_runtime/cache_tests.rs`：新增 11 项隔离单元测试，覆盖 key/路径、缓存根安全边界、ready 和元数据验证、SHA-256、坏缓存隔离、旧版本保留、staging 原子提交和重启残留、安装锁等待与取消、共享作业去重和订阅取消。
- `docs/dev-docs/5/task-report/028-runtime-cache-lock.md`：本任务报告。

## 检查结果

| 验收项 | 结果 |
| --- | --- |
| 用户数据目录、管理员缓存根和路径禁区 | 通过；默认根候选、相对覆盖拒绝、项目目录覆盖拒绝及安全绝对覆盖均有测试。 |
| 缓存 key 与规定布局隔离 | 通过；key 由版本/target/package_kind/SHA 组成，下载 partial 与 sidecar 路径、最终目录层级均有测试。 |
| ready、runtime.json、文件边界、归档大小和 SHA-256 | 通过；缓存命中逐项核验，异常缓存被隔离；坏新缓存不影响旧有效版本。 |
| staging 生命周期和提交原子性 | 通过；staging guard 清理未提交目录；提交前核验归档和服务端文件；失败不生成最终缓存，成功后目录按 key 原子提交。 |
| 安装锁与取消 | 通过；测试覆盖独立文件句柄竞争、锁释放后等待者取得锁，以及等待时取消。生产实现使用操作系统文件锁，锁文件保留以避免并发删除锁文件造成锁对象分裂。 |
| 共享安装作业 | 通过；相同 key 返回同一 job id、只运行一个 worker、共享进度/结果；释放一个订阅不会取消，释放最后一个订阅会通知取消。 |
| Rust 格式与静态检查 | 通过：`cargo fmt --all -- --check`；`cargo clippy --workspace --all-targets -j 1`。 |
| 运行时缓存单元测试 | 通过：`cargo test -p burncloud-server --lib node_runtime::cache::tests -j 1 -- --test-threads=1`，11 项通过。 |
| 全工作区测试 | 本次 `cargo test --workspace -j 1` 的单元测试和集成测试均通过；最终在 `burncloud-node-runtime` doctest 阶段失败：49 个 doctest 中 25 个通过、24 个失败，Windows 链接器报告包含 `LNK1102`（内存不足）。失败发生在既有 doctest 编译/链接，不涉及本任务源码；未修改无关 doctest 或构建配置。 |
| Gitee 实际下载与真实部署 | 未执行；真实四平台验收属于任务 036。 |

## 后续边界

- 任务 029 负责 ZIP/TAR.GZ 安全解压；任务 030 负责 llama-server 版本、依赖和 backend 实测探测。缓存完整性检查已预留固定服务端路径校验，但不替代后续探测。
- 任务 031 将本模块接入 `LlamaCppRuntimePreparer`，需在缓存命中和新安装返回前执行运行时探测，并在完整安装生命周期持有同 key 文件锁。
- 任务 032/034 继续映射运行时阶段进度并扩展取消/共享行为的接口集成测试。
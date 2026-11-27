# 030 llama-server 版本、依赖与设备探测任务报告

## 修改的信息

- 新增私有运行时探测模块，按 manifest 提供的相对路径定位 `llama-server`，拒绝越界路径、符号链接、目录以及非普通文件，并确认最终可执行路径位于解压根目录内。
- 使用 Tokio 子进程直接执行固定参数 `--version`、`--help`；macOS 非 CPU 请求额外执行 `--list-devices`。不通过 shell 执行，不把子进程原始诊断或路径带入稳定错误文本。
- 为探测子进程设置 15 秒超时、最多保留 1 MiB 输出、并发排空 stdout/stderr；取消或超时会终止并等待子进程。
- 新增受控 `RuntimeProbeSummary`，记录版本首行摘要、help/依赖探测结果、Metal 设备数、实际 backend 和 RFC 3339 探测时间。
- backend 选择遵循目标平台：Windows/Linux 使用 CPU；macOS `auto` 根据设备探测结果选 Metal 或 CPU；显式 Metal 无设备返回 `runtime_device_unavailable`；CUDA 与未支持 backend 返回 `unsupported_backend`。
- 将 `tokio-util` 加入 server crate 依赖以接收现有取消令牌。探测器仍为内部实现，由任务 031 接入运行时准备器。

## 修改的文件与文件内位置

- `crates/interfaces/server/Cargo.toml`：新增 `tokio-util.workspace = true`。
- `crates/interfaces/server/src/node_runtime.rs`：声明私有 `probe` 子模块，并注明由任务 031 接入。
- `crates/interfaces/server/src/node_runtime/probe.rs`：新增探测结构、稳定错误映射、manifest 路径定位、backend 判定、限量输出读取、子进程超时/取消清理及探测编排。
- `crates/interfaces/server/src/node_runtime/probe_tests.rs`：新增独立测试模块，覆盖 backend 选择、路径和文件类型约束、输出限制、错误脱敏、Metal 解析及可运行平台上的进程探测/取消。
- `docs/dev-docs/5/task-report/030-runtime-probe.md`：本任务验收记录。

## 检查结果

| 验收项 | 结果 |
| --- | --- |
| 仅定位 manifest 指定服务端，路径必须在解压根内且为普通文件 | 通过；测试覆盖缺失路径、目录、路径穿越、绝对/驱动器路径、错误文件名和符号链接。 |
| `--version`/`--help` 退出成功且输出有效，并记录依赖探测状态 | 通过；Unix 测试夹具执行端到端探测并校验摘要。当前 Windows 主机未执行 Unix 专用脚本夹具，四平台真实包验证留待任务 036。 |
| Windows/Linux CPU 与 macOS Metal/auto 选择及错误码 | 通过；纯单元测试覆盖四个目标映射、CUDA/未知 backend、Metal 无设备和 auto 回退。Metal 设备输出格式需在 macOS 实机验收中复核。 |
| 输出长度限制、超时/取消清理及错误脱敏 | 通过；输出截断、取消状态和稳定错误文本测试通过；Unix 子进程终止回收测试因当前主机为 Windows 未运行。 |
| Rust 格式与静态检查 | 通过：`cargo fmt --all`、`cargo fmt --all -- --check`、`git diff --check`、`cargo clippy --workspace --all-targets -j 1`。 |
| 探测单元测试 | 通过：`cargo test -p burncloud-server --lib node_runtime::probe::tests -j 1 -- --test-threads=1`，当前 Windows 主机 9 项通过。 |
| 全工作区测试 | 未全部通过：`cargo test --workspace -j 1` 的单元与集成测试执行通过，最终在既有 `burncloud-node-runtime` doctest 阶段失败。单独复测为 18 项通过、31 项失败，报错包含 Windows Rust doctest 依赖产物不兼容（`E0460`/`E0462`）和 linker `LNK1318` PDB LIMIT；失败文档位于 `crates/platform/node/`，不涉及本任务文件。未改动无关构建配置。 |
| macOS Metal 实际设备和四平台真实包 | 未执行；归属任务 036。 |

## 后续边界

- 任务 031 将把 manifest、下载、缓存、安全解压与探测串接到 `LlamaCppRuntimePreparer`，并映射到现有部署准备结果。
- 当前 Windows 测试不能替代 macOS Metal、Linux 子进程行为或四平台 Gitee 真实归档验收；这些仍须按任务 034、036 执行并如实留证。

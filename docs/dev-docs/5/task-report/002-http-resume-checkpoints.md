# 002 实现连续断点证据与续传判定

## 修改的信息

- 新增私有 HTTP 断点证据和 Range 判定。只有相同原始 URL、相同最终实体 URL、匹配的强 ETag、连续字节数和一致的完整后缀范围才允许追加；无可靠证据的 206/416 必须重发完整请求。
- 检查点先同步部分文件，再同步并原子替换同目录侧车。恢复时截掉未提交尾部，短文件、坏侧车或无原生证据的文件从零开始。侧车不证明最终 SHA-256。
- 保留现有下载错误，增加 Transport 与 Io 源错误。服务端沿用 download_failed 响应，不暴露签名地址或本地路径。
- 按任务验收要求更新必要的穷尽匹配；服务端改动仅限既有模型下载错误映射和回归测试，不改接口或任务清单。

## 修改的文件与文件内位置

- 下载 crate：`src/http.rs` 的 `resume_decision`、`restore_checkpoint` 和 `persist_checkpoint`；独立测试在 `src/http/checkpoint_tests.rs`。`src/error.rs` 增加错误链；`tests/error_sources.rs` 和 README 同步。
- 服务端：`src/api/model_download.rs` 的 `task_error_from_download` 与独立的 `model_download_tests.rs`。

## 检查结果

| 验收项 | 本地结果 |
| --- | --- |
| 200、强/弱 ETag、原始或实体地址变化、合法或非法 206 | 独立测试通过；不能把 206 或 416 响应体当完整文件 |
| 416 仅有强实体和完整连续长度证据才能待校验；短体、超长体、协议错误 | 独立测试通过 |
| 先同步再原子替换；未提交尾部截断；短文件、缺失或损坏侧车安全重下 | Windows 临时目录测试通过；错误保留原检查点，不使用旧 aria2 文件长度 |
| 新错误保留原始 source；服务端不泄露底层信息 | 错误链单元测试、文档测试及服务端回归通过 |

- Windows x64，默认 feature，离线 Cargo。`cargo fmt --all --check` 和 `cargo clippy -p burncloud-download -p burncloud-server --all-targets --offline` 通过。
- `cargo test -p burncloud-download --offline`：35 项通过（含 9 项文档测试）。`cargo test -p burncloud-server --lib api::model_download:: --offline`：10 项通过。
- 任务清单及依赖校验通过。此任务不切换现有 aria2 门面、不进行真实网络下载或部署验收，原生传输和门面接入分别留在任务 003 和 004。
- 首次提交门禁：独立构建目录的全工作区主程序构建通过；`cargo test --workspace --no-default-features` 在预提交钩子中因 `burncloud-code` 既有用例 `worktree_subdirectory_uses_common_hooks` 报 `index.lock` 路径不存在而失败（截至中止 288 通过、1 失败）。同一测试二进制在钩子外单独运行该用例通过。当时未提交或推送。
- 已单独复现既有测试的失败条件：继承提交钩子的相对 `GIT_INDEX_FILE=.git/index` 时失败，不继承时通过。验证时只为该独立仓库测试进程隔离此变量；预提交的暂存范围检查仍读取原索引，钩子和测试源码均不修改。
- 重跑真实预提交钩子：全工作区构建和原 Git 工作树用例通过；工作区测试运行到 752 通过、4 失败、2 忽略后中止。4 个失败均为既有 `burncloud-download-aria2` 守护进程用例，测试连接到了本机 `localhost:6800` 上已运行的 aria2，进入“接管已有实例”而未启动测试桩。未更改既有运行服务、测试源码或钩子；没有提交或推送。
- 在钩子外完整运行该 aria2 单测二进制，仍复现“接管已有实例”导致的失败；监听进程的启动时间早于本轮验证。单独执行 `cargo clippy --workspace --all-targets --no-default-features --offline -- -D warnings` 和 `cargo deny check` 均通过。
- 独立 aria2 前置修复 `3ca78018` 保持生产真实发现不变，将进程监管测试与机器上已有服务隔离。原提交钩子通过：受影响范围 1044 项通过、0 失败、143 项忽略，全工作区严格 Clippy 和 Cargo Deny 通过；未停止本机已有服务。随后恢复本任务的暂存改动并重新运行提交门禁。
- 恢复本任务后的首次全工作区门禁在 Windows 链接阶段因验证盘磁盘空间不足中止（os error 112，MSVC 同时报 PDB 链接错误）；尚未运行测试。保留原日志，只压缩本任务旧验证目录的生成 PDB，不更改项目构建配置、钩子或正在运行的服务。

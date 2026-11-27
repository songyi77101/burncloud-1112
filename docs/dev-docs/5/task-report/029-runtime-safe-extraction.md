# 029 运行时 ZIP/TAR.GZ 安全解压任务报告

## 修改的信息

- 新增运行时归档安全解压模块，支持 Windows ZIP 和 macOS/Linux TAR.GZ，并按固定 target 校验归档格式。
- 对归档路径执行 UTF-8、长度、空字节、绝对/UNC/驱动器路径、`.`、`..`、空段与冒号检查；只允许 staging 下普通目录和普通文件。
- 拒绝 ZIP 符号链接、Unix 特殊类型，以及 TAR.GZ 符号链接、硬链接和其他非普通文件 entry；文件通过 `create_new` 写入，重复或冲突路径失败关闭。
- 默认上限固定为路径 4096 字节、entry 100000、单文件 2 GiB、总展开 4 GiB；逐块累计真实展开长度并检查取消状态。
- 解压到同一 staging 下新建的 `extracted` 目录。失败时 RAII guard 只清理本次解压目录；不会递归清理缓存根或其他目录。成功返回前精确验证 manifest 的 `server_relative_path` 指向 staging 内普通文件。
- 模块目前由 `node_runtime.rs` 声明并标记暂未接入；任务 031 将把它连接到运行时准备器，之后移除暂时的 dead-code 标注。

## 修改的文件与文件内位置

- `crates/interfaces/server/Cargo.toml`：新增 ZIP、TAR.GZ 解压所需的 `zip`、`flate2` 和 `tar` 依赖。
- `crates/interfaces/server/src/node_runtime.rs`：声明私有 `archive` 子模块，并说明由任务 031 接入。
- `crates/interfaces/server/src/node_runtime/archive.rs`：新增 `RuntimeArchiveLimits`、`RuntimeArchiveError`、`extract_runtime_archive`、格式/路径/entry 类型校验、受限文件写入、staging 清理及服务端文件验证。
- `crates/interfaces/server/src/node_runtime/archive_tests.rs`：独立测试模块，新增 12 项测试，覆盖路径穿越、ZIP/TAR.GZ 普通文件、链接和特殊 entry、格式与 manifest 路径校验、资源上限、取消清理及旧缓存保护。
- `docs/dev-docs/5/task-report/029-runtime-safe-extraction.md`：本任务报告。

## 检查结果

| 验收项 | 结果 |
| --- | --- |
| 拒绝绝对、UNC/驱动器、空字节、`.`、`..`、空路径段和越界路径 | 通过；路径校验测试覆盖跨平台恶意路径形式。 |
| 拒绝符号链接、硬链接及其他特殊 entry | 通过；ZIP 符号链接和 TAR.GZ 符号/硬链接测试均验证返回 `runtime_archive_invalid` 且 staging 被清理。 |
| 路径、entry、单文件、总展开限制 | 通过；上限定义为 4096 字节、100000、2 GiB、4 GiB，测试以较小限额验证超限快速拒绝。 |
| target/归档格式匹配，manifest 服务端路径存在且为普通文件 | 通过；格式不匹配、路径缺失及非普通文件场景均有测试并返回 `runtime_archive_invalid`。 |
| 失败清理不触及有效缓存或 staging 外文件 | 通过；失败只清理本次 `extracted`，回归测试确认旁置 sentinel/旧缓存内容保留。 |
| Rust 格式与静态检查 | 通过：`cargo fmt --all`、`cargo fmt --all -- --check`、`git diff --check`、`cargo clippy --workspace --all-targets -j 1`。 |
| 归档解压单元测试 | 通过：`cargo test -p burncloud-server --lib node_runtime::archive::tests -j 1 -- --test-threads=1`，12 项通过。 |
| 全工作区测试 | 单元测试与集成测试执行通过；`cargo test --workspace -j 1` 最终在既有 `burncloud-node-runtime` doctest 编译/链接阶段失败，7 项 doctest 通过、42 项失败。Windows linker 报告 `LNK1318`（PDB LIMIT）及 Rust doctest 编译/链接错误；不涉及本任务归档测试。未修改无关构建配置。 |
| Gitee 下载和多平台真实部署 | 未执行；归属任务 036。 |

## 后续边界

- 任务 030 继续实现 llama-server 身份、依赖、版本和 backend 探测；安全解压成功不代表运行时已可执行。
- 任务 031 负责将解压器与下载、缓存、探测和原子提交串接，并复用现有部署生命周期。

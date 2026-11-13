# 001 修复 normalize_download_dir 的 Windows 返回路径格式

## 修改的信息

- 修复 Windows 恢复链路中普通盘符路径、`\\?\` 扩展长度路径和 `//?/` 路径表示不一致导致的模型根目录边界误判。
- 在既有目录安全校验、目录创建、canonicalize 和最终模型根目录检查顺序不变的前提下，统一 Windows 输入路径的扩展长度表示，并在全部安全检查通过后转换为普通 Windows 路径。
- Windows 路径转换同时处理反斜杠与斜杠形式、盘符绝对路径和 UNC 路径；转换结果若仍保留扩展长度前缀则返回 `InvalidDirectory`。
- 非 Windows 编译目标不执行 Windows 专用转换；未修改 `ensure_existing_path_inside`、`normalize_aria2_path`、下载调用、恢复流程、任务去重或数据库逻辑。

## 修改的文件与文件内位置

- `crates/platform/storage/download/src/utils.rs`
  - `normalize_download_dir`：保留现有安全检查顺序，在最终边界检查成功后调用普通 Windows 路径转换。
  - `to_extended_windows_path`：统一普通盘符、UNC、`\\?\` 与 `//?/` 输入表示，保证前置路径比较使用一致的 Windows 路径形式。
  - `to_regular_windows_path`、`append_windows_path_units`、`normalize_windows_separators`：新增私有路径格式转换辅助逻辑，转换失败不返回未验证路径。

## 检查结果

- 登记分支：`feat/6-windows-normalize-download-dir`。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`：通过。
- `cargo test -p burncloud-download --all-targets --offline`：通过，Windows 目标下 15 项单元测试、1 项公有接口测试全部通过。
- `cargo check -p burncloud-download --target x86_64-pc-windows-msvc --offline`：通过。
- 对照失败报告：普通 Windows 绝对下载目录在恢复时统一为与 canonicalize 一致的路径表示，避免恢复、重建、失败和并发链路在边界检查阶段提前返回 `InvalidDirectory`。
- 对照任务验收标准：不新增公有接口，不改变现有公有函数签名或错误类型；路径安全校验、模型根目录边界和非 Windows 行为保持不变。

## 回流修复记录（implement）

- 针对失败报告中的 Windows 路径表示不一致，补充普通路径分支的分隔符归一化：即使 `canonicalize` 已返回普通 Windows 路径，也转换为统一的反斜杠表示后再交给下载链路。
- 未改变模型根目录、安全校验、目录创建、canonicalize 顺序及公有接口；扩展长度路径仍在最终边界检查成功后转换。
- 回流检查：`cargo fmt --all`、`cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`、`cargo test -p burncloud-download --all-targets --offline` 均通过。
## 回流修复记录（implement，2026-11-14）

- 针对失败报告中的 `T5b`，恢复缺失 GID 时检测 aria2 返回的 GID 是否仍等于数据库旧值；发生复用时移除匹配任务并重新创建，若仍复用则返回可诊断的恢复错误，并保留数据库更新失败时的任务清理。
- 针对失败报告中的 `T8`、`T9`，完成任务监控对 aria2 返回文件路径增加 Windows 路径规范化、下载目录边界校验、同名目标安全兜底和有限次数的文件系统可见性重试，避免完成事件早于 Windows 文件可见性时误判为失败。
- 未新增公有接口，未改变 `normalize_aria2_path`、下载 URL 校验、任务去重和数据库字段；修改仅位于下载实现工作区。

### 回流检查结果

- `cargo fmt --all`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`：通过。
- `cargo test -p burncloud-download --all-targets --offline`：通过，15 项单元测试与 1 项公有接口测试全部通过。

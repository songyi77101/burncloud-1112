# 001 修复 normalize_download_dir 的 Windows 返回路径格式

## 修改的信息

- 修复 Windows 下恢复记录传入普通盘符绝对路径时，与 `canonicalize` 返回的 `\\?\` 扩展长度路径表示不一致导致的误判。
- 对 Windows 绝对输入在进入既有候选路径边界检查前统一为扩展长度路径表示，使 `starts_with`、`ensure_existing_path_inside`、目录创建和最终 canonicalize 继续使用同一套路径语义。
- 保留最终模型根目录边界检查；仅在全部安全检查成功后，将返回值中的 `\\?\` 前缀转换为普通 Windows 路径，UNC 路径保留 UNC 语义。
- 非 Windows 编译目标保持原有 `PathBuf` 返回结果；未修改 `ensure_existing_path_inside`、`normalize_aria2_path`、下载调用、恢复流程、任务去重或数据库逻辑。

## 修改的文件与文件内位置

- `crates/platform/storage/download/src/utils.rs`
  - `normalize_download_dir`：Windows 绝对输入在候选路径比较前统一路径表示。
  - `to_extended_windows_path`：新增私有 Windows 路径表示转换辅助函数。
  - 最终 canonicalize 和模型根目录边界检查之后的 Windows 返回路径格式转换逻辑保持在原位置。

## 检查结果

- 登记分支：`feat/6-windows-normalize-download-dir`。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`：通过。
- `cargo test -p burncloud-download --all-targets --offline`：通过，Windows 目标下 15 项单元测试、1 项公有接口测试全部通过。
- `cargo check -p burncloud-download --target x86_64-pc-windows-msvc --offline`：通过。
- 对照失败报告：普通 Windows 绝对下载目录可与 canonicalize 的扩展长度路径统一表示，恢复、失败、拒绝和并发链路不再因路径前缀比较提前返回 `InvalidDirectory`。
- 对照任务验收标准：目录安全校验顺序、模型根目录边界、错误类型、公开函数签名和非 Windows 行为保持不变；未新增公有接口。
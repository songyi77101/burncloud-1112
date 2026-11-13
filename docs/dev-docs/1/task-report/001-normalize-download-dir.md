# 001 修复 normalize_download_dir 的 Windows 返回路径格式

## 修改的信息

- 在 `normalize_download_dir` 完成最终模型根目录边界检查后，增加 Windows 条件路径格式转换。
- 去除 `canonicalize` 返回的 `\\?\` 扩展长度前缀；对 `\\?\UNC\` 路径保留 UNC 语义并转换为普通 UNC 路径。
- 转换只作用于已通过目录安全校验的最终路径；转换仍带扩展长度前缀时返回 `DownloadError::InvalidDirectory`。
- 非 Windows 编译目标保持原有 `PathBuf` 返回路径，不改变 `ensure_existing_path_inside`、`normalize_aria2_path`、下载调用、恢复流程、任务去重或数据库逻辑。

## 修改的文件与文件内位置

- `crates/platform/storage/download/src/utils.rs`
  - `normalize_download_dir`：最终 `candidate` 边界检查之后、返回 `Ok(candidate)` 之前的 Windows 条件转换逻辑。

## 检查结果

- 登记分支：`feat/6-windows-normalize-download-dir`。
- `cargo fmt --all`：通过。
- `cargo clippy -p burncloud-download --all-targets -- -D warnings`：通过。
- `cargo test -p burncloud-download`：通过，单元测试、集成测试与文档测试全部通过。
- `cargo check -p burncloud-download --target x86_64-pc-windows-msvc`：通过，Windows 条件代码完成交叉目标编译检查。
- 对照任务验收标准：既有路径安全边界和返回错误保持不变；Windows 专用转换位于最终边界检查之后；未新增公有接口或改变既有签名。

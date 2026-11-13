# 002 增加 normalize_download_dir 回归测试

## 修改的信息

- 为 Windows 条件测试增加普通路径断言，确认 `normalize_download_dir` 的 `models`、`models/sub` 和模型根目录内绝对路径返回值不带 `\\?\` 前缀。
- Windows 测试通过再次 `canonicalize` 返回路径与期望目录进行比较，验证去除前缀没有改变目录指向和路径语义。
- 非 Windows 保留原有 canonicalize 结果比较；父目录组件和模型根目录之外的绝对路径拒绝测试、测试目录清理逻辑及既有下载路径传递 / `normalize_aria2_path` 覆盖保持不变。

## 修改的文件与文件内位置

- `crates/platform/storage/download/tests/support/dir_norm.rs`
  - 新增 Windows 条件辅助断言 `assert_regular_windows_path`。
  - 更新 `resolves_download_directories_relative_to_the_executable`，按目标平台分别验证普通路径格式与 canonicalize 后的路径等价性。

## 检查结果

- `cargo fmt --all`：通过。
- `cargo clippy -p burncloud-download --all-targets -- -D warnings`：通过。
- `cargo test -p burncloud-download`：通过；Windows 条件回归测试、现有单元测试、集成测试与文档测试全部通过。
- `cargo check -p burncloud-download --target x86_64-pc-windows-msvc`：通过。
- 对照任务验收标准：覆盖 `models`、`models/sub`、模型根目录内绝对路径、`../x` 与越界绝对路径；使用现有清理逻辑，测试代码位于独立测试目录，未修改 `normalize_aria2_path` 预期。

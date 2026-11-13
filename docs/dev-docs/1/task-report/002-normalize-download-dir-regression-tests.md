# 002 增加 normalize_download_dir 回归测试

## 修改的信息

- 在 Windows 条件测试中增加扩展长度路径与普通盘符路径之间的测试输入转换，确保绝对目录回归场景真实覆盖恢复记录使用的普通 Windows 路径格式。
- 保留并继续验证 `models`、`models/sub` 和模型根目录约束内绝对目录的返回结果不带 `\\?\` 前缀，且 canonicalize 后仍指向同一目录。
- 越界绝对路径改用普通 Windows 路径输入验证拒绝行为；父目录组件、测试目录清理、非 Windows 路径比较和既有下载路径传递测试保持不变。

## 修改的文件与文件内位置

- `crates/platform/storage/download/tests/support/dir_norm.rs`
  - 新增 Windows 条件辅助函数 `regular_windows_path`，将测试输入转换为普通 Windows 路径以覆盖失败报告中的恢复场景。
  - 更新 `resolves_download_directories_relative_to_the_executable` 的绝对路径与越界绝对路径调用。

## 检查结果

- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`：通过。
- `cargo test -p burncloud-download --all-targets --offline`：通过，Windows 目标下目录规范化测试、其他单元测试与公有接口测试全部通过。
- `cargo check -p burncloud-download --target x86_64-pc-windows-msvc --offline`：通过。
- 对照任务验收标准：覆盖普通 Windows 绝对路径、`models`、`models/sub`、父目录组件与模型根目录外路径；测试代码位于独立测试目录，未修改生产源码中的测试代码或 `normalize_aria2_path` 预期。
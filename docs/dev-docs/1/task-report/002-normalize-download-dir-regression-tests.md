# 002 增加 normalize_download_dir 回归测试

## 修改的信息

- 在 Windows 条件测试中同时覆盖 canonicalize 可能产生的 `\\?\` 与 `//?/` 扩展长度路径输入，转换为普通盘符路径后验证规范化目录可被再次解析。
- 增加模型根目录普通绝对路径的重复规范化场景，模拟数据库恢复记录保存并重新读取 `download_dir` 的路径传递链路。
- 保留并继续验证 `models`、`models/sub`、模型根目录内绝对目录、父目录组件与模型根目录外路径；测试清理逻辑和非 Windows 路径断言保持不变。

## 修改的文件与文件内位置

- `crates/platform/storage/download/tests/support/dir_norm.rs`
  - `regular_windows_path`：兼容 `\\?\`、`//?/`、UNC 和不同分隔符形式的 Windows 路径输入。
  - `resolves_download_directories_relative_to_the_executable`：增加模型根目录普通绝对路径及重复规范化回归覆盖，验证返回路径不带扩展长度前缀且仍指向 executable 同级 `models`。

## 检查结果

- `cargo fmt --all`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`：通过。
- `cargo test -p burncloud-download --all-targets --offline`：通过，16 项测试全部通过（15 项单元测试、1 项公有接口测试）。
- `cargo check -p burncloud-download --target x86_64-pc-windows-msvc --offline`：通过。
- 对照任务验收标准：Windows `models`、`models/sub`、模型根目录内绝对路径、父目录组件与模型根目录外路径均覆盖；已有越界符号链接测试及下载 / 路径传递测试未修改；非 Windows 路径断言保持条件编译；测试代码位于独立测试目录，未修改生产源码中的测试代码或 `normalize_aria2_path` 预期。
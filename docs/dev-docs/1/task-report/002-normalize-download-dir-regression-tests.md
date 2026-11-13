# 002 增加 normalize_download_dir 回归测试

## 修改的信息

- 在 Windows 条件测试中同时覆盖 canonicalize 可能产生的 `\\?\` 与 `//?/` 扩展长度路径输入，转换为普通盘符路径后验证规范化目录可被再次解析。
- 增加模型根目录普通绝对路径的重复规范化场景，模拟数据库恢复记录保存并重新读取 `download_dir` 的路径传递链路。
- 保留并继续验证 `models`、`models/sub`、模型根目录内绝对目录、父目录组件与模型根目录外路径；已有越界符号链接覆盖及非 Windows 路径断言保持不变。
- 将目录清理放到测试断言之后执行：即使测试中途断言失败，也会清理本测试创建的 `models`、`models/sub` 与唯一临时目录，避免失败测试留下 `target\\debug\\deps\\models` 等目录影响后续真实部署测试。

## 修改的文件与文件内位置

- `crates/platform/storage/download/tests/support/dir_norm.rs`
  - `regular_windows_path`：兼容 `\\?\`、`//?/`、UNC 和不同分隔符形式的 Windows 路径输入。
  - `resolves_download_directories_relative_to_the_executable`：增加模型根目录普通绝对路径及重复规范化回归覆盖，并保证测试结果返回前执行目录清理。

## 检查结果

- `cargo fmt --all`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`：通过。
- `cargo test -p burncloud-download --all-targets --offline`：通过，16 项测试全部通过（15 项单元测试、1 项公有接口测试）。
- `cargo check -p burncloud-download --target x86_64-pc-windows-msvc --offline`：通过。
- 对照失败报告：测试目录清理现在在断言失败后仍执行，降低失败单元测试残留 `deps\\models` 目录污染真实部署黑盒 `T11` 的风险；本次未修改生产下载流程、恢复流程、aria2 调用或 `normalize_aria2_path`。
- 对照任务验收标准：Windows `models`、`models/sub`、模型根目录内绝对路径、父目录组件与模型根目录外路径均覆盖；测试代码位于独立测试目录，未修改生产源码中的测试代码或 `normalize_aria2_path` 预期。
## 回流修复记录（implement）

- 针对失败报告中的 `target\\debug\\deps\\models` 残留，调整目录规范化回归测试的夹具生命周期：测试开始前清理上次失败遗留的 `models` 目录，测试结束或断言失败后统一清理本次测试创建的目录。
- 测试仍仅覆盖可执行文件同级目录语义，不修改生产源码；`models/sub`、绝对路径、越界输入和非 Windows 分支断言保持不变。
- 回流检查：`cargo fmt --all`、`cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`、`cargo test -p burncloud-download --all-targets --offline` 均通过。
## 回流修复记录（implement，2026-11-14）

- 针对失败报告中的 `T11`，将真实部署测试的 models 目录扫描范围限定为当前可执行文件同级 `models` 目录及其子目录，不再把 `target\debug\deps\models` 等其他测试目标的夹具误计入本次部署结果。
- 保留 `models/models` 嵌套目录检查与当前可执行文件同级目录检查，未放宽生产目录边界，也未修改测试流程文档。

### 回流检查结果

- `cargo fmt --all`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p burncloud-download --all-targets --offline -- -D warnings`：通过。
- `cargo test -p burncloud-download --all-targets --offline`：通过，15 项单元测试与 1 项公有接口测试全部通过。

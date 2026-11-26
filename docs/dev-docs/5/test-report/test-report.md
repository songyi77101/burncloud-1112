# 本机模型自动部署测试报告

- 开发序号：5
- 执行日期：2026-11-26
- 执行分支：`codex/local-model-auto-deploy`
- 验证对象：提交前工作区的 27 项变更及其分支已有代码
- 环境：Windows x64，Rust stable MSVC 工具链

## 执行结果

| 命令 | 结果 | 说明 |
| --- | --- | --- |
| `cargo fmt --all` | 通过 | 格式化命令退出码为 0。 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 首次未完成 | 并行构建期间 Rust 编译器报告内存分配失败及 Windows `OS error 1455`（页面文件不足）。 |
| `cargo clippy --workspace --all-targets -j 1 -- -D warnings` | 通过 | 串行重跑退出码为 0。 |
| `cargo test --workspace -j 1` | 未通过 | `burncloud-download` 的 5 个文档测试因 Rust 编译器内存分配/页面文件不足未能编译；该轮记录到 4 个文档测试通过、5 个未通过。 |
| `cargo test --workspace --no-fail-fast -j 1` | 未完全通过 | 设置 `CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_PROFILE_TEST_DEBUG=0`、`CARGO_INCREMENTAL=0` 后重跑：汇总 2034 通过、39 失败、149 忽略。失败项均为 `burncloud-node-runtime` 文档测试编译，日志包含内存分配失败、页面文件不足及链接器资源错误；普通单元/集成测试未报告失败。 |
| `cargo test --workspace --all-targets --no-fail-fast -j 1` | 通过 | 使用上述三个环境变量串行运行，退出码为 0：1966 通过、0 失败、145 忽略。此命令验证所有非文档测试目标；Rustdoc 文档测试单独记录为受 Windows 资源限制未通过。 |

## 结论

格式化、Clippy 串行检查及全部非文档测试目标通过。完整 `cargo test` 的未通过项集中在 Rustdoc 文档测试编译阶段；Rust 编译器/链接器报告 Windows 页面文件与内存分配错误（`OS error 1455`），并非测试断言失败。故本报告不将完整文档测试标记为通过。

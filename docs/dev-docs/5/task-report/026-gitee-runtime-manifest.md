# 026 登记 Gitee 固定运行时 manifest

## 修改的信息

- 将运行时清单从历史 b8581/GitHub schema 迁移为方案二 schema v2。
- 固定服务端来源为 Gitee 仓库 `songyi777/llama-cpp`、commit `51d01f6e526bc6d45b9d252b1d2d487efb6625d8`、受管版本 `b12757`。
- 登记 Windows x64、macOS x64、macOS ARM64、Ubuntu x64 四个目标的固定文件名、压缩格式、字节数、SHA-256 和解压后的 `llama-server` 相对路径。
- 删除历史 CUDA 资源映射；清单只允许当前 Gitee 可取得的 CPU/Metal 目标，CUDA 后续由运行时准备器返回 `unsupported_backend`。
- 更新运行时说明，明确固定 raw URL、服务端校验边界、来源优先级、缓存安全边界和历史 b8581 方案的替代关系。

## 修改的文件与文件内位置

- `deploy/runtime/manifest.json`：schema v2 根字段、全局归档限制、四目标 `supported_backends` 与 `package` 登记。
- `deploy/runtime/README.md`：固定来源、四目标资源表、manifest 校验契约、CUDA/xcframework 边界、资源摘要复核方式和历史方案说明。
- `docs/dev-docs/5/task-report/026-gitee-runtime-manifest.md`：本任务验收记录。

## 检查结果

| 验收项 | 结果 |
| --- | --- |
| 固定 provider、仓库、commit、version | 通过；Python 标准库校验 manifest 根字段与固定值。 |
| 四目标文件名、格式、大小、SHA-256、服务端路径 | 通过；逐项与方案二接口契约登记值比对，四个摘要均匹配 64 位小写十六进制规则。 |
| CUDA 与 xcframework 边界 | 通过；targets 仅包含四个目标，未生成 CUDA 或 xcframework 条目；README 明确 CUDA 返回 `unsupported_backend`。 |
| 非客户端可控的 URL、版本和摘要 | 通过；manifest 使用固定 commit + 文件名 URL 模板，README 明确禁止请求体覆盖。 |
| 资源重新计算约束 | 通过契约登记；README 给出使用实际归档重新计算长度和完整 SHA-256 的命令，并明确禁止使用 Git blob id。实际下载、流式重算和断点恢复由后续任务 027 实现。 |
| manifest JSON 结构 | 通过；Python 结构校验脚本执行成功。 |
| Rust 格式与静态检查 | `cargo fmt --all -- --check` 通过；`cargo clippy --workspace --all-targets` 通过。 |
| Rust 全量测试 | 单线程执行 `cargo test --workspace -j 1` 时单元/集成测试通过；Windows doctest 在链接阶段因 `LNK1318: 非预期的 PDB 错误: FILE_SYSTEM (3)` 失败，属于当前环境链接器临时文件问题，未发现业务断言失败。 |

## 后续边界

- 本任务不实现 manifest 的 Rust 解析、target/backend 选择或错误枚举；这些由任务 027–031 接入运行时准备器。
- 本任务不下载、解压、探测或启动 `llama-server`，不代表任何平台已完成真实部署验收。

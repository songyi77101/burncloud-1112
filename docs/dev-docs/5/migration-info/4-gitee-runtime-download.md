# 方案二迁移说明：Gitee 运行期获取 llama.cpp

> 开发序号：5；迁移编号：4；标题短名：gitee-runtime-download。
> 日期：2026-11-26；本文件是对既有 `local-model-auto-deploy` 设计的增量迁移记录。
> 依据：用户已确认的《BurnCloud 方案二：Gitee 运行期获取 llama.cpp 自动部署技术书》以及 `docs/research/llama-cpp-cross-platform.md`。
> 注意：本迁移不修改 `dev-context.md`。该文件仍登记旧项目根目录 `E:/project/burncloud1.1` 与分支 `codex/local-model-auto-deploy`；当前工作区为 `E:/project/burncloud1.2`，且本地仅存在 `main`。该不一致交由 spec-manager 流程修正。

## 一、迁移目的

将开发序号 5 原有“每个发行物内嵌 b8581 基础运行包、Windows CUDA 按需下载”的运行时策略，替换为“客户端运行期从固定 Gitee commit 获取 llama.cpp 运行包，完成校验、缓存、解压、探测后启动服务”的方案二。

本记录只记录差异和替代关系，不在本文件重复既有自动部署、数据库、模型下载、进程生命周期与路由设计。新设计见同目录：

- `gitee-runtime-download-flowchart.md`
- `gitee-runtime-download-interface-contract.md`
- `gitee-runtime-download-dev-doc.md`
- `task-list/026-*.md` 至 `task-list/036-*.md`
- `task-list/gitee-runtime-download-execution-order.md`

## 二、设计差异

| 设计项 | 原设计（历史基线） | 方案二（本次生效设计） | 迁移影响 |
| --- | --- | --- | --- |
| 运行时来源 | 发行物内嵌 b8581 基础包；Windows CUDA 使用固定 GitHub 包 | Gitee 仓库 `songyi777/llama-cpp` 的固定 commit `51d01f6e526bc6d45b9d252b1d2d487efb6625d8` | 运行时不再依赖发行物内嵌基础压缩包；客户端不跟踪 `master` |
| 受管版本 | `b8581` | `b12757` 资产名，另外绑定上述不可变 commit | 版本和 commit 均由服务端 manifest 维护，客户端不能提交或覆盖 |
| 支持目标 | Windows x64 CPU、macOS x64/ARM64、Ubuntu x64 CPU；另有 Windows CUDA 12.4 | Windows x64 CPU、macOS x64、macOS ARM64、Ubuntu x64 | 四个 CPU/原生目标保持；Gitee 没有 CUDA 资产 |
| Gitee 资产 | 不参与生产部署 | 使用四个 `llama-b12757` 运行包 | `xcframework` 仅作仓库资产，不用于桌面 `llama-server` |
| CUDA | Windows 可按需下载服务包和 cudart | 当前返回 `unsupported_backend`，不下载、不降级 | CUDA 第二来源和双包接口保留为后续增强，不在本轮实现 |
| 下载实现 | 原设计有发行期准备、内嵌和现有下载门面 | 运行期独立原生 `reqwest` 流式下载 | 不启动、不依赖 aria2；运行时下载目录与模型下载目录分离 |
| 断点恢复 | 主要面向模型下载及旧下载任务 | 运行时 `.part` 与元数据独立持久化，要求强 ETag、Range、Content-Range、连续长度一致 | 远端实体变化时安全重下，不拼接不同实体 |
| 缓存位置 | 发行物或既有受管目录 | 用户数据根下的 `runtimes/` | 不写安装目录、项目目录或 `target` |
| 来源优先级 | 显式配置、内嵌或既有缓存 | `BURNCLOUD_LLAMA_SERVER`、有效缓存、Gitee 固定 commit | 显式配置无效时报告错误，不静默换用其他来源 |
| 运行时进度来源 | `bundled`、`explicit`、`cache`、`downloaded` | `explicit`、`cache`、`downloaded` | 旧契约中的 `bundled` 只保留在历史文档，不再作为当前来源值 |
| 失败回退 | 缺少内嵌包时报告 `bundled_runtime_missing` | Gitee 下载、校验、解压或探测失败即失败 | 不回退到内嵌运行时，不回退到 CPU 掩盖 CUDA 请求 |
| HTTP 边界 | 自动部署 POST/GET/DELETE 与 resolve | 完全复用现有接口 | 不新增 runtime 管理 API；新增逻辑显示为 `preparing_runtime` 子进度 |

## 三、固定 Gitee 资源登记

生产部署只允许使用服务端可信 manifest 中的登记值。以下表格是方案二的初始登记基线；编码任务 026 将把它落入 `deploy/runtime/manifest.json` 的新 schema，实际提交前仍须由资源准备工具重新读取固定 commit、重新计算并核对摘要。

| Cargo target | backend 选择 | package_kind | 文件名 | 压缩格式 | 大小（字节） | SHA-256 | `server_relative_path` |
| --- | --- | --- | --- | --- | ---: | --- | --- |
| `x86_64-pc-windows-msvc` | `cpu` | `cpu` | `llama-b12757-bin-win-cpu-x64.zip` | `zip` | `25262139` | `5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d` | `llama-server.exe` |
| `aarch64-apple-darwin` | `cpu` / `metal` | `native` | `llama-b12757-bin-macos-arm64.tar.gz` | `tar.gz` | `16862081` | `873b413c5d4a336ae623bedde8f755a3ed81b18bc6ae4b01f5a0f6fcea63754c` | `llama-b12757/llama-server` |
| `x86_64-apple-darwin` | `cpu` / `metal` | `native` | `llama-b12757-bin-macos-x64.tar.gz` | `tar.gz` | `12591660` | `f330d7bf9baff5cc401c06524618f9eaf8962439dcb9e11fa638f8e3b0bee14b` | `llama-b12757/llama-server` |
| `x86_64-unknown-linux-gnu` | `cpu` | `cpu` | `llama-b12757-bin-ubuntu-x64.tar.gz` | `tar.gz` | `23425964` | `99e34cc6a39f3afda735d43d76d15d6ead2297406cc9c0cf421b771fd3a1a84a` | `llama-b12757/llama-server` |

上表中的四个 SHA-256 均为固定 commit 对应归档的实际 SHA-256。实现任务 026 仍必须重新读取归档并计算摘要；若工具输出与登记值不一致，任务必须失败，不得用网页显示值或 Git blob id 替代归档 SHA-256。

固定 raw URL 规则：

```text
https://gitee.com/songyi777/llama-cpp/raw/51d01f6e526bc6d45b9d252b1d2d487efb6625d8/<filename>
```

客户端不传入 `<filename>`；文件名由服务端根据 target、backend 和 manifest 选择。

## 四、保留、替代与废弃关系

### 4.1 继续有效

以下旧设计不因方案二改变：

- 自动部署 POST/GET/DELETE 接口的 URL、任务 ID、幂等、取消和重启 `interrupted` 语义；
- `DeploymentService` 的进程启动、readiness、health、`/v1/models`、`/slots`、路由绑定、监控和停止；
- 模型 GGUF 的权威身份、下载、校验、缓存、预算与现有模型下载接口；
- 任务父状态 `queued/running/succeeded/failed/cancelled/interrupted` 和部署阶段 `preflight/preparing_runtime/downloading_model/verifying/starting/health_checking/attaching_route`；
- 旧运行时的许可证、依赖、四平台真实部署和 `cargo fmt/clippy/test` 门禁要求。

### 4.2 被本修订替代

以下旧任务或章节在“运行时准备”部分不再按原设计执行：

- 原 `005-fixed-runtime-catalog.md` 中的 GitHub/b8581 运行时登记；
- 原 `006-prepare-release-runtime-assets.md` 与 `007-bundle-target-runtime.md` 中的客户端内嵌基础包；
- 原 `008-safe-runtime-extraction.md`、`009-runtime-probes-backend-adapter.md`、`010-managed-runtime-preparation.md` 中依赖内嵌资源或 Windows CUDA GitHub 来源的部分；
- 原接口契约中 `RuntimeProgress.origin = bundled` 与 `bundled_runtime_missing` 的当前运行时语义；
- 原 `deploy/runtime/manifest.json` 中 b8581/GitHub 来源的生产登记。

上述旧文件和历史任务报告不删除、不重写，以便追溯；编码阶段以本迁移文档和 `gitee-runtime-download-*` 三份设计文档为准。

### 4.3 本轮明确不做

- 不在客户端调用 Gitee API 动态发现版本；
- 不追踪 `master`；
- 不允许请求体携带版本、commit、URL、SHA、解压路径或可执行路径；
- 不用 `git clone` 安装；
- 不用 Gitee `contents` Base64 传输大文件；
- 不实现 Gitee CUDA 压缩包；
- 不把 `xcframework` 当作桌面 `llama-server`；
- 不以内嵌运行时作为默认回退；
- 不新增运行时管理 HTTP API；
- 不修改 `dev-context.md` 或历史 `task-report`。

## 五、实施前置与交付状态

本迁移记录与配套技术书只完成设计和任务登记，不代表代码、下载、真实部署或四平台验收已经完成。编码阶段必须：

1. 先将资源登记表的所有 SHA-256 用固定 commit 的实际文件重新计算并校验；
2. 按增量任务 026–036 的拓扑顺序实现；
3. 在代码中同步中文 rustdoc、模块 README、根 README 和接口说明；
4. 完成单元、集成和四平台真实部署验收后，才能把目标标记为已支持。

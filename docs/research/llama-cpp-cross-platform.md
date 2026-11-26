# llama.cpp 跨平台发行与二次开发研究

> 研究日期：2026-11-26。本文核对 ggml-org/llama.cpp 官方 README、构建文档、GitHub Releases 页面，并结合本仓库现有运行时清单。上游分支与发行资产会变化；发行决策应以实际下载时的官方页面、固定 tag/commit 和文件摘要复核为准。

## 结论摘要

- `.../tree/master` 是仓库某分支的浏览入口，不是稳定版号、固定源码快照或二进制发行包。`master` 会继续前进；Gitee 镜像还可能按手动/自动策略同步，必须确认具体仓库的同步状态。本次已对用户给出的 Gitee 仓库与上游仓库执行 HEAD 查询；两边提交不同，详见第 2.2 节。[Gitee 镜像管理说明](https://help.gitee.com/repository/settings/sync-between-gitee-github)；[上游 README](https://github.com/ggml-org/llama.cpp/blob/master/README.md)
- 截至本次检索，GitHub `latest` 稳定发行是 **v1.1.0**；当日官方预发布二进制页为 **b12774**，标记为 `Pre-release`。README 的发行标识区分 `v1` 稳定发行与 `b*` nightly；不能把最新 nightly 当作最新稳定版。[稳定版](https://github.com/ggml-org/llama.cpp/releases/tag/v1.1.0)；[b12774 预发布](https://github.com/ggml-org/llama.cpp/releases/tag/b12774)
- 官方预编译资产覆盖 Windows x64/ARM64、macOS Apple Silicon/Intel、Ubuntu x64/ARM64，以及多个 GPU/加速后端；资产名中的 OS、架构和后端必须同时匹配。Linux 资产标签明确为 Ubuntu，不能据此承诺任意 Linux 发行版可直接运行。[b12774 资产列表](https://github.com/ggml-org/llama.cpp/releases/tag/b12774)；[构建说明](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)
- 本仓库当前受管运行时仍锁定 **b8581**，其目标仅为 Windows x64 CPU、macOS arm64/x64、Ubuntu 24.04 x64 CPU，Windows CUDA 12.4 为单独按需包。本文不变更此锁定；升级必须单独完成兼容性、依赖、许可和目标机验收。[运行时登记](../../deploy/runtime/manifest.json)；[受管运行时说明](../../deploy/runtime/README.md)

## 1. 仓库文档约定与研究文件位置

仓库 `docs/` 当前已有编号的 `docs/dev-docs/<开发序号>/` 开发流程资料，没有通用研究目录。为避免把背景调查混入开发序号 5 的已确认需求/任务文档，本报告放在 `docs/research/llama-cpp-cross-platform.md`；本次仅新增此研究 Markdown，不改 README、业务源码或运行时清单。

## 2. Gitee `master` 链接代表什么

### 2.1 分支浏览地址，不是版本或发行物

如果链接形式是 `https://gitee.com/<owner>/<repo>/tree/master`，其语义通常是浏览该仓库 `master` 分支当前指向的文件树；它指向一个可移动分支名，而非固定 commit、SemVer tag（如 `v1.1.0`）或 GitHub Release 附件。Git 中固定版本应使用 tag/commit；上游也分别提供源码仓库与独立的预编译 Releases 页面。[Gitee Git 基本概念](https://help.gitee.com/Git%20%E7%9F%A5%E8%AF%86%E5%A4%A7%E5%85%A8/Git%E7%9A%84%E5%9F%BA%E6%9C%AC%E6%A6%82%E5%BF%B5%E3%80%81%E5%B8%B8%E7%94%A8%E5%91%BD%E4%BB%A4%E5%8F%8A%E5%AE%9E%E4%BE%8B)；[上游源码](https://github.com/ggml-org/llama.cpp/tree/master)；[上游 Releases](https://github.com/ggml-org/llama.cpp/releases)

### 2.2 镜像新鲜度不能从 URL 推断

Gitee 官方帮助说明，GitHub→Gitee 镜像可以配置自动同步，也可手动更新，并可涉及分支、标签和提交记录；所以某个 Gitee 仓库的 `master` 是否追上 GitHub，取决于该镜像的配置和最近一次成功同步。本次查询到 Gitee `master` HEAD 为 `29b52d398b294509746ec0a3f28948a3107c0213`，上游 GitHub `master` HEAD 为 `28148b5f06666bf44e0975f2980960672c1f94d9`；两边提交不同，说明此刻不是同一快照，但仅凭不同 hash 不判断哪边领先。用于发行仍应直接选择上游固定 release/tag 及其资产摘要，而不是依赖 `master` 镜像页面。[Gitee 镜像管理说明](https://help.gitee.com/repository/settings/sync-between-gitee-github)

## 3. 稳定版与 nightly / pre-release

| 类型 | 本次检索观察 | 如何使用 |
| --- | --- | --- |
| 稳定发行 | `releases/latest` 指向 **v1.1.0**，GitHub 将其列为最新稳定发行。README 的 Release 标识筛选 `tag:v1`。 | 生产默认从稳定 tag 开始；仍须固定具体资产、校验摘要并验证目标平台，不把“稳定”误解为对所有 OS/后端都测试过。 |
| Nightly / 预发布 | **b12774** 页面明确标记 `Pre-release`，页面记录发布于 2026-11-26；README 的 Nightly 标识筛选 `b*`。v1.1.0 发行页也指向该 nightly build。 | 用于尝鲜、回归验证或确实需要新修复时；锁定 b-tag/commit 与资产摘要，升级前做目标机验证，不将滚动的 nightly 标签称为稳定版。 |
| 源码主分支 | `master` 是上游正在演进的开发分支；不是 release tag。 | 仅在需要最新源码/参与开发时使用；构建记录须固定 commit 和构建选项。 |

官方 README 当前分别列出 Release `tag:v1` 与 Nightly `b*`；Live Releases 页面将 `b12774` 标作 `Pre-release`，而 `v1.1.0` 是最新稳定 tag。上游 `docs/release.md` 的“目前 GitHub Releases 只发布 nightly”文字与当前 Live Releases 页面不一致，显然是较旧流程说明；判断当前可下载资产时应以 Live Releases/tag 页面为准。[README](https://github.com/ggml-org/llama.cpp/blob/master/README.md)；[稳定版 v1.1.0](https://github.com/ggml-org/llama.cpp/releases/tag/v1.1.0)；[预发布 b12774](https://github.com/ggml-org/llama.cpp/releases/tag/b12774)；[release 流程文档](https://github.com/ggml-org/llama.cpp/blob/master/docs/release.md)

> **时间快照：**版本、资产和“latest”会变化。本报告记录 2026-11-26 查询结果；链接仍指向实时页面，后续阅读时页面可能已更新。

## 4. 官方预编译包、架构与后端选择

README 将预编译下载入口指向 GitHub Releases，并同时提供从源码构建的说明。以下资产名称/矩阵据 b12774 官方预发布页；该页属于预发布，不等于 v1.1.0 稳定 tag 的保证。[README 下载入口](https://github.com/ggml-org/llama.cpp/blob/master/README.md)；[b12774 官方资产列表](https://github.com/ggml-org/llama.cpp/releases/tag/b12774)

| 平台 | 本次看到的官方包/架构 | 后端与选择注意事项 |
| --- | --- | --- |
| Windows | x64 CPU ZIP（[下载](https://github.com/ggml-org/llama.cpp/releases/download/b12774/llama-b12774-bin-win-cpu-x64.zip)）；ARM64 CPU；ARM64 OpenCL Adreno。 | x64 有 CUDA 12（另配 CUDA 12.4 DLL）、CUDA 13（另配 CUDA 13.5 DLL）、Vulkan、OpenVINO、SYCL、ROCm 10.0 资产；ARM64 有 CUDA 13、Vulkan 等资产。NVIDIA 包与对应版本 DLL 必须成套，不能跨 CUDA 大版本拼包。 |
| Linux | Ubuntu x64 CPU（[下载](https://github.com/ggml-org/llama.cpp/releases/download/b12774/llama-b12774-bin-ubuntu-x64.tar.gz)）、Ubuntu arm64 CPU（[下载](https://github.com/ggml-org/llama.cpp/releases/download/b12774/llama-b12774-bin-ubuntu-arm64.tar.gz)）；另有 Ubuntu 24.04/26.04 s390x CPU、Ubuntu 26.04 s390x zDNN。 | x64/arm64 有 Vulkan；x64 有 CUDA 12（另配 CUDA 12.8 libraries）、CUDA 13（另配 CUDA 13.5 libraries）、ROCm 10.0、OpenVINO、SYCL FP32/FP16；arm64 有 CUDA 13。资产以 Ubuntu 标记，不应默认等同 Alpine/musl 或所有 Linux ABI。Linux arm64 Snapdragon 条目链接到 setup guide，不是该页列出的常规预编译发行包。 |
| macOS | Apple Silicon arm64（[下载](https://github.com/ggml-org/llama.cpp/releases/download/b12774/llama-b12774-bin-macos-arm64.tar.gz)）；Intel x64（[下载](https://github.com/ggml-org/llama.cpp/releases/download/b12774/llama-b12774-bin-macos-x64.tar.gz)）。 | 按机器原生架构选包。官方构建文档说明 macOS 构建默认启用 Metal；实际运行仍应在目标机枚举/验证设备，不能只因系统是 macOS 就假定 GPU 路径成功。 |
### 本机可执行性冒烟验证（Windows x64）

2026-11-26 从官方 b12774 资产下载 Windows x64 CPU ZIP（25,288,075 字节，SHA-256：`44b70b20f3374eec3d16a002beceefef33cc3565678caeb3eea5f6cacba61836`），解压后运行 `llama-server.exe --version` 成功，输出 `version: 1.1.0-dev (build 12774, commit 171527522)`；`--help` 也能正常显示 `--model`、`--host`、`--port` 和 `--list-devices` 参数。该验证只证明 Windows 二进制可启动、CLI 可用；没有 GGUF 模型因此未做真实推理，也没有在 macOS/Linux 主机实测对应资产。

### 后端快速选择

- **CPU**：不依赖特定 GPU 运行时，适合作为最低复杂度基线；仍须核对 OS ABI、CPU 架构/指令集与包的运行依赖。官方构建文档把 CPU 列为基础构建路径。[CPU 构建](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#cpu-build)
- **CUDA**：仅用于匹配的 NVIDIA GPU、驱动和对应 CUDA runtime 组合。上游把 CUDA 作为独立构建后端；发布页把某些 CUDA libraries/DLL 单独列出，应把主包与标注的依赖一起核验。[CUDA 构建说明](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#cuda)；[b12774 资产](https://github.com/ggml-org/llama.cpp/releases/tag/b12774)
- **Metal**：macOS 专用 GPU 后端；按 Apple Silicon/Intel 架构下载对应包。源码构建时 Metal 默认启用，可通过 `GGML_METAL` 关闭。[Metal 构建说明](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#metal-build)
- **Vulkan**：跨厂商图形计算路径，但仍依赖目标机 GPU、驱动和 Vulkan 环境；不能当成“任意显卡必然加速”。[Vulkan 构建说明](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#vulkan)
- **OpenVINO / SYCL / ROCm / OpenCL 等**：按发布包写明的设备厂商、架构与后端选；不是一个可任意互换的“GPU 版”。上游构建说明分别描述各后端及依赖。[完整构建指南](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)
- 运行时能力应在实际机器验证；上游文档提供 `--list-devices` 设备枚举入口。没有检测到期望设备时应给出明确错误/回退策略，不要仅凭所下载的文件名宣称 GPU 已启用。[构建指南：多后端与设备枚举](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)

## 5. 面向本仓库的二次开发建议

1. **将版本、目标、架构、后端、资产 URL、长度和 SHA-256 一起锁定。**避免从 `master` 或 `latest` 动态拼运行时下载地址；如果更新到 nightly，使用明确 b-tag/commit，经过验收再提升；稳定发行同样按固定资产摘要落锁。当前 [manifest](../../deploy/runtime/manifest.json) 已采用固定版本和源包 SHA 的思路，应保留这种不可变资源身份。
2. **保持“基础包”与 GPU 可选包分离。**根据产品承诺先覆盖 CPU 基线，再按需求增加 CUDA/Metal/Vulkan 等目标变体；不要给不支持的平台暗示能力。当前需求资料将 Windows x64、macOS arm64/x64、Ubuntu 24.04 x64 作为既定基础目标，CUDA 独立按需准备；不要因为上游 nightly 有更多资产就自动扩产品支持范围。[开发需求](../dev-docs/5/burncloud-1112-13-issue.md)；[运行时资源约定](../../deploy/runtime/README.md)
3. **升级前做接口/行为兼容验收。**至少记录运行时版本输出、server 参数与帮助输出、模型加载、健康检查、OpenAI 兼容端点、流式响应、取消/停止行为和目标后端设备枚举。若直接集成公开 C API，要参考上游 SemVer/兼容策略；若通过 `llama-server` 子进程集成，则不能把 C API 的 SemVer 规则误当成 REST/CLI 完全不变的承诺。[README 的 server/API 入口](https://github.com/ggml-org/llama.cpp/blob/master/README.md)；[发布过程与 SemVer 说明](https://github.com/ggml-org/llama.cpp/blob/master/docs/release.md)
4. **每个发行目标单独做干净环境验证。**在无开发工具链、无预装 llama.cpp、无旧运行时缓存的目标机测试启动与真实推理；记录 OS 版本、CPU 架构、驱动、后端、包 SHA、动态依赖及实际设备。上游源码构建指南展示了不同平台/后端的前置条件，因此“能下载/能编译”不能替代目标机部署验收。[构建指南](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)；[本仓库发行门禁设计](../dev-docs/5/task-list/024-four-target-release-gates.md)
5. **把依赖、许可和签名作为发行输入，而不是事后备注。**逐项审核压缩包中可执行文件及动态库的再分发条件、许可证/NOTICE、Windows runtime、Linux `.so`、macOS 签名/公证；校验解压路径边界和文件摘要。当前本仓库运行时说明明确指出上游许可证不自动覆盖所有随包 DLL/依赖，锁文件必须针对最终实际打包内容生成。[受管运行时发行说明](../../deploy/runtime/README.md)

## 6. 部署风险清单

| 风险 | 影响 | 控制措施 |
| --- | --- | --- |
| 把 `master`/Gitee 镜像页当固定发行 | 分支会移动；镜像可能未同步；源码页也不等于经过构建的二进制。 | 固定上游 release tag、commit 和 asset SHA；镜像只作代码浏览/受控加速，并验证来源与摘要。[Gitee 镜像说明](https://help.gitee.com/repository/settings/sync-between-gitee-github) |
| 把 nightly 当稳定版 | `b*` 是预发布构建，可能包含快速变化或回归；“最新”不等于适合生产。 | 稳定优先 v1 tag；nightly 单独标记、单独验收和回滚。[README 版本标识](https://github.com/ggml-org/llama.cpp/blob/master/README.md)；[b12774 页面](https://github.com/ggml-org/llama.cpp/releases/tag/b12774) |
| 架构/ABI 不匹配 | 例如 x64/ARM64 混用、Ubuntu 包部署到不兼容的 libc/发行版，可能无法启动或加载动态库。 | 按 OS + CPU 架构选包；盘点实际动态依赖；在最低支持系统版本验证。Linux 包标签只声明 Ubuntu 变体。[官方发行资产](https://github.com/ggml-org/llama.cpp/releases/tag/b12774) |
| GPU 包与机器不匹配 | 驱动、CUDA runtime、GPU 型号或 Vulkan/Metal 能力不符合要求，可能启动失败或静默跑 CPU。 | 核对包旁注明的 CUDA libraries/DLL；启动后枚举设备并执行真实推理；区分显式 GPU 失败与 auto 回退。[build.md](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)；[发行资产](https://github.com/ggml-org/llama.cpp/releases/tag/b12774) |
| 依赖/授权遗漏 | 主程序能启动仍不代表随包 DLL/共享库具有再分发许可；安全软件或系统签名策略可能阻止执行。 | 对最终归档做完整库存、SHA、依赖、license/NOTICE、签名/公证审计；不要以 llama.cpp 主仓库许可证替代所有传递依赖审核。[本仓库运行时发行规则](../../deploy/runtime/README.md) |
| 未固定下载内容或缓存被替换 | 镜像、代理或重复下载内容变化可能导致错误包进入运行时目录。 | 对下载长度和 SHA-256 做强制校验；安装后复核库存/路径；缓存身份绑定版本与目标，失败不覆盖已验证资源。[本仓库固定资源约定](../../deploy/runtime/README.md) |
| 只测 CPU/只测编译 | 不能证明 CUDA、Metal、Vulkan 或其他目标上的设备探测、健康检查和真实推理可用。 | 每个 OS/架构/后端独立列验收证据；没测的组合标为未验证，不宣传支持。[本仓库四目标验收任务](../dev-docs/5/task-list/024-four-target-release-gates.md) |

## 7. 可复核的一手资料

- [ggml-org/llama.cpp README](https://github.com/ggml-org/llama.cpp/blob/master/README.md)：项目、安装方式、预编译下载入口、Release/Nightly 标识。
- [官方构建指南 `docs/build.md`](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)：平台、CPU、Metal、CUDA、Vulkan、其他后端与设备枚举。
- [官方最新稳定发行入口](https://github.com/ggml-org/llama.cpp/releases/latest)；本次解析到 [v1.1.0](https://github.com/ggml-org/llama.cpp/releases/tag/v1.1.0)。
- [本次检索到的 b12774 预发布与平台资产](https://github.com/ggml-org/llama.cpp/releases/tag/b12774)。
- [上游 `docs/release.md`](https://github.com/ggml-org/llama.cpp/blob/master/docs/release.md)：SemVer 与 release workflow；注意其 GitHub Releases 发布方式说明与当前页面状态不一致。
- [Gitee 官方仓库镜像管理说明](https://help.gitee.com/repository/settings/sync-between-gitee-github)：镜像同步方向、自动/手动选项与同步内容。
- 本仓库：[运行时 manifest](../../deploy/runtime/manifest.json)、[受管运行时说明](../../deploy/runtime/README.md)、[本机模型部署需求](../dev-docs/5/burncloud-1112-13-issue.md)。

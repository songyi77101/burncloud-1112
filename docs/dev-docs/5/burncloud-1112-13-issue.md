# feat: 完善跨平台本机模型自动部署与内嵌 llama.cpp 运行时

https://github.com/songyi77101/burncloud-1112/issues/13

# BurnCloud 本机模型自动部署完善

## 范围边界
- 本期：四目标各内嵌一份 b8581 基础压缩运行时；Windows CUDA 12.4 显式按需下载；跨平台 HTTP 下载、受管缓存、安全解压、异步部署与持久化、客户端接入、发行门禁。
- 不做：Linux CUDA/ARM64、Alpine、驱动或 Toolkit 自动安装、用户端编译、远程节点、非 GGUF 转换、新安装器体系。

## 背景与目标
用户没有 GitHub 访问能力，也无需预装 llama.cpp。选择目录模型后自动完成运行时准备、模型下载和校验、进程启动、健康检查与路由接入。基础运行时离线提供，模型独立下载。不改动用户显式配置的运行时或系统 PATH。

## 需求与验收标准
| # | 需求 | 必要/可选 | 触发场景 | 行为 | 可观察结果 |
|---|---|---|---|---|---|
|1|跨平台下载|必要|下载模型或 CUDA|reqwest 流式下载，安全续传、取消、恢复，兼容旧记录|无需 aria2；远端变化不会错误拼接|
|2|分平台内嵌|必要|启用 bundled-runtime 构建|CI 预取并核对固定资产；构建不联网且缺失/错误立即失败|每个发行物仅含本平台基础包|
|3|运行时准备|必要|用户首次部署|优先显式配置，其次缓存，否则安全解压内嵌资源并探测|版本/平台/后端缓存共享；失败不覆盖有效版本|
|4|Windows CUDA|可选|用户显式选择|下载固定 CUDA12.4 服务包及 cudart，校验、探测，不静默降级|无 GitHub API 请求；失败明确可见|
|5|异步部署|必要|POST /api/node/models/deployments|服务端校验目录身份和 SHA256；持久化任务；准备模型与运行时后启动、健康检查、接路由|202+task_id；GET 查询真实阶段和两个子任务；DELETE 取消|
|6|并发与恢复|必要|双击/共享准备/重启|复用在途任务；独立取消；重启未完成任务中断|不重复进程，不误删共享资源，不自动重启|
|7|客户端|必要|选择模型/后端|提交异步任务，轮询，显示真实进度与失败原因|刷新可恢复；健康与路由完成前不显示可对话|
|8|发行与验收|必要|四目标发行|Windows x64、macOS arm64/x64、Ubuntu24.04 x64；许可、VC++、签名、体积报告门禁|实机测试结果分平台记录，未测不标通过|

## 已确认方案
候选版本 b8581。Windows/Linux 默认 CPU，macOS auto 优先 Metal。显式运行时配置保留。内嵌方式适配现有单文件更新。下载前按后端检查 RAM/VRAM 与磁盘。任务状态包含 preflight、preparing_runtime、downloading_model、verifying、starting、health_checking、attaching_route；终态 succeeded/failed/cancelled/interrupted。SQLite/PostgreSQL 同步迁移。停止完成部署仍用既有停止接口。

## 发布门禁
必须执行 fmt/clippy/test，补独立回归测试，中文注释、README 与接口文档。四目标及 Windows NVIDIA 真实对话验收独立记录。授权、签名、干净环境与硬件未具备时不得宣称发布可用。

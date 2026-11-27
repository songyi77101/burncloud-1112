# Gitee 运行期获取 llama.cpp 公有接口契约

> 开发序号：5；标题短名：gitee-runtime-download。
> 本文件是 `local-model-auto-deploy-interface-contract.md` 中运行时相关条款的增量修订。自动部署 HTTP URL、方法、父任务状态、取消幂等和模型字段继续沿用旧契约；本文件只覆盖运行时来源、运行时进度、错误语义和 resolve 预算的变化。
> 状态：方案二契约已由用户在当前请求中确认；本文是实现依据，不表示代码或真实部署已经完成。

## 一、HTTP 边界与不变项

| 接口名称 | URL | 方法 | 方案二行为 |
| --- | --- | --- | --- |
| 创建或复用自动部署任务 | `/api/node/models/deployments` | POST | 不变；后台任务中将运行时准备来源改为显式路径、缓存或 Gitee 固定 commit |
| 查询自动部署任务 | `/api/node/models/deployments/{task_id}` | GET | 不变；`data.runtime` 返回修订后的 `RuntimeProgress` |
| 取消自动部署任务 | `/api/node/models/deployments/{task_id}` | DELETE | 不变；只取消当前父任务订阅，不删除其他任务使用的共享运行时 |
| 解析目录模型与部署预算 | `/api/node/models/resolve` | POST | 不变；预算中的运行时包来自固定 Gitee manifest；CUDA 无 Gitee 资源时返回 `unsupported_backend` |

本修订**不新增**以下接口：

- `GET /runtime`、`POST /runtime/install`、`DELETE /runtime/{id}` 等独立运行时管理接口；
- 任意 URL 下载接口；
- 任意本地路径运行接口；
- 客户端自定义版本、commit、SHA-256、压缩包文件名或解压目录接口。

运行时下载、校验、解压、探测和缓存提交均作为自动部署任务的 `preparing_runtime` 子阶段展示。

## 二、可信来源与 manifest 契约

### 2.1 固定来源

| 字段 | 固定值 |
| --- | --- |
| `provider` | `gitee` |
| `repository` | `songyi777/llama-cpp` |
| `commit` | `51d01f6e526bc6d45b9d252b1d2d487efb6625d8` |
| `version` | `b12757`（由四个归档文件名和探测结果共同确认） |
| raw URL 模板 | `https://gitee.com/songyi777/llama-cpp/raw/<commit>/<filename>` |
| API 动态发现 | 禁止用于生产部署；仅资源登记/人工刷新可使用 |
| `xcframework` | 不属于桌面 `llama-server` 资源，不得进入目标映射 |

URL 中的 `<commit>` 和 `<filename>` 均由服务端 manifest 拼接。请求体、客户端本地缓存元数据和 HTTP 查询参数不能覆盖它们。

### 2.2 目标资源表

以下值是服务端可信登记的初始值；实现时必须重新计算并核对。Git blob id、Gitee API 返回的 Git `sha` 和文件 SHA-256 不是同一概念，不能混用。

| `target` | 可选有效 backend | `package_kind` | `filename` | `archive_format` | `size_bytes` | `sha256` | `server_relative_path` |
| --- | --- | --- | --- | --- | ---: | --- | --- |
| `x86_64-pc-windows-msvc` | `cpu` | `cpu` | `llama-b12757-bin-win-cpu-x64.zip` | `zip` | `25262139` | `5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d` | `llama-server.exe` |
| `aarch64-apple-darwin` | `cpu`, `metal` | `native` | `llama-b12757-bin-macos-arm64.tar.gz` | `tar.gz` | `16862081` | `873b413c5d4a336ae623bedde8f755a3ed81b18bc6ae4b01f5a0f6fcea63754c` | `llama-b12757/llama-server` |
| `x86_64-apple-darwin` | `cpu`, `metal` | `native` | `llama-b12757-bin-macos-x64.tar.gz` | `tar.gz` | `12591660` | `f330d7bf9baff5cc401c06524618f9eaf8962439dcb9e11fa638f8e3b0bee14b` | `llama-b12757/llama-server` |
| `x86_64-unknown-linux-gnu` | `cpu` | `cpu` | `llama-b12757-bin-ubuntu-x64.tar.gz` | `tar.gz` | `23425964` | `99e34cc6a39f3afda735d43d76d15d6ead2297406cc9c0cf421b771fd3a1a84a` | `llama-b12757/llama-server` |

### 2.3 manifest 必填字段

服务端 manifest 的每个目标包至少包含：

| 字段 | 类型 | 规则 |
| --- | --- | --- |
| `provider` | string | 只能为 `gitee` |
| `repository` | string | 只能为 `songyi777/llama-cpp` |
| `commit` | string | 40 位小写十六进制，必须等于固定 commit |
| `version` | string | 当前为 `b12757` |
| `target` | string | 只能是四个已登记 Cargo target 之一 |
| `backend` | string | 包族：`cpu` 或 `native`；有效请求后端由 `supported_backends` 决定 |
| `supported_backends` | string[] | Windows/Linux 为 `cpu`；macOS 为 `cpu`、`metal` |
| `filename` | string | 只能是 manifest 登记的基名，不接受路径分隔符 |
| `archive_format` | string | `zip` 或 `tar.gz`，必须与平台匹配 |
| `size_bytes` | uint64 | 非零，必须等于实际归档字节数 |
| `sha256` | string | 64 位小写十六进制，必须等于完整归档 SHA-256 |
| `server_relative_path` | string | 相对路径，不能含 `..`；解压后必须是普通文件 |
| `archive_limits` | object | 至少包含最大路径、entry 数、单文件字节数、总展开字节数 |
| `dependencies` | string[] | 可选的依赖/许可证审计标识，不作为客户端输入 |
| `license_components` | string[] | 可选的许可证组件清单；不替代实际授权审核 |

默认展开限制为：最大路径 4096 字节、最大 entry 100000、单文件 2147483648 字节、总展开 4294967296 字节。若后续 manifest 收紧限制，按包登记值执行。

## 三、状态枚举

### 3.1 父任务状态和阶段

与旧契约保持完全一致：

| 状态 | 枚举值 | 含义 | 是否终态 |
| --- | --- | --- | --- |
| 已登记 | `queued` | 已持久化，等待后台接手 | 否 |
| 执行中 | `running` | 正在准备、启动或清理 | 否 |
| 成功 | `succeeded` | 健康、路由和最终持久化均成功 | 是 |
| 失败 | `failed` | 任一阶段失败，或失败/取消清理失败 | 是 |
| 已取消 | `cancelled` | 取消被接受且本任务资源清理完成 | 是 |
| 已中断 | `interrupted` | 服务重启发现本机未完成任务 | 是 |

父 `stage` 仍只取：

```text
preflight
preparing_runtime
downloading_model
verifying
starting
health_checking
attaching_route
```

运行时下载、校验、解压、探测和提交不能新增父级 stage；详细状态放在 `RuntimeProgress.phase`。

### 3.2 运行时准备 phase

`RuntimePreparationPhase` 是 `RuntimeProgress.phase` 的可选字符串值：

| phase | 含义 |
| --- | --- |
| `cache_checking` | 检查显式路径、缓存根、版本、target、包类型和摘要 |
| `waiting_for_shared_job` | 等待同 key 的共享准备作业；不重复下载 |
| `downloading` | 从固定 Gitee raw URL 流式下载或安全续传 |
| `verifying` | 校验归档大小和完整 SHA-256 |
| `extracting` | 在 staging 目录执行 ZIP/TAR.GZ 安全解压 |
| `probing` | 探测 `llama-server`、动态依赖、版本和目标后端 |
| `committing` | 写入元数据、ready 标记和原子缓存目录 |
| `ready` | 已得到可复用的 `PreparedRuntime` |
| `failed` | 本运行时准备作业失败 |
| `cancelled` | 当前父任务已解除运行时依赖 |
| `interrupted` | 服务重启使当前准备任务中断 |

### 3.3 运行时来源

`RuntimeOrigin` 只能取：

| 值 | 含义 |
| --- | --- |
| `explicit` | 使用管理员设置的 `BURNCLOUD_LLAMA_SERVER`，已通过版本、路径、依赖和 backend 探测 |
| `cache` | 使用用户数据目录中已校验、再次探测通过的受管缓存 |
| `downloaded` | 当前任务从固定 Gitee commit 下载并提交了新缓存 |

旧契约中的 `bundled` 不再返回；不以内嵌运行时作为方案二默认回退。

### 3.4 CUDA 扩展契约

HTTP `backend` 枚举保留 `cuda` 以兼容已有自动部署契约，但本次四目标 Gitee manifest 没有 CUDA 包映射。请求 CUDA 时在 preflight 返回 `unsupported_backend`；不得下载 CPU 包冒充 CUDA，也不得静默切换到 CPU。

后续如果加入 CUDA 来源，内部资源登记必须能表达：来源 provider、固定 commit/tag、服务包和 cudart 两个固定包、各自大小/SHA、驱动要求、双包探测结果。那是单独增强任务，不属于本修订实现。

## 四、RuntimeProgress 契约

### 4.1 字段

`GET /api/node/models/deployments/{task_id}` 的 `data.runtime` 使用下表。基础字段沿用旧契约，方案二对来源、phase 和包身份作如下明确化：

| 字段 | 类型 | 是否必返 | 含义 |
| --- | --- | --- | --- |
| `status` | `TaskStatus` | 是 | 当前父任务对运行时分支的状态；复用父状态枚举 |
| `stage` | `DeploymentStage|null` | 是 | 未开始为 null；运行时执行时固定为 `preparing_runtime` |
| `phase` | `RuntimePreparationPhase|null` | 是 | 运行时内部准备阶段；未开始为 null |
| `detail` | string|null | 是 | 可展示说明，不是额外状态枚举 |
| `task_id` | string|null | 是 | 运行时共享作业不暴露独立 HTTP 任务 ID，固定为 null |
| `completed_bytes` | uint64 | 是 | 当前下载阶段已确认连续字节；缓存命中可为归档大小 |
| `total_bytes` | uint64|null | 是 | manifest 登记的归档总大小；未知时为 null，不能用 0 伪造 |
| `download_speed` | uint64 | 是 | 当前字节/秒；非传输阶段为 0 |
| `actual_sha256` | string|null | 是 | 完整归档实际摘要；未完成或显式路径时为 null |
| `origin` | `RuntimeOrigin|null` | 是 | `explicit`、`cache` 或 `downloaded` |
| `version` | string|null | 是 | 受管运行时为 `b12757`；显式路径为探测到的实际版本 |
| `target` | string|null | 是 | 服务端实际 target；未选择时为 null |
| `backend` | string|null | 是 | `cpu` 或 `metal`；CUDA 当前不会进入运行时准备 |
| `package_kind` | string|null | 是 | `cpu` 或 `native`；未选择时为 null |
| `archive_sha256` | string|null | 是 | manifest 期望摘要；仅用于展示和诊断，不能由客户端反向提交 |
| `error` | `TaskError|null` | 是 | 当前运行时分支的失败/中断原因 |

客户端不得把 `archive_sha256`、`version`、`target` 或 `backend` 回填到下一次请求作为可信输入。响应不包含可直接执行的完整本地路径；`PreparedRuntime` 路径只在服务内部传递给现有部署服务。

### 4.2 进度语义

- 运行时下载使用归档字节数，不把 entry 数、解压字节或时间估算当作网络进度。
- `origin=cache` 时可以 `completed_bytes=total_bytes=size_bytes`，`detail` 必须说明“复用已校验缓存”，不代表本次发生网络传输。
- `origin=explicit` 时 `completed_bytes=0`、`total_bytes=null`、`actual_sha256=null`，因为服务端不把用户显式路径当作受管归档下载。
- Gitee 归档在下载完成后必须重新计算完整 SHA-256；不得把期望摘要复制到 `actual_sha256`。
- 远端实体变化、安全重下或校验失败时 `completed_bytes` 可以回退；客户端不得强制单调增长。
- 共享作业成功后，某个父任务取消不会修改其他父任务的进度；已终态父任务的快照不能被共享作业迟到事件覆盖。

### 4.3 JSON 示例

```json
{
  "status": "running",
  "stage": "preparing_runtime",
  "phase": "downloading",
  "detail": "正在从固定 Gitee 资源下载 llama-b12757-bin-ubuntu-x64.tar.gz",
  "task_id": null,
  "completed_bytes": 10485760,
  "total_bytes": 23425964,
  "download_speed": 5242880,
  "actual_sha256": null,
  "origin": "downloaded",
  "version": "b12757",
  "target": "x86_64-unknown-linux-gnu",
  "backend": "cpu",
  "package_kind": "cpu",
  "archive_sha256": "99e34cc6a39f3afda735d43d76d15d6ead2297406cc9c0cf421b771fd3a1a84a",
  "error": null
}
```

## 五、部署接口详情

### 5.1 创建或复用自动部署任务

========================================

- 接口名称：创建或复用自动部署任务。
- URL：`/api/node/models/deployments`。
- 方法：`POST`。
- 说明：请求只携带模型身份和请求 backend。服务端根据当前平台、固定 manifest、显式环境配置和缓存状态准备运行时；后台在 `preparing_runtime` 阶段报告运行时子进度，准备完成后复用既有 `DeploymentService` 启动和健康检查。
- 幂等规则：继续使用旧契约的规范化 `source + model + revision + variant_id + checksum + backend` 组合键，原子复用当前本机 `queued/running` 任务。运行时版本、commit、URL、SHA 和路径不参与客户端幂等输入，因为它们由服务端可信 manifest 决定。
- 入参列表：

| 名称 | 位置 | 类型 | 是否必填 | 规则 |
| --- | --- | --- | --- | --- |
| `model` | body | string | 是 | 沿用旧契约的模型身份校验 |
| `source` | body | string | 是 | 沿用旧契约支持的模型来源枚举 |
| `revision` | body | string | 是 | 沿用旧契约的安全修订段校验 |
| `variant_id` | body | string | 是 | 沿用旧契约的 GGUF 相对文件标识校验 |
| `checksum` | body | string | 是 | 沿用旧契约的 `sha256:` 期望摘要校验 |
| `backend` | body | string | 否 | `auto`、`cpu`、`metal`、`cuda`；具体可用性由 target/manifest/preflight 决定 |
| `runtime_version` | body | 不允许 | — | 出现即按未知字段拒绝 |
| `runtime_url` | body | 不允许 | — | 出现即按未知字段拒绝 |
| `runtime_commit` | body | 不允许 | — | 出现即按未知字段拒绝 |
| `runtime_sha256` | body | 不允许 | — | 出现即按未知字段拒绝 |
| `runtime_path` | body | 不允许 | — | 出现即按未知字段拒绝 |

- 出参列表：继续返回 HTTP `202`、`success`、`reused` 和 `data` 任务快照；`data.runtime` 使用本文件第四节的 `RuntimeProgress`。
- 状态流转：请求校验和登记成功后父任务进入 `queued`；后台接手进入 `running/preflight`；运行时准备期间进入 `running/preparing_runtime`；准备成功后才可进入后续模型、启动、健康和路由阶段。
- 错误码：请求级错误继续使用 `invalid_deployment_request`、`request_too_large`、`deployment_storage_failed`、`deployment_service_unavailable`；运行时失败写入任务 `data.error`，GET 仍按查询成功返回 HTTP `200`。

========================================

### 5.2 查询自动部署任务

========================================

- 接口名称：查询自动部署任务与运行时子进度。
- URL：`/api/node/models/deployments/{task_id}`。
- 方法：`GET`。
- 说明：天然幂等。返回父任务、模型子任务、方案二 `RuntimeProgress`、当前部署视图和 `ready_to_chat`；不会因查询而触发下载、解压、启动或自动重试。
- 幂等规则：天然幂等；结果随后台进度和当前健康视图变化，历史终态不被重写。
- 入参列表：

| 名称 | 位置 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- | --- |
| `task_id` | path | UUID string | 是 | 服务端生成的自动部署任务 ID；当前本机所有者范围内查找 |

- 出参列表：HTTP `200`、`success:true`、`data`；任务为 `failed`、`cancelled` 或 `interrupted` 时查询仍成功，错误在 `data.error`/`data.cleanup_error` 中。
- 错误码：`invalid_task_id`、`deployment_task_not_found`、`deployment_storage_failed`。

========================================

### 5.3 取消自动部署任务

========================================

- 接口名称：取消尚未完成的自动部署任务。
- URL：`/api/node/models/deployments/{task_id}`。
- 方法：`DELETE`。
- 说明：继续使用旧契约的取消与成功提交串行判定。取消运行时准备只解除当前父任务的订阅；若共享作业仍有其他订阅者，作业继续运行。若该任务已创建自己的进程/路由，仍由既有清理逻辑处理。
- 幂等规则：取消意图已接受但尚未清理完成时重复调用返回 `202`；已 `cancelled` 返回 `200` 与同一快照；`succeeded`、`failed`、`interrupted` 返回 `409`，不改写终态。
- 入参列表：

| 名称 | 位置 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- | --- |
| `task_id` | path | UUID string | 是 | 当前本机所有者范围内的任务 ID |

- 出参列表：清理未完成为 HTTP `202`；清理完成为 HTTP `200`；均返回 `data.runtime` 的最终子状态。
- 错误码：`invalid_task_id`、`deployment_task_not_found`、`deployment_not_cancellable`、`deployment_storage_failed`。

========================================

### 5.4 解析目录模型与部署预算

========================================

- 接口名称：解析目录模型与所选 backend 的部署预算。
- URL：`/api/node/models/resolve`。
- 方法：`POST`。
- 说明：继续只做模型发现、后端筛选和预算预览，不下载、不解压、不安装运行时、不启动进程。`runtime_packages` 必须从服务端固定 manifest 生成；CUDA 在当前 Gitee 来源没有可用包时返回 `reason.code=unsupported_backend`。
- 幂等规则：天然幂等；缓存和硬件变化可能使预算变化，预算不是磁盘预留或运行时授权。
- 入参列表：

| 名称 | 位置 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- | --- |
| 既有 `model/source/revision` 字段 | body | 既有类型 | 按旧契约 | 旧字段和校验不变 |
| `backend` | body | string | 否 | `auto`、`cpu`、`metal`、`cuda`；缺省规则沿用旧契约 |
| `runtime_version` | body | 不允许 | — | 不允许客户端覆盖固定版本 |
| `runtime_url` | body | 不允许 | — | 不允许客户端指定下载地址 |
| `runtime_sha256` | body | 不允许 | — | 不允许客户端指定摘要 |

- 出参列表：保留旧 `success`、`data.status`、`model`、`hardware`、`candidates`、`reason`；每个候选的 `deployment_estimate.runtime_packages` 使用下列字段：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `name` | string | manifest 中的固定文件名 |
| `size_bytes` | uint64 | 固定归档大小 |
| `checksum` | string | `sha256:` 加 64 位小写摘要 |
| `origin` | string | `cache` 或 `downloaded`；显式运行时无受管包时为空 |
| `download_required` | boolean | 当前缓存是否已有有效归档 |
| `provider` | string | 当前为 `gitee` |
| `version` | string | 当前为 `b12757` |

- 错误码：保留旧解析错误；不支持的 target/backend 使用 `unsupported_platform` 或 `unsupported_backend`，不触发隐藏下载。

========================================

## 六、错误响应与错误码

### 6.1 HTTP 请求级错误

继续使用：

```json
{
  "success": false,
  "error": {
    "code": "invalid_deployment_request",
    "message": "中文错误信息"
  }
}
```

| HTTP | code | 触发条件 |
| ---: | --- | --- |
| 400 | `invalid_deployment_request` | JSON、字段、未知字段、backend 或模型身份非法 |
| 400 | `invalid_task_id` | path 中不是有效 UUID |
| 404 | `deployment_task_not_found` | 当前本机所有者不存在该任务 |
| 409 | `deployment_not_cancellable` | 任务已是不可取消终态 |
| 413 | `request_too_large` | 请求体超过旧契约的 16 KiB 上限 |
| 500 | `deployment_storage_failed` | 任务登记、进度或终态无法可靠持久化 |
| 503 | `deployment_service_unavailable` | 服务不能可靠接受后台任务 |

### 6.2 已登记任务的运行时错误

以下错误作为 `TaskError` 写入 `data.error`；查询任务本身仍返回 HTTP `200`。错误消息不能泄露完整本地路径、环境变量值、原始堆栈或密钥。

| code | 阶段 | 含义与处理 |
| --- | --- | --- |
| `unsupported_platform` | `preflight` | 当前 OS/架构不在四目标，不能转源码编译 |
| `unsupported_backend` | `preflight` | target 没有请求 backend 的可信包；CUDA 在当前 Gitee manifest 走此错误，不降级 |
| `runtime_manifest_invalid` | `preflight` | provider、repository、commit、字段、格式、大小或摘要登记非法 |
| `runtime_source_unavailable` | `preparing_runtime` | 固定 commit 的 raw 资源不可访问、403/404 或来源实体不成立 |
| `runtime_configuration_invalid` | `preparing_runtime` | `BURNCLOUD_LLAMA_SERVER` 存在但不是合法可执行文件，或版本/target/backend 探测不匹配 |
| `runtime_download_failed` | `preparing_runtime` | 网络、连接、无进展、重试耗尽或不可恢复 HTTP 错误 |
| `runtime_size_mismatch` | `preparing_runtime` | 实际归档长度与 manifest 不一致；不启用、不覆盖有效缓存 |
| `runtime_checksum_mismatch` | `preparing_runtime` | 完整归档 SHA-256 不一致；安全清理可疑归档 |
| `runtime_cache_invalid` | `preparing_runtime` | ready、元数据、路径、摘要或再次探测不一致；隔离坏缓存后可重新准备 |
| `runtime_archive_invalid` | `preparing_runtime` | 路径穿越、绝对路径、链接、entry 数或展开大小限制失败 |
| `runtime_install_conflict` | `preparing_runtime` | 无法取得同 key 安装锁，或原子提交与其他有效安装冲突 |
| `runtime_probe_failed` | `preparing_runtime` | `llama-server` 版本/帮助、动态库、执行权限或依赖探测失败 |
| `runtime_device_unavailable` | `preparing_runtime` | macOS 显式 Metal 无可用设备/能力；不静默改为 CPU |
| `process_start_failed` | `starting` | 现有部署服务启动失败 |
| `health_check_failed` | `health_checking` | `/health`、`/v1/models` 或 `/slots` 未就绪 |
| `route_attachment_failed` | `attaching_route` | 路由接入或代次校验失败 |
| `cleanup_failed` | 失败/取消清理 | 自有进程或路由不能确认撤销；父任务为 `failed` |
| `service_restarted` | 恢复 | 服务重启发现本机未完成任务；不自动启动，用户需重试 |

### 6.3 内部重试事件不直接作为错误码

`ETag` 缺失、Range 起点不一致、Content-Range 异常、服务器以 200 响应 Range、远端实体变化等首先触发安全重下。只有在有限重试仍不能完成时，才将最终结果报告为 `runtime_download_failed`、`runtime_size_mismatch` 或 `runtime_checksum_mismatch`。客户端不需要理解内部断点事件。

## 七、接口与安全验收

- 对三个自动部署接口执行有效请求、非法 JSON、未知字段、重复字段、非法 backend、重复提交、GET 终态查询、DELETE 幂等和跨本机 owner 隔离测试。
- 验证客户端无法通过 HTTP 指定 Gitee URL、commit、SHA、版本、包名、解压目录或任意可执行路径。
- 验证四目标 manifest 映射、固定 raw URL、大小和 SHA-256 均来自服务端；Gitee Git `sha` 不替代归档 SHA-256。
- 验证运行时进度显示 `cache_checking/downloading/verifying/extracting/probing/committing/ready`，且父 stage 仍为 `preparing_runtime`。
- 验证 CUDA 在当前来源明确返回 `unsupported_backend`，不产生 CPU 运行时成功结果。
- 验证取消 A 不会删除 B 正在使用的共享缓存；服务重启后任务为 `interrupted`，新任务可以复用有效缓存或安全断点。

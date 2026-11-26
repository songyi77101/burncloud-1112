# BurnCloud 本机模型自动部署公有接口契约

> 开发序号：5；关联 Issue：songyi77101/burncloud-1112#13。
> 上游依据：已于 2026-11-24 经用户确认的 `local-model-auto-deploy-flowchart.md`。
> 状态：用户已于 2026-11-24 确认接口契约定稿。本文不表示接口已经实现；下一步编写开发文档，开发文档确认前不进入任务拆分或编码。

## 一、接口清单与兼容边界

| 接口名称 | URL | 方法 | 本期行为 |
| --- | --- | --- | --- |
| 创建自动部署任务 | `/api/node/models/deployments` | POST | 新增；登记或复用在途任务，返回 202 与任务 ID |
| 查询自动部署任务 | `/api/node/models/deployments/{task_id}` | GET | 新增；返回持久化任务、两个子任务与当前同代次路由视图 |
| 取消自动部署任务 | `/api/node/models/deployments/{task_id}` | DELETE | 新增；请求取消未完成任务，清理完成后才进入 cancelled |
| 解析目录模型与部署预算 | `/api/node/models/resolve` | POST | 保留；增加可选 backend，并在候选项中补充部署预算 |

下列既有接口的 URL、方法和原有请求/响应语义不变：

- `POST /api/node/models/download` 及其 GET/DELETE 任务接口。
- `POST /console/api/node/deployments` 手动本地文件部署。
- `GET /console/api/node/deployments/{model_id}` 部署查询。
- `POST /console/api/node/deployments/{model_id}/stop` 已运行模型停止。

自动部署在服务端调用既有能力，不通过 HTTP 自调用手动路径部署入口，也不把客户端路径直接转换成 Verified 工件。没有新增任意 URL 下载、任意本地路径运行、远程节点或驱动安装接口。

## 二、统一协议约束

- 请求和 JSON 响应使用 UTF-8；带请求体的调用使用 `Content-Type: application/json`。
- 新增三个接口返回 `Cache-Control: no-store`，禁止浏览器或代理缓存旧任务状态。
- 新建任务、查询及取消响应统一使用 `{ "success": true, "data": ... }`；POST 另含顶层 `reused` 布尔值，表示本次请求是否复用了在途任务。
- HTTP 请求错误使用 `{ "success": false, "error": { "code": "...", "message": "中文错误信息" } }`。
- `GET` 成功查询到 failed/cancelled/interrupted 任务时仍返回 HTTP 200、`success: true`；部署错误在 `data.error` 中，不混淆“查询成功”和“部署成功”。
- 新增请求的 JSON 体上限为 16 KiB；未知请求字段、错误字段类型、重复字段、非法枚举或格式错误均拒绝，不默默接受 `url`、`model_path`、`executable` 等字段。
- `task_id` 为服务端生成的 UUID 文本，客户端按不透明字符串保存，不与旧下载任务的数字 ID 混用。
- ID 及结果限定在当前本机 Node 所有者内；其他主机的记录按未找到处理，不因共享 PostgreSQL 而跨主机读取、取消或恢复任务。
- 沿用现有 Node 接口的路由信任策略，不新增账户权限体系，也不把任务 ID 当作认证凭证。本期新增接口不代表可以把本机管理服务无保护地暴露公网；已有接口的认证和监听策略不在本文中擅自改写。
- 错误消息不得默认泄露环境变量、完整本地路径、原始堆栈或密钥；底层细节保留在受控日志中。

## 三、状态枚举

### 3.1 任务状态

本表与正式流程图逐一对应，不新增 cancelling、ready、stopped 等父任务状态。

| 状态 | 枚举值 | 含义 | 是否终态 |
| --- | --- | --- | --- |
| 已登记 | queued | 已持久化，等待后台接手 | 否 |
| 执行中 | running | 正在执行或清理取消请求 | 否 |
| 成功 | succeeded | 健康、路由和最终结果持久化均完成 | 是 |
| 失败 | failed | 阶段失败或自有资源清理失败 | 是 |
| 已取消 | cancelled | 取消被接受且自有资源清理完成 | 是 |
| 已中断 | interrupted | 服务重启恢复时发现本机未完成任务 | 是 |

### 3.2 部署阶段

`stage` 仅取：`preflight`、`preparing_runtime`、`downloading_model`、`verifying`、`starting`、`health_checking`、`attaching_route`。

- queued 的初始阶段为 preflight；终态保留结束时的阶段，不引入伪造的“100%阶段”。
- 两分支并行时，父 stage 优先显示尚未完成的 preparing_runtime；运行时就绪后显示模型分支的 downloading_model/verifying。客户端始终分别读取两个子任务，不能用父阶段推断另一分支已经完成。
- 模型、运行时都成功后，父阶段才进入 starting、health_checking、attaching_route。
- 解压、版本/依赖/设备探测和健康检查使用阶段及说明文本，不暴露虚假百分比。

### 3.3 后端

| 字段 | 可选值 | 规则 |
| --- | --- | --- |
| 请求 backend / requested_backend | auto、cpu、cuda | 未提交时默认为 auto；null 或未知值无效 |
| effective_backend | cpu、metal、cuda 或 null | 以实际探测和准备结果为准，尚未确定时为 null |

Windows/Linux 的 auto 仅选择 CPU；macOS auto 优先实测 Metal，确实不可用时使用 CPU 并说明原因。cuda 仅支持 Windows x64，必须显式选择，失败不静默改成 CPU。CPU 请求不得因原生包具备 GPU 能力而自动启用 GPU。

## 四、共享响应类型

### 4.1 DeploymentTaskData

下表字段均返回；尚无结果的可空字段返回 null，不依靠省略字段表示阶段。

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| task_id | string | 稳定部署任务 ID |
| status | TaskStatus | 本任务状态，取 3.1 节枚举 |
| stage | DeploymentStage | 本任务阶段，取 3.2 节枚举 |
| model | string | 规范化后的仓库模型 owner/name |
| source | string | modelscope、hf-mirror 或 huggingface |
| revision | string | 用户所选仓库修订，不静默替换成其他分支或最新版 |
| variant_id | string | 所选 GGUF 仓库相对文件标识 |
| checksum | string | 本次请求期望摘要，规范化为 sha256: 加 64 位小写十六进制；不代表已经实际校验 |
| requested_backend | string | 用户提交的后端，缺省值展开为 auto |
| effective_backend | string 或 null | 当前实际后端，取 3.3 节 |
| backend_reason | string 或 null | auto 后端选择或回退的中文说明，不用该字段掩盖 CUDA 失败 |
| cancel_requested | boolean | 取消意图是否已被服务端接受，不等于清理已完成 |
| model_download | PreparationProgress | 模型分支状态与真实进度 |
| runtime | RuntimeProgress | 运行时分支状态、来源和真实下载进度 |
| estimate | DeploymentEstimate 或 null | 服务端预检后的下载/磁盘预算；预检未完成时为空 |
| deployment | DeploymentView 或 null | 本任务部署标识与同代次的当前进程/路由视图 |
| ready_to_chat | boolean | 只有任务成功且当前同代次进程健康、路由 routable 时为 true |
| error | TaskError 或 null | 阶段失败或中断原因；正常取消不伪造阶段错误 |
| cleanup_error | TaskError 或 null | 清理失败的独立原因，不能覆盖原始阶段错误 |
| created_at | string | RFC 3339 创建时间 |
| updated_at | string | RFC 3339 最近持久化时间 |
| finished_at | string 或 null | 终态提交时间；未终态时为空 |

### 4.2 PreparationProgress 与 RuntimeProgress

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| status | TaskStatus | 该分支对本父任务的状态；复用 3.1 节枚举 |
| stage | DeploymentStage 或 null | 尚未开始为空；模型分支仅 downloading_model/verifying，运行时仅 preparing_runtime |
| detail | string 或 null | 可展示的阶段说明，例如正在解压、验证依赖、探测设备；不是状态机枚举 |
| task_id | string 或 null | 模型分支关联的稳定下载任务 ID；运行时内部共享作业不暴露为独立可取消 HTTP 资源，因此此值为空 |
| completed_bytes | uint64 | 真实已完成字节，不用解压文件数或时间估算替代 |
| total_bytes | uint64 或 null | 已知总字节数；未知时为空，不能写成 0 假装已知 |
| download_speed | uint64 | 当前字节/秒；非传输阶段为 0，不提供伪造 ETA |
| actual_sha256 | string 或 null | 模型文件真实计算出的 SHA-256；不能从期望 checksum 复制填入 |
| error | TaskError 或 null | 本分支错误 |

RuntimeProgress 在上述字段基础上增加：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| origin | string 或 null | explicit、cache、bundled、downloaded；表示本次准备采用的来源 |
| version | string 或 null | 已确认的运行时版本；受管资源为固定 b8581，显式配置的实际版本必须探测后报告 |

进度语义：

- 模型 completed_bytes 表示当前部分/最终文件已有字节，包含经核对后使用的断点；缓存命中时可显示实际文件大小及“复用已校验文件”。estimate 中预计新增网络下载量可以为 0，两者不是同一统计值。
- 运行时 completed_bytes/total_bytes 汇总本次需要在线准备的包，不把内嵌包解压量计入网络下载量；纯内嵌/有效缓存准备为 0/0。
- CUDA 服务包和 cudart 必须全部传输、校验和探测成功后 runtime.status 才能 succeeded，不能以其中一个包完成充当全部完成。运行时没有可合成的单文件摘要，runtime.actual_sha256 始终为空；每个固定包按各自清单摘要校验。
- 远端实体变更导致安全重下时，实际文件进度允许回退；客户端不得通过强制单调增长掩盖重下。
- 父任务取消后，子状态 cancelled 表示本任务已退出该依赖，不表示仍被其他任务使用的共享作业已全局终止；已经 succeeded 的子任务保持成功。
- 父任务终态时不留下仍在排队/运行的本任务子状态：已完成子项保留 succeeded，失败子项保留 failed，未执行或已解除依赖的子项为 cancelled 并说明原因；重启时未完成子项为 interrupted。该规则不改变其他使用者的共享作业状态。
- 终态快照不会被共享作业的迟到进度覆盖。

### 4.3 DeploymentEstimate

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| checked_at | string | RFC 3339 估算/检查时间 |
| model_download_bytes | uint64 或 null | 预计新增模型网络下载量；未知大小为空，不以 0 放行 |
| runtime_download_bytes | uint64 或 null | 预计新增运行时网络下载量；基础包离线准备为 0，CUDA 由固定包与有效缓存计算 |
| runtime_packages | RuntimePackage[] | 所需受管包清单；显式运行时无需受管包时为空 |
| disk_checks | DiskCheck[] | 按真实目标文件系统合并后的额外空间预算与可用空间 |
| requirements | string[] | 可直接展示的依赖及驱动要求，来自固定发行元数据和实际探测，不根据“检测到 NVIDIA”就宣称满足 |

RuntimePackage 字段：name:string、size_bytes:uint64、checksum:string（sha256: 加 64 位小写十六进制）、origin:bundled/download、download_required:boolean。该类型只返回固定资源信息，不接受客户端反向指定包 URL、版本或摘要。Windows CUDA 的包清单必须同时包含服务包和 cudart。

DiskCheck 字段：storage:model/runtime/model_and_runtime、required_bytes:uint64、available_bytes:uint64 或 null、sufficient:boolean 或 null。空间需求含最终新增文件、下载暂存和解压暂存；同一文件系统必须合并计算，不能各自校验通过但合计超量。无法确定可用空间时不得伪报 sufficient:true，任务在传输前以磁盘检查错误结束。

估算只对检查时的状态成立：候选预览不是磁盘预留或模型兼容承诺。自动部署开始传输前必须重新检查；用户确认所选后端不等于授权更换模型、安装驱动或系统提权。

### 4.4 DeploymentView 与 TaskError

DeploymentView 字段：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| model_id | string | 服务端生成的实际部署/路由模型标识，供既有查询与停止接口使用；不从文件名主干推断 |
| generation | uint64 或 null | 本任务持有的进程代次；尚未启动时为空 |
| route_attachment_id | integer 或 null | 本任务绑定的路由标识；不授予取消其他代次路由的权限 |
| process_ready | boolean | 当前同代次进程是否健康就绪；无法取得视图时为 false |
| route_status | string 或 null | 同代次当前路由状态，沿用 unroutable/routable/saturated/quarantined/detaching/detached/failed；无可靠观测时为空 |
| observed_at | string 或 null | 当前视图的 RFC 3339 观测时间；服务重启或代次不匹配时为空 |

任务 succeeded 是历史部署结果，不因后来停止、繁忙或进程退出而改成另一终态。ready_to_chat 必须基于当前同代次视图计算；重启后拿不到当前视图时为 false，不能仅凭数据库里的 succeeded 告诉用户可以对话。查询不得为刷新视图而自动启动进程。客户端推理继续走 BurnCloud 路由，不能把未受管理的本地 endpoint 当作聊天入口。

TaskError 字段仅为 code:string、message:string；可用 code 见第八节。cleanup_error 非空时父任务为 failed，不能在资源未清理完成时返回 cancelled。接口不会把完整本地缓存路径、任意可执行文件或提权命令作为可执行响应字段交给客户端。

## 五、创建自动部署任务

========================================

- 接口名称：创建或复用自动部署任务。
- URL：`/api/node/models/deployments`。
- 方法：POST。
- 说明：同步完成黑盒边界校验、任务查重和持久化登记；耗时目录核对、硬件/磁盘检查和实际准备在后台执行。只有后台权威核对与预检通过后才开始传输。
- 幂等规则：在当前本机 Node 内，以规范化 source、model、revision、variant_id、checksum 和请求后端为键，原子复用 queued/running 任务；取消已请求但未终态的任务仍占用该键。终态任务不重置为 running；用户显式重试创建新任务，复用有效文件与安全断点。
- 并发规则：后端不同的任务可共享运行时和模型文件；同一实际模型与后端的进程启动由现有部署服务串行管理，不能以新 task_id 绕过进程归属和代次检查。
- 入参列表：

| 名称 | 位置 | 类型 | 必填 | 规则 |
| --- | --- | --- | --- | --- |
| model | body | string | 是 | owner/name；复用现有目录模型 ID 校验，每段不超过 128 字节，无空段、控制字符或路径穿越 |
| source | body | string | 是 | modelscope、hf-mirror、huggingface，严格枚举 |
| revision | body | string | 是 | 非空，不超过 128 字节，复用现有安全修订段规则 |
| variant_id | body | string | 是 | 不超过 256 字节，GGUF 相对文件标识；无绝对路径、反斜线、控制字符、空段或 . / .. 段 |
| checksum | body | string | 是 | sha256: 加 64 位十六进制；允许摘要大小写并规范化 |
| backend | body | string | 否 | auto/cpu/cuda，缺省 auto；不允许由请求修改固定运行时版本 |

model/revision/variant_id/checksum 允许去除整体首尾空白后再校验，但不改写内部文件名、仓库大小写或修订。服务端重新发现 source/model/revision 的文件清单，并逐项核对 variant_id、SHA-256、文件类型和大小；工件 URL 由服务端构造。提交任意 artifact_uri、url 或本地路径不属于新接口契约。

仓库分支或标签不是不可变工件证明：服务端固定本次确认的身份和期望摘要；若仓库元数据或下载内容改变，必须明确失败，不选择其他量化、其他修订或其他来源来掩盖不一致。

- 出参列表：HTTP 202，success:true、data:DeploymentTaskData、reused:boolean。
- 响应头：Location 指向该任务的相对查询路径；未终态时 Retry-After 为 1 秒，客户端可据此查询，不再等待长同步部署响应。
- 错误码：400 invalid_deployment_request；413 request_too_large；503 deployment_service_unavailable；500 deployment_storage_failed。已登记任务的后续预检/部署错误见第八节，不倒改创建请求的 HTTP 响应。

请求示例，摘要仅演示格式，不能作为真实模型凭据：

```json
{
  "model": "owner/model-GGUF",
  "source": "modelscope",
  "revision": "master",
  "variant_id": "model-Q4_K_M.gguf",
  "checksum": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "backend": "auto"
}
```

## 六、查询及取消自动部署任务

========================================

- 接口名称：查询自动部署任务。
- URL：`/api/node/models/deployments/{task_id}`。
- 方法：GET。
- 说明：查询持久化父任务及子任务状态；仅补充属于该任务代次的当前路由视图，不改变任务状态或触发部署。
- 幂等规则：天然幂等。页面刷新后使用保存的 task_id 继续查询，不通过重新 POST 恢复页面。
- 入参列表：task_id，path，string，必填；必须是有效 UUID，且对应当前本机所有者的记录。
- 出参列表：HTTP 200，success:true、data:DeploymentTaskData；未终态时可返回 Retry-After:1。
- 错误码：400 invalid_task_id；404 deployment_task_not_found；500 deployment_storage_failed。

========================================

- 接口名称：取消尚未完成的自动部署任务。
- URL：`/api/node/models/deployments/{task_id}`。
- 方法：DELETE。
- 说明：无请求体。对 queued/running 原子登记 cancel_requested；停止后续推进，解除共享依赖，并清理本任务自有进程/路由。取消与成功提交串行判定：先成功提交则本次取消失败；先接受取消则迟到结果不能提交成功。
- 幂等规则：取消意图已接受但未终态时重复调用仍返回 202；已经 cancelled 时返回 200 和原取消快照。succeeded/failed/interrupted 返回 409，不改写原终态。
- 入参列表：task_id，path，string，必填；格式及本机所有权要求同 GET。
- 出参列表：清理尚未完成返回 HTTP 202、success:true、data:DeploymentTaskData，且 cancel_requested:true；清理已完成且 cancelled 返回 HTTP 200 和同一任务快照。
- 错误码：400 invalid_task_id；404 deployment_task_not_found；409 deployment_not_cancellable；500 deployment_storage_failed。

清理发生失败时最终为 failed，保留 cleanup_error；DELETE 的 202 不代表进程已经退出。仅有当前父任务不再需要下载时，才能停止其独占传输；还有其他使用者则不得取消共享作业。取消不删除已校验模型或已启用运行时。

已部署成功模型的停止继续使用 `POST /console/api/node/deployments/{model_id}/stop`，model_id 来自 deployment 结果。该操作不是取消历史部署任务，不能把 succeeded 改成 cancelled。

## 七、既有目录解析接口的向后兼容补充

========================================

- 接口名称：解析目录模型与所选后端部署预算。
- URL：`/api/node/models/resolve`。
- 方法：POST。
- 说明：继续由现有仓库发现与服务端资源配置合并生成候选。增加后端感知的筛选和预算，供用户在创建任务前看到 CUDA 下载增量、磁盘需求和驱动要求；此调用不下载、不解压、不安装依赖或启动模型。
- 幂等规则：只读，天然幂等；结果会随仓库、硬件和缓存变化，不能作为永久授权或磁盘预留。
- 入参列表：原有 model/source/revision 的字段及校验规则保留；新增可选 backend:string，枚举和默认值同第 3.3 节。
- 出参列表：保留 success、data.status、model、hardware、candidates、reason 等既有字段及类型；每个 candidates 项增加 deployment_estimate:DeploymentEstimate。
- 兼容规则：原候选的 min_ram_bytes、min_vram_bytes、disk_size_bytes、runtime、artifact_uri、checksum 不得为放行而清空或造假。按 backend 分别评估 CPU RAM、GPU VRAM 和工件驻留需求；新部署 POST 仍须重新核对权威身份，不能把这里的 artifact_uri 当作客户端可信证明。
- 后端不可用规则：已知但当前平台不支持的后端按既有 unsupported 结构返回 reason.code:unsupported_backend；实际依赖/设备是否可启动仍以部署任务预检和探测为准。未识别的 backend 或非法请求返回 400 invalid_model，保留原接口错误信封。
- 错误码：保留现有 invalid_model、model_repository_not_found、model_repository_unavailable 等错误响应和原有状态码，不借本次改造统一重写旧接口。

旧客户端可忽略新增候选字段，但缺省后端按本期 auto 语义筛选，候选集合可能与旧的 GPU 门槛筛选不同；新客户端在用户切换后端后重新解析，展示相应预算和要求，再由用户主动提交部署任务。不会把 CPU 选择静默提升为 CUDA，也不会为获取候选而清空 GPU 资源要求。

## 八、错误码与触发约束

### 8.1 请求级错误

| HTTP 状态码 | code | 场景 |
| --- | --- | --- |
| 400 | invalid_deployment_request | 新 POST 的 JSON、字段、凭据格式、未知字段或后端枚举无效 |
| 400 | invalid_task_id | GET/DELETE 的任务 ID 不是有效 UUID |
| 404 | deployment_task_not_found | 当前本机所有者下没有该任务 |
| 409 | deployment_not_cancellable | 任务终态不是 cancelled，不能把停止或历史失败伪装成取消 |
| 413 | request_too_large | 新增 POST 请求体超过 16 KiB |
| 500 | deployment_storage_failed | 任务登记、读取或取消意图无法可靠持久化 |
| 503 | deployment_service_unavailable | 服务尚不能可靠接受后台任务；不得返回虚假的 202 |

### 8.2 已登记任务的阶段错误

以下 code 进入 TaskError；GET 仍为 HTTP 200。字段语义与流程图的失败、取消和中断分支对应，不新增父任务状态。

| code | 阶段/场景 | 可观察结果 |
| --- | --- | --- |
| model_repository_not_found | preflight；所选仓库/修订不存在 | 失败，不下载替代模型 |
| model_repository_unavailable | preflight；支持的源不可访问或返回无效元数据 | 失败，用户可检查网络后重试 |
| artifact_identity_changed | preflight；文件、类型或权威 SHA-256 与所选凭据不一致 | 失败，要求重新选择/解析，不自动换版本 |
| unsupported_platform | preflight；不在本期四目标内 | 失败，不转向源码编译 |
| unsupported_backend | preflight；例如 Linux 请求 CUDA | 失败，不静默改后端 |
| insufficient_memory | preflight；所选后端 RAM/VRAM 条件不满足 | 失败，说明资源约束，不篡改清单 |
| insufficient_disk_space | preflight；合并后的最终/暂存空间不足 | 失败，在传输前结束 |
| disk_space_unavailable | preflight；无法可靠完成目标文件系统空间检查 | 失败，不把未知当作足够 |
| bundled_runtime_missing | preparing_runtime；开发构建没有内嵌资源且无可用显式配置/缓存 | 明确缺失；官方缺资源构建应提前失败 |
| runtime_configuration_invalid | preparing_runtime；显式程序路径、版本约束或程序身份无效 | 失败，不覆盖用户配置或查找其他程序替换 |
| runtime_download_failed | preparing_runtime；固定 CUDA 包传输失败 | 失败，保留可恢复断点，不访问 GitHub API |
| runtime_checksum_mismatch | preparing_runtime；任一运行时/依赖包摘要不符 | 失败，不启用可疑安装 |
| runtime_archive_invalid | preparing_runtime；归档路径、链接、条目或解压资源限制不满足 | 失败，安全清理本次暂存，不覆盖有效缓存 |
| vc_runtime_missing | preparing_runtime；Windows 缺少必需 VC++ 依赖 | 提示安装发行物随附、具有授权的 x64 运行库并重试；不自动提权 |
| cuda_unavailable | preparing_runtime；设备/驱动或 cudart 实际探测失败 | 失败，由用户重新选择 CPU，不自动改选 |
| runtime_probe_failed | preparing_runtime；版本/依赖/执行权限/原生后端探测失败 | 失败，保留可操作原因，不绕过系统安全检查 |
| model_download_failed | downloading_model；模型传输失败 | 失败，不伪造已校验工件 |
| checksum_mismatch | verifying；实际文件摘要与固定期望不符 | 失败，不启动；错误摘要不得登记为可复用可信文件 |
| model_incompatible | starting/health_checking；固定运行时不能加载所选 GGUF | 明确失败，不擅自更换模型或量化 |
| process_start_failed | starting；子进程未能可靠创建 | 失败并清理本任务资源 |
| health_check_failed | health_checking；退出、健康失败或既有探测超时 | 失败并清理本任务资源 |
| route_attachment_failed | attaching_route；路由接入失败、代次不匹配或未达到可路由条件 | 失败，不能显示可以对话 |
| deployment_storage_failed | 任一阶段；进度/最终状态无法可靠持久化 | 不报告成功，必要时清理新进程和路由 |
| cleanup_failed | 失败/取消清理；自有进程或路由不能确认已撤销 | failed；原始错误保留，cleanup_error 给出独立原因 |
| service_restarted | 启动恢复发现本机 queued/running 任务 | interrupted，用户主动重试，不自动重复启动 |

用户请求正常取消且清理成功时 status:cancelled、error:null、cleanup_error:null。发生原始阶段错误且随后清理也失败时，error 保留原始阶段原因，cleanup_error.code 为 cleanup_failed；仅取消清理失败时 error 和 cleanup_error 均明确指向清理失败，不能报告正常取消。

## 九、接口验收与公开范围

- 三个新接口覆盖有效请求、无效 JSON/凭据、未知字段、任务未找到、重复提交、取消幂等、终态操作和存储失败。
- 覆盖模型/运行时分支并行与进度、共享作业取消、迟到结果隔离、实际摘要不匹配、设备探测/健康/路由失败，以及服务重启后的 interrupted。
- succeeded 但进程后来退出、停止、路由繁忙或当前视图丢失时，ready_to_chat 不得保持错误的 true；历史终态也不被改写。
- resolve 对旧请求保持可解析，对新增 backend 正确筛选；CUDA 未主动选择时运行时预计在线下载量为 0，不能触发加速包下载。
- 后续编码同步更新 `api/openapi.rs`、服务端 README 和根 README；既有手动接口与下载接口保持兼容测试。
- 本契约新增的是 HTTP 边界，DTO、处理器及 Rust 内部函数默认私有；不为测试扩大跨 crate 公共 API。现有 DownloadManager、RuntimePreparer/PreparedRuntime 的具体复用和必要内部可见性在接口定稿后的开发文档中列明。
- 用户已于 2026-11-24 确认 URL/方法、字段类型、状态/阶段、错误码、取消与幂等规则及 resolve 的兼容补充；该确认不等于开发文档或接口实现已完成。

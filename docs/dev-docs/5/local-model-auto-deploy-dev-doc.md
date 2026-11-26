# BurnCloud 本机模型自动部署开发文档

> 开发序号：5；Issue：songyi77101/burncloud-1112#13；分支：codex/local-model-auto-deploy。
> 日期：2026-11-24。设计起点：用户已确认基于现有实现继续开发。
> 上游：同目录已确认的 `local-model-auto-deploy-flowchart.md` 与 `local-model-auto-deploy-interface-contract.md`。
> 状态：用户于 2026-11-24 确认定稿。本文定义后续实现，不表示功能、发行物或实机测试已经完成；后续按既定流程拆分任务并编码。

## 一、实现范围与复用决策

### 1.1 不变的需求与 HTTP 边界

- 分别发行 Windows x64、macOS ARM64、macOS x64、Linux x64；每份程序只内嵌本目标的一份 b8581 基础压缩包。CUDA 不默认内嵌，模型不随客户端打包。
- Windows/Linux 的 auto 使用 CPU；macOS 的 auto 优先实际探测 Metal，否则 CPU。只有 Windows 用户显式选择 cuda，才准备 CUDA 12.4 服务包和 cudart 两个固定包。CUDA 失败不自动降级。
- 不做 Linux CUDA/ARM64、musl、驱动/Toolkit 安装、源码编译、远程节点、非 GGUF 转换或新安装器。Linux 使用原有服务端/Web 界面。
- 只新增 POST `/api/node/models/deployments`、GET/DELETE `/api/node/models/deployments/{task_id}`；扩展已有 POST `/api/node/models/resolve` 的 backend 和 deployment_estimate。具体请求/响应、严格 JSON、错误码、HTTP 状态以接口契约为准，不另建第二套接口。
- 旧下载接口、手动本地路径部署、查询和停止接口继续保留；自动任务取消不是已完成模型的停止操作。沿用现有 Node 路由信任策略，不新建账户权限体系。
- 新接口不接受 URL、本地路径、可执行程序或运行时版本。选择凭据由服务端重新向所选源核对，不将前端传回的 SHA 或旧数据库记录当作实际校验结果。

### 1.2 源码核对与改造位置

| 位置 | 实际现状 | 本次设计 |
| --- | --- | --- |
| download/src/manager.rs、operations.rs、monitor.rs、utils.rs | with_store 调用 Windows aria2 引导；gid 与稳定数据库 ID 并存 | reqwest 流式传输替代引导/轮询；保留公有签名与稳定 ID；gid 成为不透明传输代次 |
| database/src/migration/mod.rs | 迁移登记到 0019 | 两种数据库新增并登记 0020_local_model_deployments |
| database-sys/src/download.rs | 已有下载身份、状态、实际 SHA 与稳定 ID | 复用原门面；只同步中文文档中的传输语义，不新增部署仓储公有 API |
| server/src/api/model_resolution.rs | 已有真实文件清单发现及内置资源要求合并 | 保留发现逻辑，增加可复用的服务端凭据核对与按后端预检 |
| server/src/api/model_download.rs | 已有下载、真实 SHA 校验和取消接口 | 保持原 HTTP 兼容；自动链路复用 DownloadManager，不能 HTTP 自调用 |
| server/src/node_runtime.rs | 只取 BURNCLOUD_LLAMA_SERVER 并检查文件存在 | 增加实际版本/依赖/设备探测和受管运行时；显式配置优先，不覆盖 |
| server/src/node_deployment.rs | start_local 按 file_stem 命名并直接构造 Verified；路由接入错误可能仅记日志 | 手动语义保留；新增只接受真实已校验工件的入口，自动链路额外检查健康和同代次路由 |
| node/src/deployment/service.rs | deploy_prepared 可能返回 Unhealthy；stop_model 只按模型 ID 停止 | 不把 Ok 直接映射 succeeded；补充精确代次停止，避免清理新代次 |
| client/src/domains/chat/ | 目录模型先下载再调用手动路径部署；HttpBackend.generate 仍委派本地替身 | 目录模型改任务提交/查询/取消；用服务端 model_id 调既有 BurnCloud 数据面，不生成模拟回复 |
| .github/workflows/cd-release.yml | 当前发行主要是 Windows 单 exe ZIP | 四目标矩阵、内嵌资源检查、许可证/签名/体积报告门禁 |

所有路径以仓库根为基准。新增生产实现限定在已登记工作区；不改用户现有 llama.cpp、target 下运行测试资源和用户未跟踪的 server/docs 文件。不扩建通用下载框架、插件体系或更新安装器。

### 1.3 模块组织与依赖

- `download/src/http.rs` 承载 HTTP 传输与续传；manager/operations 保留门面；原 aria2 crate 不删除、不自动启动。为兼容保留 DownloadError::Aria2 及其类型依赖，公有枚举不擅自删变体。
- `server/src/model_deployment.rs` 是任务门面与编排；`model_deployment_store.rs` 处理本模块 SQL；`model_artifacts.rs` 处理权威身份、预算与模型共享准备；不把服务端业务移入通用数据库 crate。
- `server/src/managed_runtime.rs` 处理受管作业、探测、缓存；`runtime_archive.rs` 只处理归档输入边界；`api/model_deployments.rs` 只处理 HTTP 适配。现有 node_runtime.rs 仍提供 RuntimePreparer/RuntimeAdapter。
- `client/.../chat/deployment.rs` 承载新任务 DTO、请求及轮询，`inference.rs` 承载既有数据面请求与 SSE 解析；catalog/backend/page/state/model 按职责连接，不继续把所有代码塞进 catalog.rs。
- 只在跨文件调用处使用本文明确列出的 `pub(super)` / `pub(crate)`；Rust 对外 `pub` 仅有既有 API 和第六节明确登记的精确代次停止方法。测试不作为公开理由。
- 复用已经注入的 Database、DownloadManager、RuntimePreparer、RuntimeAdapter、DeploymentService、LocalRouteAttacher；不在服务层另开数据库或全局进程管理器。依赖只沿原 workspace 方向，不引用其他 crate 私有实现。
- 新依赖限于实际使用的 tar、flate2、fs2（安全文件锁/空间查询）及现有 reqwest/sha2/tokio 能力；具体解析版本写入 Cargo.lock，不在文档声称未知版本已验证。

## 二、状态、并发与失败语义

### 2.1 唯一状态集合

```text
TaskStatus = queued | running | succeeded | failed | cancelled | interrupted
DeploymentStage = preflight | preparing_runtime | downloading_model | verifying | starting | health_checking | attaching_route
RequestedBackend = auto | cpu | cuda
EffectiveBackend = cpu | metal | cuda
```

父任务和两个准备分支都使用契约 TaskStatus，不增加 cancelling/ready/stopped 父状态。阶段与子阶段约束完全复用契约第 3、4 节。运行时尚未完成时父阶段为 preparing_runtime，之后显示模型分支 downloading_model/verifying；两分支完成后才进入 starting。

- 对外 stage 不直接序列化 Node 内部 DeploymentStage：Preparing/Queued 等内部进度留在 detail；Starting 映射 starting；WaitingReady/HealthChecking 映射 health_checking；InitialResult(Ready) 后核对路由时映射 attaching_route。
- 模型 actual_sha256 统一为实际计算出的 64 位小写十六进制；请求 checksum 保留 sha256: 前缀，两者比较先规范化摘要字节，不能用前缀差异误判或直接复制期望。实际字节只来自文件/传输；本地解压、探测、健康、路由不造百分比。运行时纯离线准备为 0/0；CUDA 两包合计；runtime.actual_sha256 固定 null。
- 终态保存历史结果，不被迟到子进度、心跳、取消或查询改写。子项终态按契约收束，不能留 queued/running；原始 error 与 cleanup_error 分列。

### 2.2 三种键，不混用 ID

1. **请求去重键 request_key**：对规范化后的 source/model/revision/variant_id/checksum/requested_backend 按固定顺序 JSON 编码再 SHA-256；不以分隔符拼接，避免歧义。与 owner_id 一起唯一占用 queued/running，在取消清理结束前也不释放。
2. **模型工件键 artifact_key**：同上但不含 backend，保存完整身份；新文件写入 `models/<artifact_key>/`。不同源或摘要不混用。GGUF 文件名仅作展示，不作信任凭据。
3. **部署键 deployment_key**：owner_id + artifact_key + effective_backend 的完整摘要；model_id 为 `bc` 加该摘要前 32 位十六进制，恰好 34 个字符，满足现有本地挂载名称上限。数据库保留完整键，发现短 ID 对应其他完整键时失败，不覆盖。model_id 不包含 task_id，不能借重试绕过同模型/后端串行约束。

auto/cpu 等不同请求键最终得到同一部署键时，共用该键的启动互斥锁。锁跨启动、健康、接路由、提交/取消清理一直持有。若已有同身份、同后端、相同实测运行时指纹、同代次且当前健康/routable 的实例，任务借用它而非再启进程；取消借用任务不停止已有实例。已有实例不健康、不可路由或运行时指纹不同则报告当前阶段失败，要求使用现有停止入口处理，不擅自覆盖其他成功任务的进程。

### 2.3 持久化与取消线性化

- POST 只在本机所有者就绪、后台队列可接收且 queued 已持久化后返回 202。后台预检耗时不阻塞响应。插入用部分唯一索引处理竞争，冲突后读取同 owner 在途记录，不用先查后插伪幂等。
- 所有状态更新采用 `WHERE owner_id=? AND task_id=? AND row_version=? AND status IN ('queued','running')` 的 CAS；取消与最终 succeeded 的事务在同一行上串行。成功更新额外要求 cancel_requested=0。CAS 冲突重读，不丢失已接受取消。
- HTTP DELETE 先提交取消意图，再唤醒父任务。不能先取消内存令牌后写库。原终态 succeeded/failed/interrupted 返回 409；cancelled 返回原快照 200；正在清理仍为 202。
- 并发上限为 32 个在途父任务和 4 个同时执行的准备作业；已存在的同键请求优先复用，满额时新请求以 503 拒绝且不留无 worker 的 queued。每个父任务持有自己的订阅取消令牌；共享准备作业持有独立令牌，父取消只解除订阅。模型和运行时已开始的共享作业即使最后一个订阅者退出，也完成有超时边界的一次缓存准备；不会启动模型。其服务级所有者负责最终清理暂存，并在无网络进展超时或服务关闭时结算。这样不会误取消仍被旧下载 HTTP 接口或其他父任务使用的下载。
- 启动调用不采用丢弃 future/abort JoinHandle 的方式取消。已经进入 deploy_prepared 的调用由拥有者等待其结算，再精确停止本次新建代次并撤销其路由。取消响应可立即 202，但 cancelled 必须等清理证据齐全；现有 readiness 上限 600 秒，期间界面说明“等待启动结算并清理”，不声称进程已停。排队但还未调用部署服务的任务直接退出，不创建进程。
- 失败后不删除已校验模型、共享归档和其他任务使用的运行时。仅删除已核实在自有 staging 根内的暂存；错误路径、链接或跨根删除请求必须拒绝。
- 正常取消清理成功时 error/cleanup_error 都为空；仅取消清理失败时两者指向 cleanup_failed，有原始阶段错误则保留原错误并单列 cleanup_error。进度每秒最多持久化一次，阶段/取消/终态立即写库。任何必要写库失败都禁止声称成功，清理新建资源；数据库仍不可用时 GET 返回存储错误而非编造终态，恢复后仍按中断处理。

### 2.4 启动恢复与本机隔离

- owner_id 是用户数据根中原子创建并持久化的 UUID，不用主机名、IP 或数据库 URL 推断，不从请求读取。session_id 是本次服务实例 UUID。取得数据根单实例文件锁后，才做恢复和接受任务；锁失败服务不可接受自动部署。
- 文件锁从初始化持有到服务退出，防同一数据根的第二个进程把仍在运行的任务误标中断。共享 PostgreSQL 上其他数据根 owner 的记录不读取、不取消、不恢复。
- 恢复只对当前 owner 的 queued/running 标 interrupted，未完成子项相应中断。保留有效文件、摘要和断点；不重跑 deploy_prepared，不恢复旧 ready_to_chat。
- 可证明归属的旧 attachment 使用既有 LocalRouteAttacher 隔离/撤销；失败返回服务初始化错误并记受控日志，阻止接受新部署。已提交 interrupted 的记录只保留 service_restarted，不能再附加只适用于 failed 的 cleanup_error 或改写终态。未记录 attachment 或没有原进程句柄时不按 PID/进程名猜测终止；新会话没有同代次心跳即 fail-closed，向日志报告无法证明清理的残留。不能给旧数据库成功快照制造实时视图。
- owner 文件/受管根丢失或损坏时不得回退到“全表恢复”；初始化失败并给出修复提示。用户数据根不支持多机克隆共享运行实例。

## 三、下载、运行时与模型准备细则

### 3.1 用户数据与磁盘预算

用户数据根复用 `burncloud_database::get_default_database_path()` 的父目录，只把它用于目录约定，不跟随外部 PostgreSQL 的地址创建系统路径：Windows 为用户 Local/BurnCloud，macOS/Linux 为当前项目沿用的 ~/.burncloud。

```text
<用户数据根>/
  node-owner.json
  node-deployments.lock
  models/<artifact_key>/<固定文件名及传输断点>
  runtimes/downloads/<版本>/<固定包文件及传输断点>
  runtimes/<版本>/<平台>/<包类型>/<安装摘要>-<安装ID>/
  runtimes/.staging/<随机作业ID>/
```

- 下载器继续允许原 executable/models 根，新增且仅新增上述受管 models 和 runtimes/downloads 根。不能允许整个用户目录或任意绝对路径。旧相对路径语义保留；新服务只传自己生成的绝对路径。受管下载不得为了初始化旧根而强制创建不可写的 executable/models，先按目标根分支再创建。源目录、已存在父级及最终路径都规范化，拒绝越界链接/reparse point。
- 新模型、运行包下载及 staging 固定在同一个受管数据根。不得在其子根上跟随用户创建的挂载或目录链接跨到另一文件系统；Unix 比较设备号，Windows 拒绝子目录 reparse point。最终运行时内经过验证的内部相对链接例外，只能留在安装根内。
- 因新写入根固定在一个已确认文件系统，DiskCheck 合并为 storage:model_and_runtime；不按模型和运行时各自独立放行。同一文件系统的额外空间预算为模型尚需新增文件/断点字节、仍需落盘的归档、完整解压暂存、必要的新依赖副本及 64 MiB 安全余量之和；rename 不重复计整份文件，现有有效安装不重复计。
- 原始 disk_size_bytes 仍作为模型最低空间要求，不得因已有下载量为 0 而清空配置要求。解压精确上界来自固定受审归档清单；未知模型总量或空间查询失败时任务在传输前失败，不能当 0 放行。预算不是磁盘预留，运行中 ENOSPC 仍正常报告失败。
- 候选 resolve 可以返回未知空间 sufficient:null，但不能伪报 true；部署在后台重新做确定检查。预览不下载包、不解压、不启动进程。

### 3.2 HTTP 流式传输与断点

- 保留 DownloadManager 现有公有函数、数字 task_id、任务状态/数据库记录、取消、等待及上层摘要确认机制；不初始化 aria2，不请求其 RPC。gid 保留为 `http:<随机UUID>` 形状的不透明传输代次，重试可变，旧 GID 不作 HTTP 身份。
- 显式关闭自动重试 POST；文件 GET 使用单流、有限重试（沿现有 MAX_RETRIES=1），连接超时 15 秒、无字节进展 60 秒超时。长文件不设过短总下载时限；取消令牌参与每轮请求/读写。保持有界块缓冲，不将文件读入内存。
- 通用下载入口按原规则拒绝非 HTTP(S)、凭证 URL、私网/回环/链路本地等目标。每一跳最多五次手动重定向，重新检查 URL 和解析 IP，并将已校验地址固定给实际连接，防 DNS 重绑定；环境 HTTP 代理导致目标解析不能被固定验证时拒绝该路径，不宣称已防重绑定，固定 HTTPS 镜像/Release 代理 URL 不受此限制；HTTPS 不允许降级 HTTP，不向新主机转发敏感头。测试回环传输通过私有测试注入，不在正式构建开放 SSRF 开关。
- 自动模型入口先约束到所选仓库源构造的 HTTPS 地址，仍校验所有跳转。CUDA 入口只能来自固定清单代理路径，重定向若转向 github.com、api.github.com、githubusercontent.com、githubassets.com 等直连资源域名则失败，不借跳转偷偷改成用户直连 GitHub。
- 文件名优先来自已经核对的目标文件标识；DownloadManager 现有 URL 推导补充通用 basename，不能信任 Content-Disposition 写任意路径。旧记录继续使用原已校验文件名，不自动重命名用户旧模型。
- 私有原子侧车 `<文件>.part.json` 记录本机下载 task_id、URL、已解析实体身份、强 ETag、总大小、已同步连续长度。先写并 sync 部分文件，再原子更新侧车；断电后文件多于已确认长度时截断到该长度，少于时安全重下。侧车不替代最终 SHA。
- 恢复只在部分文件、当前 URL 和强 ETag 可重新核对时发送 Range 与 If-Range；请求 Accept-Encoding:identity，禁用透明内容解压。只有 206 的起点/终点/总量、ETag 和实际体长吻合才追加。200、实体变化、弱/缺少 ETag、非法 Content-Range 或不可靠 416 一律丢弃未验证断点安全重下，不拼接。
- 416 只有在本地连续长度等于经强实体核对的完整长度时才可进入 verifying；否则重下。200 从零写入，206 截短/超长失败，200 在 EOF 时检查 Content-Length（存在时）。最终 SHA 由上层实际读取全文件计算。
- aria2 旧部分文件可能为非连续分片/稀疏文件：没有本下载器有效侧车时不凭文件长度续传，保留旧记录稳定 ID，但重新开始 HTTP 文件；旧完整文件可重新算 SHA 后复用。旧 .aria2 元数据不作为新传输身份，不删除不属于本次任务的文件。
- 写完临时文件并完成传输状态持久化才进入 Verifying；未进行真实摘要确认不能返回 Succeeded。取消保留可验证的部分文件；恢复重新核对实体，不信任旧下载速度或旧 GID。

### 3.3 模型身份与后端资源检查

- 服务端复用 discover_manifest_at 的 ModelScope/HF 清单和内置资源合并，以完整 source/model/revision/variant_id 定位 GGUF，重新比对 SHA-256、类型及大小。缺权威 SHA 不在本接口承诺可信；身份改变为 artifact_identity_changed，不替换量化/修订/来源。
- 分支修订不是不可变对象：能取到真实提交修订时记录 resolved_revision 用于生成下载地址，否则保留请求修订并以固定 SHA 和下载后全量校验防漂移。resolved_revision 是内部证据，不改写对外 revision。
- 原 min_ram_bytes、min_vram_bytes、disk_size_bytes 不改写。cpu 检查 RAM，不要求 GPU 显存；cuda 检查 RAM 和实际所选 NVIDIA 设备显存；macOS auto 先保证 CPU 可部署的 RAM 基线，Metal 探测且统一内存条件满足才选 Metal，否则保留 CPU 并给出原因。不能将共享内存加总冒充独立 VRAM。
- 预检阶段进行硬件/空间预算，运行时阶段实际探测程序与设备；effective_backend 只在实际选择后写入。显式 cpu 始终使用零 GPU layers，auto 不是 Windows 的隐式 CUDA 授权。
- 缓存命中也重新检查真实文件及全量 SHA；有损坏即隔离为不可信，不改动仍被运行实例使用的旧文件。下载至新暂存并校验后原子启用，Verified 工件只在此时构造。文件句柄/路径身份在启动前再核对，不能仅凭旧日志中的 verified 字样放行。

### 3.4 受管运行时与安全解压

1. 显式 BURNCLOUD_LLAMA_SERVER 存在时只验证并探测该程序，不回退搜索 PATH，不替换、不下载覆盖。版本约束以请求和实测为准；显式版本不是自动谎报 b8581。失败明确 runtime_configuration_invalid/runtime_probe_failed。
2. 否则按 b8581+目标+包类型加入服务级共享准备作业；先验证受管缓存，再从本平台内嵌资源准备；macOS CPU/Metal 共享同一原生包安装，设备选择仍逐次探测。开发构建无资源、无有效缓存时 bundled_runtime_missing，不临时访问 GitHub 或编译。
3. Windows cuda 固定准备两个包，均通过各自 SHA 后才解压合并；同路径同内容可复用，同路径不同内容必须在受审组合清单中明确，否则拒绝。不下载 Toolkit 或驱动。
4. 归档在随机 staging 中处理；受信包先核对压缩字节 SHA，再枚举条目。拒绝绝对/盘符/UNC 路径、..、反斜线混淆、NUL、Windows ADS/保留设备名、大小写碰撞、重复冲突条目、设备/FIFO、越界/循环链接及写穿已有链接。路径上限 4096 字节、最多 100000 条目、单文件上限 2 GiB、总展开上限 4 GiB，同时不得超过受审包清单上界。
5. 先创建目录和普通文件，所有链接延后统一解析到安装根内的有效目标；hardlink 只指向已提取普通文件，symlink 只能使用相对路径，链必须无环。后续文件不能写入链接目录。Windows 的首期 ZIP 如出现未受审链接则拒绝；macOS/Linux 保留受审内部链接及必要执行权限，去除 suid/sgid 和多余写权限。不调用 shell 解压或要求用户安装工具。
6. 受管资源完整保留 DLL/dylib/so 和许可证；mac/Linux 归档顶层 llama-b8581 按清单定位，不靠全盘找第一个同名程序。完成版本、动态库、依赖、设备探测后写 ready.json，再将同文件系统 staging 原子 rename 成内容寻址安装。已有有效目录不覆盖；损坏或使用中的目录不原地修补。物理目录名加安装 ID，允许保留被使用的坏副本并原子启用新副本；逻辑缓存键仍为版本/平台/包类型，不增设数据库全局指针。
7. 缓存 ready.json 不是信任根。复用时根据内置固定包 SHA 验证保留归档，并以归档内容或受审只读库存核对安装文件、内部链接和权限；仅有一个可改 marker 不足以视为缓存有效。失败在新 staging 重建，不修改已有运行实例目录。
8. 探测子进程只执行固定参数 `--version`、`--list-devices`，每次 15 秒、捕获输出上限 1 MiB；超时由自有句柄终止并等待退出，失败返回错误。CPU 不要求 GPU 存在；Metal/CUDA 必须确认设备被实际枚举并将设备标识交给适配器，启动后仍须健康验证。
9. 启动参数由适配器生成：CPU 使用 `--n-gpu-layers 0`；选定 Metal/CUDA 使用固定 GPU layers 设置和实际设备选择参数，不能仅依赖自动默认值。显式启用 `/slots` 所需固定选项（b8581 以实际 `--help` 核对 `--slots`），同时核对 `/v1/models` 精确别名与容量；不能健康通过却因缺容量端点永久不可路由。参数支持性以固定版本 `--help` 的受审验收为门禁；不使用 shell 拼接用户内容。目标始终 loopback，别名为服务端 model_id。
10. Windows 缺 VC++ 时先将受审内嵌安装程序解出到用户根 dependencies/vc_redist.x64.exe（已存在则核对摘要/签名，不覆盖未知文件），界面给出固定相对位置与用户确认操作，不为此新增 HTTP 运行命令端点。随后提示安装本发行内嵌的、授权与签名/摘要均经过审核的 x64 安装程序；写到用户根内，由用户确认运行和系统授权，不自动提权。macOS 不关闭 Gatekeeper、不删除隔离属性，签名/系统拦截如实报告。

## 四、数据结构

以下为设计伪代码，不是已新增 Rust 公共类型。标量字段标“独占拥有”表示值归该对象；借用字段不负责释放；Arc/只读资源只释放自己的引用。未标公开的类型和字段均保持私有。HTTP DTO 的 Rust 字段保持私有，serde 序列化不要求 pub。

========================================

- 名称：DeploymentCommand
- 类型：私有严格请求结构
- 说明：由新 POST 边界校验生成，后台不得重新接受客户端 URL。
- 结构：

```python
class DeploymentCommand:
    model: str  # 独占拥有；规范化的仓库身份
    source: ModelSource  # 独占拥有；严格来源枚举
    revision: str  # 独占拥有；用户修订，不静默替换
    variant_id: str  # 独占拥有；安全 GGUF 相对文件标识
    checksum: Sha256  # 独占拥有；期望摘要而非实际摘要
    backend: RequestedBackend  # 独占拥有；缺省 auto
```

========================================

- 名称：DeploymentTask
- 类型：模块内结构（跨文件类型 pub(crate)，字段 pub(super) 仅按需要开放）
- 说明：对外字段与契约 DeploymentTaskData 完全一致；内部加持久化和所有权证据，不序列化路径/进程句柄。
- 结构：

```python
class DeploymentTask:
    task_id: Uuid  # 独占拥有；部署 UUID，不是下载数字 ID
    owner_id: Uuid  # 独占拥有；本机用户数据根身份
    session_id: Uuid  # 独占拥有；创建任务的服务代次
    request_key: Sha256  # 独占拥有；完整请求去重键
    command: DeploymentCommand  # 独占拥有；规范化身份
    status: TaskStatus  # 独占拥有；契约六态
    stage: DeploymentStage  # 独占拥有；契约七阶段
    effective_backend: Optional[EffectiveBackend]  # 独占拥有；实测后填写
    backend_reason: Optional[str]  # 独占拥有；中文后端说明
    cancel_requested: bool  # 独占拥有；已持久化的取消意图
    model_download: PreparationProgress  # 独占拥有；本父任务的模型子快照
    runtime: RuntimeProgress  # 独占拥有；本父任务的运行时子快照
    estimate: Optional[DeploymentEstimate]  # 独占拥有；最近确定的预算
    deployment: Optional[DeploymentOwnership]  # 独占拥有；归属凭据，不是实时健康证明
    error: Optional[TaskError]  # 独占拥有；原始错误
    cleanup_error: Optional[TaskError]  # 独占拥有；独立清理错误
    created_at: Timestamp  # 独占拥有；RFC3339
    updated_at: Timestamp  # 独占拥有；最近持久化时间
    finished_at: Optional[Timestamp]  # 独占拥有；终态时间
    row_version: i64  # 独占拥有；CAS 代次
```

========================================

- 名称：PreparationProgress / RuntimeProgress
- 类型：私有 DTO；确有跨文件需要的类型 pub(crate)
- 说明：对应接口契约 4.2；运行时扩展而非另一套子状态。
- 结构：

```python
class PreparationProgress:
    status: TaskStatus  # 独占拥有；该父任务视角
    stage: Optional[DeploymentStage]  # 独占拥有；模型仅下载/校验，运行时仅准备
    detail: Optional[str]  # 独占拥有；可展示说明，不是新状态
    task_id: Optional[str]  # 独占拥有；模型下载数字 ID 转字符串；运行时恒 null
    completed_bytes: u64  # 独占拥有；真实文件/传输字节
    total_bytes: Optional[u64]  # 独占拥有；未知为 null
    download_speed: u64  # 独占拥有；非传输阶段为 0
    actual_sha256: Optional[str]  # 独占拥有；实际模型 SHA；运行时恒 null
    error: Optional[TaskError]  # 独占拥有；本分支错误

class RuntimeProgress(PreparationProgress):
    origin: Optional[str]  # 独占拥有；explicit/cache/bundled/downloaded
    version: Optional[str]  # 独占拥有；程序实测版本
```

========================================

- 名称：DeploymentEstimate / RuntimePackage / DiskCheck
- 类型：私有 DTO；需要跨文件的字段只在模块内开放
- 说明：字段精确遵循接口契约 4.3，不把未知值变成 0。
- 结构：

```python
class DeploymentEstimate:
    checked_at: Timestamp  # 独占拥有；检查时间
    model_download_bytes: Optional[u64]  # 独占拥有；预计增量，不是文件总量
    runtime_download_bytes: Optional[u64]  # 独占拥有；基础离线为 0
    runtime_packages: list[RuntimePackage]  # 独占拥有；固定所需包
    disk_checks: list[DiskCheck]  # 独占拥有；按文件系统合并
    requirements: list[str]  # 独占拥有；依赖及驱动的可信说明

class RuntimePackage:
    name: str  # 独占拥有；固定包名
    size_bytes: u64  # 独占拥有；固定压缩大小
    checksum: str  # 独占拥有；sha256: 前缀摘要
    origin: str  # 独占拥有；bundled/download
    download_required: bool  # 独占拥有；实际是否需要网络下载

class DiskCheck:
    storage: str  # 独占拥有；本设计新写入统一为 model_and_runtime
    required_bytes: u64  # 独占拥有；合并额外空间
    available_bytes: Optional[u64]  # 独占拥有；未知为空
    sufficient: Optional[bool]  # 独占拥有；未知不作通过
```

========================================

- 名称：ConfirmedArtifact
- 类型：模块内严格工件证据
- 说明：只由服务端清单核对产生，未验证字节时不能转换为 PreparedArtifact。
- 结构：

```python
class ConfirmedArtifact:
    command: DeploymentCommand  # 独占拥有；本次确认身份
    resolved_revision: Optional[str]  # 独占拥有；可得时固定远端提交修订
    artifact_key: Sha256  # 独占拥有；模型缓存键
    url: Url  # 独占拥有；服务端构造并经信任边界校验
    size_bytes: u64  # 独占拥有；权威文件大小
    min_ram_bytes: u64  # 独占拥有；原配置，不清零
    min_vram_bytes: u64  # 独占拥有；原配置，不直接套用于 CPU
    disk_size_bytes: u64  # 独占拥有；原配置
    target: PathBuf  # 独占拥有；受管根内目标，不对外返回
```

========================================

- 名称：DeploymentOwnership
- 类型：私有运行归属记录
- 说明：进程归 DeploymentService 独占拥有；该结构仅持归属凭据，不持裸 PID 所有权。
- 结构：

```python
class DeploymentOwnership:
    deployment_key: Sha256  # 独占拥有；完整实际模型/后端键
    model_id: str  # 独占拥有；34 字符路由 ID
    generation: Optional[u64]  # 独占拥有；实际事件代次
    route_attachment_id: Optional[i32]  # 独占拥有；具体路由 ID
    created_by_task: bool  # 独占拥有；false 表示只借用现有实例，不能清理他人资源
    originating_session: Uuid  # 独占拥有；归属会话，避免重启冒认
    runtime_fingerprint: Sha256  # 独占拥有；实际安装库存/显式程序摘要和版本证据
```

========================================

- 名称：DeploymentTasks / TaskWorker
- 类型：服务级所有者与任务级拥有者
- 说明：生命周期显式管理，不形成 Arc 引用环，不在 Drop 中假装完成异步清理。
- 结构：

```python
class DeploymentTasks:
    store: DeploymentStore  # 独占拥有；共享数据库引用的本模块仓储
    node: Arc[NodeDeploymentState]  # 共享；既有进程/路由门面
    artifacts: Arc[ModelPreparations]  # 共享；模型准备作业所有者
    runtimes: Arc[ManagedRuntimes]  # 共享；运行时准备作业所有者
    owner_id: Uuid  # 独占拥有；本机身份
    session_id: Uuid  # 独占拥有；本次会话
    owner_lock: Arc[File]  # 共享；与任务上下文共持，最后清理结束才释放单实例锁
    workers: Mutex[Map[Uuid, TaskWorker]]  # 独占拥有；后台任务及取消通知
    deployment_locks: Mutex[Map[Sha256, Weak[Mutex]]]  # 独占拥有；按实际键串行，弱引用避免无限增长
    shutdown: CancellationToken  # 独占拥有；服务关闭信号

class TaskWorker:
    task_id: Uuid  # 独占拥有；任务归属
    cancelled: CancellationToken  # 独占拥有；仅取消本订阅者
    join: JoinHandle  # 独占拥有；正常 shutdown 必须等待，不随请求断开丢弃
```

========================================

- 名称：SharedPreparation / PreparedRuntimeInfo
- 类型：服务模块内共享作业及不可变结果
- 说明：模型、运行时共享作业使用同样的所有权规则，但不新建通用插件框架。
- 结构：

```python
class SharedPreparation:
    key: Sha256  # 独占拥有；模型身份键或运行时包键
    progress: WatchSender  # 独占拥有；最新不可变进度快照
    completion: SharedResult  # 共享；完成结果及错误，不是父任务终态
    join: JoinHandle  # 独占拥有；服务负责等待和暂存清理
    staging: Optional[TempDir]  # 独占拥有；受管 staging，成功 rename 后解除临时所有权

class PreparedRuntimeInfo:
    prepared: PreparedRuntime  # 独占拥有；保留现有 executable 公有形状
    effective_backend: EffectiveBackend  # 独占拥有；实际选择
    device: Optional[str]  # 独占拥有；实际设备标识
    origin: str  # 独占拥有；本次来源
    version: str  # 独占拥有；实测版本
    install_root: Optional[PathBuf]  # 独占拥有；受管目录；显式配置不拥有用户目录
```

========================================

- 名称：RuntimeAsset / ResumeMetadata
- 类型：只读发行清单项 / 私有传输侧车
- 说明：前者编入构建，后者仅是安全断点证据，均不能由部署请求覆盖。
- 结构：

```python
class RuntimeAsset:
    version: str  # 共享；固定 b8581
    target: str  # 共享；精确 Rust target
    package_kind: str  # 共享；cpu/native/cuda12.4
    filename: str  # 共享；固定归档名
    size_bytes: u64  # 独占拥有；压缩字节数
    sha256: Sha256  # 独占拥有；实际内嵌/下载包摘要，签名派生时不是源摘要
    source_sha256: Sha256  # 独占拥有；7.1 固定上游源包摘要，供溯源
    unpacked_bytes: u64  # 独占拥有；受审归档展开上界
    server_relative_path: str  # 共享；清单定位而非搜索系统 PATH
    download_url: Optional[str]  # 共享；CUDA 代理路径；基础内嵌不在线取包
    bundled_bytes: Optional[StaticBytes]  # 共享；只读内嵌压缩资源
    inventory: ArchiveInventory  # 共享；受审路径/大小/摘要/权限/链接库存

class ResumeMetadata:
    task_id: i64  # 独占拥有；原下载稳定 ID
    url: str  # 独占拥有；原始经过验证地址
    entity_url: str  # 独占拥有；最终实体地址
    strong_etag: Optional[str]  # 独占拥有；没有强 ETag 则不追加
    total_bytes: Optional[u64]  # 独占拥有；实体总长度
    committed_bytes: u64  # 独占拥有；已同步的连续文件长度
```

========================================

- 名称：sys_model_deployments
- 类型：新增数据库表
- 说明：SQLite/PostgreSQL 同步迁移。类型统一为 TEXT/BIGINT/INTEGER，JSON 用 TEXT 保持 sqlx Any 的读写一致；全部 SQL 参数绑定，不拼接身份。以下按字段作为表头列出类型与约束；每个 SQL 字段独立定义，不将 JSON 用作去重主键。
- 结构：

| task_id | owner_id | session_id | request_key | source | model | revision | variant_id | checksum | requested_backend | effective_backend | backend_reason | status | stage | cancel_requested | model_progress_json | runtime_progress_json | estimate_json | ownership_json | error_json | cleanup_error_json | row_version | created_at | updated_at | finished_at |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| TEXT PRIMARY KEY | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NOT NULL | TEXT NULL | TEXT NULL | TEXT NOT NULL | TEXT NOT NULL | INTEGER NOT NULL DEFAULT 0 | TEXT NOT NULL | TEXT NOT NULL | TEXT NULL | TEXT NULL | TEXT NULL | TEXT NULL | BIGINT NOT NULL DEFAULT 0 | TEXT NOT NULL | TEXT NOT NULL | TEXT NULL |
| 服务生成 UUID | 本机所有者，所有查询/CAS 必须带上 | 创建会话 | 完整请求摘要 | 契约三来源枚举 | 仓库模型 | 用户修订 | GGUF 相对标识 | 规范期望 SHA | auto/cpu/cuda | cpu/metal/cuda，尚未实测为空 | 中文选择说明 | 契约六态 CHECK | 契约七阶段 CHECK | CHECK IN (0,1) | PreparationProgress 序列化，含稳定下载 ID 与实际摘要 | RuntimeProgress，含来源/版本/两个包汇总进度 | 预检预算 | DeploymentOwnership；实时健康/ready_to_chat 不持久化为事实 | 原始错误 | 独立清理错误 | 非负 CAS 版本 | RFC3339 UTC | RFC3339 UTC | 仅终态非空 |

索引：`UNIQUE(owner_id,request_key) WHERE status IN ('queued','running')`；另建 `(owner_id,status)` 恢复索引。事务内插入/读取在途与 CAS 更新，索引约束是最终并发保障。终态约束由仓储转移函数统一实施，JSON 反序列化错误是存储错误，不转成默认成功值。

迁移文件：`migrations/{sqlite,postgres}/0020_local_model_deployments.sql`，同时登记两个迁移目录。保留 0019 和所有 sys_downloads 字段，不重写历史数据。部署记录不级联删除模型/运行时；本轮不建清理历史任务 API，不另建运行时数据库表。

========================================

- 名称：客户端 DeploymentSession / PreparedModel 调整
- 类型：chat 模块内状态
- 说明：目录任务与本地替身分支分开，不能为适配新接口伪造本地路径。
- 结构：

```python
class DeploymentSession:
    server_base: str  # 独占拥有；任务所属服务，换服务不能继续取消旧任务
    task_id: str  # 独占拥有；部署 UUID
    run_id: u64  # 独占拥有；现有过期结果隔离标识
    selection: CatalogSelection  # 独占拥有；目录选择
    requested_backend: RequestedBackend  # 独占拥有；用户主动选择
    snapshot: Optional[DeploymentTaskData]  # 独占拥有；最近真实状态

class PreparedModel:
    model: ChatModel  # 独占拥有；沿用模型
    variant: ModelVariant  # 独占拥有；沿用变体
    actual_sha256: str  # 独占拥有；来自成功任务的真实模型摘要
    local_path: Optional[str]  # 独占拥有；仅旧本地替身/手动分支使用，目录部署为 None
    route_model_id: Optional[str]  # 独占拥有；目录任务返回的服务端路由 ID
    deployment_task_id: Optional[str]  # 独占拥有；刷新后核对当前 ready_to_chat
```

准备状态持久化只保存 server_base、task_id、选择身份与 backend，不保存模型路径、推理密钥或自造终态。桌面写用户数据根的小 JSON；Web 用已有页面浏览器存储调用方式保存任务 ID，LiveView 刷新重新 GET。无法恢复记录时显示未恢复，不重新 POST。

任务上下文 `TaskContext` 只共享 Store/Node/模型与运行时准备器、实际键锁表和 owner_lock，不反向持有 DeploymentTasks 或其 workers。worker 完成包装器必须处理/记录 Result；后台任务不持有自己的 JoinHandle 所有者，避免 Arc 环。每个上下文字段只释放自己的共享引用，不单独关闭他人资源。

`build_application` 私有组合函数返回 Router 和同一 DeploymentTasks 生命周期句柄；两个既有 create_app 公有函数继续只返回 Router，start_server 正常退出则显式调用 shutdown 并等待。低层 Router 被异常丢弃时只触发取消信号，不能在 Drop 中 await 或宣称清理完成；仍在结算的上下文持有 owner_lock，防旧任务未清理完新实例就取得锁。进程/运行时异常退出依赖既有 kill_on_drop 兜底，下次启动仍按 interrupted 处理，不把这种兜底当作正常取消验收。

所有权销毁顺序：先停止 UI 轮询（不等于取消服务任务），服务关机时通知/结算父任务，等待已开始的部署完成或失败并精确停止本会话自有代次（含成功后由服务持有的运行实例，不停止用户显式程序的其他进程），释放订阅，再结算共享准备/清理暂存，最后释放数据库引用和数据根文件锁。永久有效模型/运行时缓存不随服务析构删除。进程 kill_on_drop 是异常退出兜底，不能替代有错误返回的正常清理。

## 五、函数定义

每个条目明确函数名、公开属性、多线程属性和边界。`公有接口` 指 HTTP 契约边界，不等于 Rust pub；处理器仍是私有函数。`公有函数` 只因实际跨文件/跨 crate 使用而开放，范围以本表为上限。其余辅助函数一律私有，不因测试导出。类型别名沿用第四节。

校验规则中“白盒”使用可恢复 Result 检查内部契约，不对外部输入 assert/panic；“不校验”指不重复检查调用者已保证的函数参数，仍必须验证自己读取的网络、归档、磁盘和子进程等新外部数据。

========================================

### F01 create_deployment_task

- 函数名称：`create_deployment_task`
- 公开属性：公有接口（Rust 私有）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::api::model_deployments`
- 职责说明：校验并适配 POST，不在处理器里下载或启动模型。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| state | `State<AppState>` | 是 | 注入状态 |
| body | `Result<Json<CreateDeploymentRequest>, JsonRejection>` | 是 | 16KiB 限制下严格 JSON |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Response` | 202 与 Location/data/reused；失败为契约 HTTP 错误 |

- 错误返回：400 invalid_deployment_request；413 request_too_large；503 deployment_service_unavailable；500 deployment_storage_failed
- 校验规则：黑盒校验：拒绝未知/重复/错类型字段，逐项执行契约第 5 节格式/长度/枚举约束，再转换 DeploymentCommand。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentTasks::submit |
| 被调用 | api::model_deployments::routes |

========================================

### F02 get_deployment_task

- 函数名称：`get_deployment_task`
- 公开属性：公有接口（Rust 私有）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::api::model_deployments`
- 职责说明：读取本机任务及同代次实时视图，不触发启动。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| state | `State<AppState>` | 是 | 本机门面 |
| task_id | `Path<String>` | 是 | 不透明部署 UUID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Response` | 200；Cache-Control:no-store；运行中 Retry-After:1 |

- 错误返回：400 invalid_task_id；404 deployment_task_not_found；500 deployment_storage_failed
- 校验规则：黑盒校验：UUID 格式和当前 owner 归属；没有本机记录视为未找到。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentTasks::snapshot |
| 被调用 | api::model_deployments::routes |

========================================

### F03 cancel_deployment_task

- 函数名称：`cancel_deployment_task`
- 公开属性：公有接口（Rust 私有）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::api::model_deployments`
- 职责说明：登记取消意图，区分已接受和清理完成。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| state | `State<AppState>` | 是 | 本机门面 |
| task_id | `Path<String>` | 是 | 部署 UUID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Response` | 202 清理中或 200 已取消；不把 cancel 转成 stop |

- 错误返回：400 invalid_task_id；404 deployment_task_not_found；409 deployment_not_cancellable；500 deployment_storage_failed
- 校验规则：黑盒校验：UUID/owner；无请求体；终态按契约拒绝或幂等返回。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentTasks::cancel |
| 被调用 | api::model_deployments::routes |

========================================

### F04 resolve_model（既有扩展）

- 函数名称：`resolve_model（既有扩展）`
- 公开属性：公有接口（保持 Rust 私有）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::api::model_resolution`
- 职责说明：保留原解析响应，添加 backend 与候选预算，按后端判定可部署候选。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| state | `State<AppState>` | 是 | 既有注入状态 |
| request | `ResolveModelRequest` | 是 | 旧字段加可选 backend，默认 auto |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Response` | 原 resolve 响应；候选额外含 deployment_estimate |

- 错误返回：沿用既有 resolve 错误码；不新增 HTTP 错误码
- 校验规则：黑盒校验：既有模型/来源/修订校验加 backend 严格枚举；不能抹掉原资源要求。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | discover_manifest_at；estimate_deployment |
| 被调用 | 既有 resolve 路由 |

========================================

### F05 routes

- 函数名称：`routes`
- 公开属性：公有函数（pub(super)）
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`server::api::model_deployments`
- 职责说明：仅注册三个新增任务路由、限体和 no-store/拒绝适配。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| 无 | — | — | 无业务入参；方法接收者按所属类型借用 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Router<AppState>` | 新增任务路由；不改变旧手动和下载路由 |

- 错误返回：无
- 校验规则：白盒校验：无外部参数，编译期限定路由集合。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | create/get/cancel_deployment_task |
| 被调用 | server::api::routes |

========================================

### F06 DeploymentTasks::new

- 函数名称：`DeploymentTasks::new`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：DeploymentTasks 方法
- 所属模块或包：`server::model_deployment`
- 职责说明：注入现有能力，取得 owner/单实例锁，恢复本机记录后启动有限后台调度。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| db | `Arc<Database>` | 是 | 现有连接 |
| node | `Arc<NodeDeploymentState>` | 是 | 进程门面 |
| downloads | `Arc<DownloadManager>` | 是 | 唯一下载器 |
| runtimes | `Arc<ManagedRuntimes>` | 是 | 共享运行时所有者 |
| data_root | `PathBuf` | 是 | 组合根推导；测试临时根 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Arc<DeploymentTasks>, TaskError>` | 初始化完成的服务；失败不接受任务 |

- 错误返回：deployment_service_unavailable；deployment_storage_failed；service_restarted 仅写恢复记录
- 校验规则：白盒校验：根是明确绝对用户数据根/测试根，连接可用，依赖不新建数据库；外部 owner 文件单独严格解码。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentTasks::recover；DeploymentStore；既有数据库路径接口 |
| 被调用 | create_app_with_node_request_state |

========================================

### F07 DeploymentTasks::submit

- 函数名称：`DeploymentTasks::submit`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：DeploymentTasks 方法
- 所属模块或包：`server::model_deployment`
- 职责说明：对已校验命令原子登记/复用任务，持有后台任务而非依赖 HTTP 请求存活。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| command | `DeploymentCommand` | 是 | HTTP 已校验命令 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(DeploymentTask, bool), TaskError>` | 持久化快照及 reused |

- 错误返回：deployment_storage_failed；deployment_service_unavailable
- 校验规则：白盒校验：严格命令不变量、当前 owner 服务可接受作业；不重复解析原 JSON。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentStore::insert_or_join；run_task |
| 被调用 | create_deployment_task |

========================================

### F08 DeploymentTasks::snapshot

- 函数名称：`DeploymentTasks::snapshot`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：DeploymentTasks 方法
- 所属模块或包：`server::model_deployment`
- 职责说明：合并持久化任务和当前同代次视图，计算 ready_to_chat。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `Uuid` | 是 | 已通过边界格式校验 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Option<DeploymentTaskData>, TaskError>` | 只返回当前 owner；实时视图缺失时 ready_to_chat=false |

- 错误返回：deployment_storage_failed
- 校验规则：白盒校验：当前服务 owner 已初始化；不信任数据库中的历史健康值。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentStore::load；NodeDeploymentState::status；current_task_view |
| 被调用 | get_deployment_task；POST/DELETE 响应适配 |

========================================

### F09 DeploymentTasks::cancel

- 函数名称：`DeploymentTasks::cancel`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：DeploymentTasks 方法
- 所属模块或包：`server::model_deployment`
- 职责说明：CAS 提交 cancel_requested 后通知该父任务，返回契约取消判定。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `Uuid` | 是 | 本机任务 ID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<CancelDecision, TaskError>` | 未找到/不可取消/已接受/已取消的内部判定，由 HTTP 映射 |

- 错误返回：deployment_storage_failed；deployment_not_cancellable
- 校验规则：白盒校验：owner 与任务版本一致，先持久化再通知；错误不伪装成功。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentStore::load；DeploymentStore::compare_update |
| 被调用 | cancel_deployment_task |

========================================

### F10 DeploymentTasks::recover

- 函数名称：`DeploymentTasks::recover`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：DeploymentTasks 方法
- 所属模块或包：`server::model_deployment`
- 职责说明：持锁启动时中断当前 owner 上次未完成记录，不重放启动。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| 无 | — | — | 无业务入参；方法接收者按所属类型借用 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), TaskError>` | 本机恢复结束 |

- 错误返回：deployment_storage_failed；cleanup_failed
- 校验规则：不校验；前置：owner 文件和排他锁已取得；读取记录属于新外部输入，按 schema 解码。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentStore::interrupt_incomplete；既有 LocalRouteAttacher 的 quarantine/detach |
| 被调用 | DeploymentTasks::new |

========================================

### F11 run_task

- 函数名称：`run_task`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::model_deployment`
- 职责说明：执行预检、两个并行准备、按实际键串行启动、健康/路由核对和终态提交。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| context | `TaskContext` | 是 | 各能力的共享引用，不反向持有 worker 所有者 |
| task_id | `Uuid` | 是 | 已持久化任务 |
| cancelled | `CancellationToken` | 是 | 仅该父任务通知 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), TaskError>` | 已提交终态或明确存储失败；不丢后台错误 |

- 错误返回：契约 8.2 全部适用阶段码；禁止新增对外码
- 校验规则：不校验；前置：submit/recover 已建立任务所有权。每阶段新外部数据仍按相应边界验证。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | confirm_artifact；estimate_deployment；prepare_model；ManagedRuntimes::prepare；NodeDeploymentState::start_verified；finish_task |
| 被调用 | DeploymentTasks::submit 的服务级 worker |

========================================

### F12 finish_task

- 函数名称：`finish_task`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::model_deployment`
- 职责说明：在取消/失败时先清理自有资源，再用 CAS 提交终态；成功提交同时检查取消位。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task | `DeploymentTask` | 是 | 最新版本任务 |
| outcome | `TaskOutcome` | 是 | 阶段结果及原始错误 |
| ownership | `Option<DeploymentOwnership>` | 否 | 只允许本任务新建代次清理 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentTask, TaskError>` | 最终快照；CAS 失败重读，绝不覆盖终态 |

- 错误返回：deployment_storage_failed；cleanup_failed
- 校验规则：不校验；前置：编排传入已建立归属的结果；不把阶段失败覆盖成正常取消。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | NodeDeploymentState::stop_owned；DeploymentEventStore::cleanup_owned_route；DeploymentStore::compare_update |
| 被调用 | run_task |

========================================

### F13 current_task_view

- 函数名称：`current_task_view`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`server::model_deployment`
- 职责说明：严格比较会话/模型/代次，投影契约 DeploymentView 并计算实时可对话状态。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task | `&DeploymentTask` | 是 | 历史结果 |
| current | `Option<&DeploymentSnapshot>` | 否 | 既有事件存储的实时视图 |
| session_id | `Uuid` | 是 | 本会话 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `(Option<DeploymentView>, bool)` | 非同代次/非本会话/过期观测为 false；未知路由值投影 null |

- 错误返回：无
- 校验规则：不校验；前置：task 已按 schema 解码；比较当前观测是业务判定，不是重复参数检查。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 无；纯投影 |
| 被调用 | DeploymentTasks::snapshot |

========================================

### F14 DeploymentStore::insert_or_join

- 函数名称：`DeploymentStore::insert_or_join`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentStore 方法
- 所属模块或包：`server::model_deployment_store`
- 职责说明：以部分唯一索引保证在途请求原子去重，冲突读取赢家任务。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| owner | `Uuid` | 是 | 当前本机 |
| session | `Uuid` | 是 | 当前会话 |
| command | `&DeploymentCommand` | 是 | 严格命令 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(DeploymentTask, bool), TaskError>` | 新建或原在途任务；数据库事务已提交 |

- 错误返回：deployment_storage_failed
- 校验规则：白盒校验：owner/session 不为 nil、摘要规范，SQL 值只绑定参数。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | Database::get_connection/pool；sqlx 事务 |
| 被调用 | DeploymentTasks::submit |

========================================

### F15 DeploymentStore::load

- 函数名称：`DeploymentStore::load`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentStore 方法
- 所属模块或包：`server::model_deployment_store`
- 职责说明：按 owner+task_id 读取并解码，不扫描所有历史。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| owner | `Uuid` | 是 | 当前本机 |
| task_id | `Uuid` | 是 | 稳定任务 ID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Option<DeploymentTask>, TaskError>` | 本机记录或 None |

- 错误返回：deployment_storage_failed
- 校验规则：白盒校验：owner 不为 nil；枚举/JSON/整数解码为存储边界检查，坏数据不默认成功。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 现有 Database 参数化查询 |
| 被调用 | snapshot/cancel/run_task |

========================================

### F16 DeploymentStore::compare_update

- 函数名称：`DeploymentStore::compare_update`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentStore 方法
- 所属模块或包：`server::model_deployment_store`
- 职责说明：带 owner/row_version/未终态条件写入快照，取消与成功判定同事务线性化。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| expected | `&DeploymentTask` | 是 | 原版本 |
| next | `&DeploymentTask` | 是 | 本次完整下一快照 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Option<DeploymentTask>, TaskError>` | Some 表示提交；None 表示 CAS 冲突需重读 |

- 错误返回：deployment_storage_failed
- 校验规则：白盒校验：任务/owner 不变，版本仅增一，流转符合六态，成功时取消位为 0，终态子项收束且 finished_at 非空。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | sqlx 参数化 UPDATE；原子事务 |
| 被调用 | cancel/run_task/finish_task |

========================================

### F17 DeploymentStore::interrupt_incomplete

- 函数名称：`DeploymentStore::interrupt_incomplete`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentStore 方法
- 所属模块或包：`server::model_deployment_store`
- 职责说明：只将本机上次 queued/running 记录标为 interrupted，设置子状态和 service_restarted。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| owner | `Uuid` | 是 | 持有排他锁的本机所有者 |
| session | `Uuid` | 是 | 新会话 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Vec<DeploymentOwnership>, TaskError>` | 需处理的旧归属证据；不包含其他 owner |

- 错误返回：deployment_storage_failed
- 校验规则：白盒校验：owner/session 有效，禁止无 owner WHERE 的全表更新；不把旧成功改为 interrupted。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentStore::compare_update；sqlx 事务 |
| 被调用 | DeploymentTasks::recover |

========================================

### F18 confirm_artifact

- 函数名称：`confirm_artifact`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::model_artifacts`
- 职责说明：重新发现所选源清单，核对文件身份，构造严格下载证据。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| client | `&reqwest::Client` | 是 | 受限发现客户端 |
| command | `&DeploymentCommand` | 是 | 已校验选择 |
| data_root | `&Path` | 是 | 受管根 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<ConfirmedArtifact, TaskError>` | 固定身份/摘要/大小/服务端 URL |

- 错误返回：model_repository_not_found；model_repository_unavailable；artifact_identity_changed
- 校验规则：白盒校验：选择不变量；元数据响应做文件类型/大小/SHA/仓库归属的黑盒输入验证。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | discover_manifest_at；既有资源配置合并 |
| 被调用 | run_task |

========================================

### F19 estimate_deployment

- 函数名称：`estimate_deployment`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::model_artifacts`
- 职责说明：按后端解释原资源门槛并合并下载/解压预算，预览无副作用。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| artifact | `&ConfirmedArtifact` | 是 | 权威工件 |
| backend | `RequestedBackend` | 是 | 用户所选后端 |
| hardware | `&HardwareSnapshot` | 是 | 真实硬件 |
| runtimes | `&ManagedRuntimes` | 是 | 只读包/缓存状态 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentEstimate, TaskError>` | 网络增量、固定包、磁盘检查、依赖说明 |

- 错误返回：unsupported_platform；unsupported_backend；insufficient_memory；insufficient_disk_space；disk_space_unavailable
- 校验规则：白盒校验：工件大小已知、算术 checked_add/sub 防溢出；缓存与文件系统是新输入，必须实际核对。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 固定 RuntimeAsset 目录；fs2::available_space；既有硬件探测 |
| 被调用 | run_task；resolve_model |

========================================

### F20 prepare_model

- 函数名称：`prepare_model`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::model_artifacts`
- 职责说明：加入模型共享作业：复核缓存或下载，实际全量算 SHA，原子启用可信 GGUF。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| artifact | `Arc<ConfirmedArtifact>` | 是 | 不可变严格证据 |
| preparations | `&ModelPreparations` | 是 | 服务级作业登记，保证同一工件只准备一次 |
| downloads | `Arc<DownloadManager>` | 是 | 现有下载门面 |
| progress | `WatchSender` | 是 | 真实进度输出 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PreparedArtifact, TaskError>` | 仅真实摘要一致时得到 Verified |

- 错误返回：model_download_failed；checksum_mismatch；deployment_storage_failed
- 校验规则：白盒校验：身份键/受管目标一致；私有字节校验不重复检查调用参数。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DownloadManager 既有 start/wait/identity/mark_verified；sha256_file |
| 被调用 | 模型共享作业所有者；run_task 订阅 |

========================================

### F21 sha256_file

- 函数名称：`sha256_file`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::model_artifacts`
- 职责说明：按有界缓冲读取真实 GGUF，返回规范实际摘要，不复制期望值。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| file | `File` | 是 | 已安全打开的受管普通文件 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Sha256, std::io::Error>` | 全文件实际 SHA-256 |

- 错误返回：上层映射 model_download_failed 或 checksum_mismatch
- 校验规则：不校验；前置：调用方已取得目标文件句柄；读取/身份变化错误必须传播。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | sha2；tokio 文件 IO |
| 被调用 | prepare_model |

========================================

### F22 ManagedRuntimes::prepare

- 函数名称：`ManagedRuntimes::prepare`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：ManagedRuntimes 方法
- 所属模块或包：`server::managed_runtime`
- 职责说明：优先显式配置，然后共享缓存作业，按需离线安装或下载 CUDA。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| backend | `RequestedBackend` | 是 | 主动请求后端 |
| progress | `WatchSender` | 是 | 本调用者进度桥接；不取消共享作业 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PreparedRuntimeInfo, TaskError>` | 带实测版本/设备/来源的现有 PreparedRuntime |

- 错误返回：bundled_runtime_missing；runtime_configuration_invalid；runtime_download_failed；runtime_checksum_mismatch；runtime_archive_invalid；vc_runtime_missing；cuda_unavailable；runtime_probe_failed
- 校验规则：白盒校验：backend 在当前平台范围，包清单完整；显式配置和缓存内容是新的外部输入，不能只查文件存在。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | install_runtime；probe_runtime；DownloadManager 既有 API |
| 被调用 | run_task；LlamaCppRuntimePreparer::prepare |

========================================

### F23 install_runtime

- 函数名称：`install_runtime`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::managed_runtime`
- 职责说明：按固定包列表验证压缩 SHA，安全解压，探测成功后原子启用新安装。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| assets | `&[RuntimeAsset]` | 是 | 清单已选定，CUDA 恰有两包 |
| staging | `TempDir` | 是 | 本共享作业拥有的暂存 |
| progress | `WatchSender` | 是 | 网络或阶段进度 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PreparedRuntimeInfo, TaskError>` | 启用完成的安装；失败显式清理 staging |

- 错误返回：runtime_download_failed；runtime_checksum_mismatch；runtime_archive_invalid；runtime_probe_failed；vc_runtime_missing；cuda_unavailable
- 校验规则：不校验；前置：准备门面已选定包；归档字节/设备/操作系统结果按新输入边界验证。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | unpack_runtime_archive；probe_runtime；已有下载 API；同文件系统 rename |
| 被调用 | ManagedRuntimes::prepare 的共享 worker |

========================================

### F24 probe_runtime

- 函数名称：`probe_runtime`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::managed_runtime`
- 职责说明：执行固定版本/设备探测，限制输出和时间，确认 CPU/Metal/CUDA 与依赖。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| executable | `&Path` | 是 | 显式或暂存中的程序 |
| backend | `RequestedBackend` | 是 | 用户请求 |
| origin | `RuntimeOrigin` | 是 | 实际来源 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PreparedRuntimeInfo, TaskError>` | 实际版本、后端、设备与选择说明 |

- 错误返回：runtime_configuration_invalid；runtime_probe_failed；vc_runtime_missing；cuda_unavailable
- 校验规则：不校验；前置：调用者已确定路径来源；程序输出/退出码是外部结果，严格解析并按句柄清理超时探测。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | tokio::process::Command；固定清单参数 |
| 被调用 | install_runtime；ManagedRuntimes::prepare |

========================================

### F25 unpack_runtime_archive

- 函数名称：`unpack_runtime_archive`
- 公开属性：公有函数（pub(super)）
- 多线程属性：普通（在 spawn_blocking 上执行）
- 函数归属：独立函数
- 所属模块或包：`server::runtime_archive`
- 职责说明：处理可信摘要之后仍不可信的归档结构，受限提取并保留安全内部链接/执行位。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| archive | `&Path` | 是 | 摘要已通过的压缩包 |
| destination | `&Path` | 是 | 独占新 staging |
| asset | `&RuntimeAsset` | 是 | 固定条目及展开限制 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), TaskError>` | 完整安全展开或错误；不得留下已启用缓存 |

- 错误返回：runtime_archive_invalid
- 校验规则：白盒校验：目标属于本作业 staging；归档每个路径/类型/链接/资源消耗做黑盒校验，细则见 3.4。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | zip/tar/flate2 安全 API；不直接调用无约束 unpack |
| 被调用 | install_runtime |

========================================

### F26 LlamaCppRuntimePreparer::prepare（既有 trait 实现）

- 函数名称：`LlamaCppRuntimePreparer::prepare（既有 trait 实现）`
- 公开属性：公有函数（保持 RuntimePreparer trait 边界）
- 多线程属性：异步
- 函数归属：LlamaCppRuntimePreparer 方法
- 所属模块或包：`server::node_runtime`
- 职责说明：保留 RuntimeRequest/PreparedRuntime 形状，手动旧入口也可取得基础运行时。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| request | `RuntimeRequest` | 是 | 运行时名称及可选版本 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PreparedRuntime, RuntimePrepareError>` | 只返回现有 executable；自动链路的后端信息留在私有门面 |

- 错误返回：保留 RuntimePrepareError::PrepareFailed 映射；底层中文原因保留
- 校验规则：白盒校验：只支持 llama.cpp，显式版本约束不能忽略；不以 None 追逐最新版。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | ManagedRuntimes::prepare |
| 被调用 | 既有 NodeDeploymentState::start_local |

========================================

### F27 LlamaCppRuntimeAdapter::for_backend

- 函数名称：`LlamaCppRuntimeAdapter::for_backend`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：普通
- 函数归属：LlamaCppRuntimeAdapter 方法
- 所属模块或包：`server::node_runtime`
- 职责说明：为自动链路固定实际后端和设备；旧 new 的可见性/语义不擅自改变。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| model_id | `String` | 是 | 服务端路由标识 |
| runtime | `&PreparedRuntimeInfo` | 是 | 探测结果 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<LlamaCppRuntimeAdapter, RuntimeAdapterError>` | 计划参数固化后的适配器 |

- 错误返回：保留 RuntimeAdapterError::PlanFailed；上层映射 process_start_failed
- 校验规则：白盒校验：model_id 满足 34 字符限制，GPU 后端设备证据存在；不接受任意参数字符串。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 既有 RuntimeAdapter::plan（增加固定后端参数生成） |
| 被调用 | NodeDeploymentState::start_verified |

========================================

### F28 NodeDeploymentState::start_verified

- 函数名称：`NodeDeploymentState::start_verified`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：NodeDeploymentState 方法
- 所属模块或包：`server::node_deployment`
- 职责说明：接收真实 Verified 工件，不走 start_local 的路径捷径；调用现有部署服务并保留完整结果。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| model_id | `String` | 是 | 完整身份/实际后端映射的确定 ID |
| artifact | `PreparedArtifact` | 是 | 模型分支实际校验结果 |
| runtime | `PreparedRuntimeInfo` | 是 | 运行时分支实际探测结果 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentSnapshot, TaskError>` | 进程结果和真实 generation；并不直接宣布任务成功 |

- 错误返回：process_start_failed；model_incompatible；health_check_failed；route_attachment_failed；cleanup_failed
- 校验规则：白盒校验：工件是 Verified，模型 ID 与归属一致，后端配置严格；部署服务 Ok(Unhealthy) 不等于成功。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentService::deploy_prepared；LlamaCppRuntimeAdapter::for_backend；DeploymentEventStore::get |
| 被调用 | run_task，持有 deployment_key 启动锁 |

========================================

### F29 NodeDeploymentState::stop_owned

- 函数名称：`NodeDeploymentState::stop_owned`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：NodeDeploymentState 方法
- 所属模块或包：`server::node_deployment`
- 职责说明：只停止本任务新建的精确代次；借用进程不触碰。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| ownership | `&DeploymentOwnership` | 是 | 已核实的创建者/代次证据 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), TaskError>` | 原代次已停止或确认不存在；不等于停止所有同名模型 |

- 错误返回：cleanup_failed
- 校验规则：白盒校验：created_by_task、session、正 generation；路由和进程各自匹配，不能用裸 PID/名称清理。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentService::stop_model_generation；DeploymentEventStore::cleanup_owned_route |
| 被调用 | finish_task；正常服务关闭 |

========================================

### F30 DeploymentEventStore::cleanup_owned_route

- 函数名称：`DeploymentEventStore::cleanup_owned_route`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：DeploymentEventStore 方法
- 所属模块或包：`server::node_deployment`
- 职责说明：按 model_id/generation/attachment ID 精确隔离并撤销自有路由，错误不只写日志。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| ownership | `&DeploymentOwnership` | 是 | 具体代次与路由证据 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), TaskError>` | 目标已撤销或不存在；新代次保持不变 |

- 错误返回：cleanup_failed
- 校验规则：白盒校验：路由绑定三要素匹配；旧事件不修改新绑定；未证明归属时拒绝清理。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 既有 LocalRouteAttacher::quarantine/detach |
| 被调用 | stop_owned；finish_task |

========================================

### F31 DeploymentService::stop_model_generation

- 函数名称：`DeploymentService::stop_model_generation`
- 公开属性：公有函数（跨 crate pub，明确新增公开）
- 多线程属性：异步
- 函数归属：DeploymentService 方法
- 所属模块或包：`node::deployment::service`
- 职责说明：在 coordinator 内比较实际 generation 后移走对应受管句柄并停止；保留旧 stop_model 对外语义。精确停止内部需对 generation 比较并取得停止操作槽位，停止失败保留原代次监控，不能给同一存活进程伪造新 generation。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| model_id | `&str` | 是 | 受管模型标识 |
| generation | `u64` | 是 | 必须为正的精确进程代次 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<bool, DeploymentError>` | true 表示停止匹配代次，false 表示不存在/已换代；不停止新代次 |

- 错误返回：沿用 DeploymentError/DeploymentErrorKind，不新增 HTTP 码
- 校验规则：白盒校验：非空模型 ID、generation>0；比较与移走在同一 coordinator 临界区，不先查后调用旧 stop_model。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 最小复用既有 stop_old_runtime；比较/移走同一临界区，停止操作槽位保护至结算；ProcessManager::stop；同代次停止事件 |
| 被调用 | NodeDeploymentState::stop_owned |

========================================

### F32 run_http_transfer

- 函数名称：`run_http_transfer`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`download::http`
- 职责说明：执行一代原生 HTTP 传输，真实更新进度，响应取消并落盘状态。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task | `HttpTransfer` | 是 | 由下载门面创建的严格传输目标 |
| client | `reqwest::Client` | 是 | 受限 HTTP 客户端 |
| cancelled | `CancellationToken` | 是 | 该下载任务取消通知 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PathBuf, DownloadError>` | 完整文件进入 Verifying，尚非上层 Verified |

- 错误返回：现有 DownloadError；新 Transport/Io 保留源错误，见六节
- 校验规则：白盒校验：稳定 ID/目标根/代次不变量；每跳 URL、DNS、HTTP 响应和文件写入是新外部边界。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | resume_decision；persist_checkpoint；既有 DownloadDB 状态/进度方法 |
| 被调用 | DownloadManager 既有 start_or_join/restore_incomplete 改造 |

========================================

### F33 resume_decision

- 函数名称：`resume_decision`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`download::http`
- 职责说明：将本轮已检查的响应头及断点证据转换为追加/从零/错误，不做多次猜测拼接。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| metadata | `Option<&ResumeMetadata>` | 否 | 侧车证据 |
| response | `&ResponseHeaders` | 是 | 状态、实体和 Content-Range |
| file_length | `u64` | 是 | 已核对连续长度 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<TransferMode, DownloadError>` | 从零、追加或实体确认后的待校验结果 |

- 错误返回：DownloadError::Failed 保留明确协议上下文
- 校验规则：不校验；前置：HTTP 门面已完成 URL 和侧车形状检查；响应内容一致性属于新输入校验。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 无；纯判定 |
| 被调用 | run_http_transfer |

========================================

### F34 persist_checkpoint

- 函数名称：`persist_checkpoint`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`download::http`
- 职责说明：先同步文件连续字节再原子落侧车；写库与侧车失配不可虚报完成。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| file | `&mut tokio::fs::File` | 是 | 本次部分文件 |
| metadata | `&ResumeMetadata` | 是 | 已经确认的实体与连续长度 |
| sidecar | `&Path` | 是 | 同受管根内侧车 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), DownloadError>` | 可安全恢复的检查点 |

- 错误返回：DownloadError::Io；数据库错误由调用方处理
- 校验规则：不校验；前置：根、文件名和实体已核对；IO 错误必须传播。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 文件 sync_data；同目录临时文件原子替换 |
| 被调用 | run_http_transfer |

========================================

### F35 DownloadManager::with_store（既有私有改造）

- 函数名称：`DownloadManager::with_store（既有私有改造）`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：不再 quick_start；建立 reqwest 管理器并恢复旧下载记录。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| db | `DownloadDB` | 是 | 现有下载数据库门面 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DownloadManager, DownloadError>` | 唯一管理器；不启动外部下载程序 |

- 错误返回：沿用 DownloadError；新增源错误见六节
- 校验规则：不校验；前置：new/with_database 已取得可用门面；恢复数据库/文件时做新输入检查。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 既有 restore_incomplete；run_http_transfer |
| 被调用 | DownloadManager::new/with_database |

========================================

### F36 build.rs::main

- 函数名称：`build.rs::main`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`server 构建脚本`
- 职责说明：按 Cargo TARGET 和 bundled-runtime feature 校验并生成唯一内嵌资源声明；绝不联网。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| 无 | — | — | 无业务入参；方法接收者按所属类型借用 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), BuildError>` | OUT_DIR 生成只读资源引用；无 feature 则无内嵌资源 |

- 错误返回：构建失败，不映射 HTTP 错误码
- 校验规则：不校验；环境/清单/归档是构建外部输入，必须验证 target、版本、文件名、长度、SHA 和展开元数据。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | sha2；固定 manifest 读取；include_bytes 生成 |
| 被调用 | cargo build |

========================================

### F37 prepare_runtime_assets.py::main

- 函数名称：`prepare_runtime_assets.py::main`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`deploy/scripts`
- 职责说明：发行环境预取固定资产并核对摘要/许可/包库存；不在用户端运行。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| target | `str` | 是 | 四目标之一 |
| output | `Path` | 是 | CI 明确暂存根 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `exit code` | 仅在资源与清单/证据一致时成功 |

- 错误返回：非零退出阻断发行；不伪造许可/签名/测试证据
- 校验规则：不校验；CLI 参数是外部边界，由 argparse 的严格 choices/路径检查负责，下载内容再核验。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 固定运行时 manifest；标准库 HTTP/归档/SHA；发行签名验证工具 |
| 被调用 | cd-release 工作流资源阶段 |

========================================

### F38 DeploymentClient::submit

- 函数名称：`DeploymentClient::submit`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentClient 方法
- 所属模块或包：`client::domains::chat::deployment`
- 职责说明：以已选凭据和后端提交异步任务，立刻保存 task_id。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| selection | `&CatalogSelection` | 是 | 目录模型/来源/修订 |
| variant | `&ModelVariant` | 是 | 量化凭据 |
| backend | `RequestedBackend` | 是 | 用户确认选择 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentTaskData, ChatError>` | 202 任务，不以此创建 PreparedModel |

- 错误返回：沿用 ChatError；保留服务端请求错误中文原因
- 校验规则：白盒校验：候选身份/摘要与当前选择一致；服务端响应再做 schema 检查，禁自动重试副作用请求。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | POST 新任务接口；save_session |
| 被调用 | HttpBackend::deploy_catalog |

========================================

### F39 DeploymentClient::get

- 函数名称：`DeploymentClient::get`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentClient 方法
- 所属模块或包：`client::domains::chat::deployment`
- 职责说明：按已有 task_id 查询，无自动 POST。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `&str` | 是 | 所属服务的 UUID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentTaskData, ChatError>` | 最新快照；任务 failed 仍是成功查询结果 |

- 错误返回：ChatError；404 不创建替代任务
- 校验规则：白盒校验：task_id 与保存服务匹配；响应身份/状态/字段一致，未知状态不当成功。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | GET 新任务接口 |
| 被调用 | poll；刷新恢复 |

========================================

### F40 DeploymentClient::cancel

- 函数名称：`DeploymentClient::cancel`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentClient 方法
- 所属模块或包：`client::domains::chat::deployment`
- 职责说明：请求取消部署任务，不使用旧下载取消接口或已运行模型停止接口。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `&str` | 是 | 当前任务 UUID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentTaskData, ChatError>` | 202 后继续查询；200 cancelled 才显示取消完成 |

- 错误返回：ChatError；409 展示原终态而非改写
- 校验规则：白盒校验：当前 session/run_id 对应同一任务；不发裸模型 ID。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DELETE 新任务接口 |
| 被调用 | 页面取消操作 |

========================================

### F41 DeploymentClient::poll

- 函数名称：`DeploymentClient::poll`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：DeploymentClient 方法
- 所属模块或包：`client::domains::chat::deployment`
- 职责说明：按 Retry-After 或一秒间隔查询，有限退避网络错误，发布两个独立分支真实进度。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `String` | 是 | 所属任务 |
| run_id | `u64` | 是 | 客户端过期结果隔离 |
| callback | `Callback<DeploymentTaskData>` | 是 | 界面回调 |
| stop | `CancellationToken` | 是 | 只停止轮询，不自动取消服务任务 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentTaskData, ChatError>` | 任务终态快照；刷新后可继续 |

- 错误返回：ChatError；连接中断显示未知/重试，不自造服务端 failed
- 校验规则：白盒校验：当前任务/服务不变；每次响应 identity/run_id 比较，终态不被旧响应覆盖。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentClient::get；ChatState::apply_deployment_snapshot |
| 被调用 | HttpBackend::deploy_catalog；刷新恢复 |

========================================

### F42 save_session

- 函数名称：`save_session`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`client::domains::chat::deployment`
- 职责说明：按平台保存可恢复任务引用，不保存密钥或模型路径。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| session | `&DeploymentSession` | 是 | 已收到服务端 task_id |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), ChatError>` | 本地恢复记录已保存；失败明确告知不会刷新恢复 |

- 错误返回：ChatError；不取消已经成功登记的服务任务
- 校验规则：不校验；前置：submit/get 已验证服务和 ID；存储是新边界，IO/浏览器异常必须传播。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 桌面原子小文件；Web 浏览器存储桥接 |
| 被调用 | DeploymentClient::submit；刷新恢复 |

========================================

### F43 restore_session

- 函数名称：`restore_session`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`client::domains::chat::deployment`
- 职责说明：读取保存的任务 ID，校验所属服务后查询；不重建旧可信文件凭据。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| server_base | `&Url` | 是 | 当前客户端服务 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Option<DeploymentSession>, ChatError>` | 可继续查询的引用或无记录 |

- 错误返回：ChatError；记录损坏/服务不符提示，不重新 POST
- 校验规则：不校验；前置：配置地址已通过客户端构造边界；持久化内容须严格解码、限大小、核对身份。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentClient::get；平台存储读取 |
| 被调用 | 聊天页面挂载 |

========================================

### F44 HttpBackend::deploy_catalog

- 函数名称：`HttpBackend::deploy_catalog`
- 公开属性：公有函数（ChatBackend 的 pub(super) 模块内契约）
- 多线程属性：异步（沿现有 BackendFuture）
- 函数归属：HttpBackend 方法
- 所属模块或包：`client::domains::chat::backend`
- 职责说明：目录分支改为提交/查询自动部署，旧本地替身 initialize 保留。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| request | `InitializationRequest` | 是 | 含当前后端选择 |
| callback | `Callback<DeploymentTaskData>` | 是 | 新任务进度回调 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `BackendFuture<PreparedModel>` | 仅成功且当前 ready_to_chat 时构造含 route_model_id 的就绪模型 |

- 错误返回：ChatError；后台 failed/cancelled/interrupted 明确映射
- 校验规则：白盒校验：是目录选择且凭据有效；缺少真实摘要/路由 ID/ready_to_chat 不放行。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DeploymentClient::submit/poll；不再调用 deploy_model_with_route |
| 被调用 | page::spawn_preparation 的目录分支 |

========================================

### F45 ChatState::apply_deployment_snapshot

- 函数名称：`ChatState::apply_deployment_snapshot`
- 公开属性：公有函数（pub(super)）
- 多线程属性：普通
- 函数归属：ChatState 方法
- 所属模块或包：`client::domains::chat::state`
- 职责说明：按 run_id/任务 ID 消费最新快照，分别呈现模型和运行时，不线性伪造阶段事件。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| run_id | `u64` | 是 | 本地操作代次 |
| snapshot | `DeploymentTaskData` | 是 | 已解码快照 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Option<StreamPlan>` | 仅真正成功可触发待发送消息；旧代次返回 None |

- 错误返回：ChatError 保存在界面状态；不新增服务端错误码
- 校验规则：白盒校验：当前 run_id/task_id/选择一致；succeeded 但 ready=false 不触发对话；不保存假路径。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 现有准备 UI 与过期回调防护 |
| 被调用 | DeploymentClient::poll 回调；刷新恢复 |

========================================

### F46 generate_via_router

- 函数名称：`generate_via_router`
- 公开属性：公有函数（pub(super)）
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`client::domains::chat::inference`
- 职责说明：目录模型通过既有 BurnCloud /v1/chat/completions 生成，不直连 llama-server、不回退模拟文本。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| base | `&Url` | 是 | 主服务根 |
| request | `GenerationRequest` | 是 | 真实 route_model_id 与历史消息 |
| api_token | `&str` | 是 | 用户在已有 Token 页面管理的数据面令牌，仅本次内存持有 |
| callback | `Callback<GenerationEvent>` | 是 | 真实流事件 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Option<InferenceMetrics>, ChatError>` | 解析真实 SSE；无 usage/速度数据时返回 None，不硬编码示例指标 |

- 错误返回：沿用既有数据面 HTTP/SSE 错误映射 ChatError，不新增端点或绕过认证
- 校验规则：白盒校验：route_model_id/消息非空，API Token 不写日志/恢复文件；SSE 做长度、UTF-8、帧序及完成标记校验。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 既有 /v1/chat/completions；decode_sse |
| 被调用 | HttpBackend::generate 的目录分支；本地替身分支保留 |

========================================

### F47 decode_sse

- 函数名称：`decode_sse`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`client::domains::chat::inference`
- 职责说明：跨网络分块解析真实 OpenAI SSE，区分 thinking/content/完成与错误。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| buffer | `&mut Vec<u8>` | 是 | 有界未完成帧缓存 |
| chunk | `&[u8]` | 是 | 本轮网络块 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Vec<GenerationEvent>, ChatError>` | 本轮完整事件；半帧留存；EOF 无完成标记报错 |

- 错误返回：ChatError；不得伪造回复或推理指标
- 校验规则：不校验；前置：调用方已建立数据面请求；网络帧本身作为新外部输入严格解析。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | serde_json；SSE 空行分帧 |
| 被调用 | generate_via_router |

### 5.1 既有下载公共 API 保持与实现约束

下列函数的签名和可见性保持原状，并分别提供独立测试；它们是既有函数，不增加新的对外下载接口。其传输生命周期由上述 HTTP 传输与 with_store 定义接续，不再通过 aria2。

| 函数名称 | 公开属性 | 多线程属性 | 入参/返回沿用 | 校验与调用 |
| --- | --- | --- | --- | --- |
| DownloadManager::new | 公有函数 pub | 异步 | 无 / Result<Self> | 初始化数据库白盒检查；调用 with_store |
| DownloadManager::with_database | 公有函数 pub | 异步 | Arc<Database> / Result<Self> | 共享连接有效；调用 with_store，不另开库 |
| DownloadManager::start_download | 公有函数 pub | 异步 | url,dir / Result<DownloadTaskHandle> | 黑盒 URL/路径边界在入口完成；身份键与稳定 ID 保留；调用原生传输 |
| DownloadManager::get_task | 公有函数 pub | 异步 | task_id / Result<Option<DownloadTaskSnapshot>> | 正数字 ID 与记录白盒核对；新快照无 aria2 RPC |
| DownloadManager::set_task_identity | 公有函数 pub | 异步 | 沿现有身份参数 / Result | 严格身份、摘要格式和稳定 ID 白盒核对；不可更换在途工件 |
| DownloadManager::mark_task_verified | 公有函数 pub | 异步 | 沿现有任务/摘要参数 / Result | 实际摘要由上层给出，白盒核对格式/任务阶段；与取消/终态串行 |
| DownloadManager::mark_task_verification_failed | 公有函数 pub | 异步 | 沿现有任务/错误参数 / Result | 白盒核对状态和错误；失败文件不得登记可信 |
| DownloadManager::cancel_task | 公有函数 pub | 异步 | task_id / Result | 白盒核对可取消状态；先落盘意图再取消该下载令牌 |
| DownloadManager::wait_for_task | 公有函数 pub | 异步 | task_id / Result<PathBuf> | 白盒 ID/阶段；Verifying 仍由上层做真实 SHA |
| DownloadManager::download | 公有函数 pub | 异步 | url,dir / Result<PathBuf> | 保留原 start+wait 语义，不假称上层已 Verified |

上述方法属于下载 crate 已有对外能力，其 URL/目录来自外部时，入口执行黑盒安全校验，不因内部调用而跳过。其余只有内部严格类型的调用按白盒契约处理。被调用方继续是现有 CLI/旧模型下载 HTTP 和新模型/运行时准备作业；错误继续用 DownloadError，恢复/进度/GID 文档同步去除强绑定 aria2 的描述。

### 5.2 既有函数的限定调整

- `discover_manifest_at` 保持现有 pub(crate)，作为已存在的公有函数使用；不把 build_variant 等私有发现细节导出。
- `LlamaCppRuntimeAdapter::plan` 保持既有 RuntimeAdapter 实现边界；增加固定后端参数但保留文件/Verified/端口检查。旧构造器保持原公开属性；新自动链路只走 for_backend。
- `DeploymentEventStore::publish/get` 保持现有边界；自动任务需要的路由错误必须能被快照可靠观察。旧 get 可能产生的 lost 等非契约视图不能原样塞入新 route_status，映射为 null、ready=false，并保留 detail 诊断。
- `create_app_with_node_request_state` 保持现有公有签名；组合根创建共享 ManagedRuntimes 与 DeploymentTasks，将新增字段放在 AppState 的 pub(crate) 范围。初始化失败必须禁用任务接受或返回错误，不给缺 worker 的服务返回 202。
- `page::spawn_preparation` 保持原函数可见性，目录分支调用 deploy_catalog；旧替身分支不重构。既有迟到 callback/run_id 检查保留。
- `HttpBackend::generate` 保持 ChatBackend 契约：目录模型委派 generate_via_router，旧本地替身仍只用于原演示模型。目录接口失败不切换模拟回复。
- `ChatBackend` 只增加本模块确实使用的 deploy_catalog 异步方法和对应实现，不扩为跨 crate 公开 trait。目录 resolve 的内部请求带所选 backend，后端变更使旧候选/预算失效并重新解析。

### 5.3 构造、停机及既有下载方法的逐函数定义

========================================

### F48 DeploymentStore::new

- 函数名称：`DeploymentStore::new`
- 公开属性：公有函数（pub(super)）
- 多线程属性：普通
- 函数归属：DeploymentStore 方法
- 所属模块或包：`server::model_deployment_store`
- 职责说明：只封装现有共享连接，不新建数据库。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| db | `Arc<Database>` | 是 | 现有连接 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `DeploymentStore` | 本模块仓储 |

- 错误返回：无
- 校验规则：白盒校验：连接已初始化；不重复外部配置校验。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 无 |
| 被调用 | DeploymentTasks::new |

========================================

### F49 ManagedRuntimes::new

- 函数名称：`ManagedRuntimes::new`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：普通
- 函数归属：ManagedRuntimes 方法
- 所属模块或包：`server::managed_runtime`
- 职责说明：建立共享运行时登记和固定资源目录引用。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| downloads | `Arc<DownloadManager>` | 是 | 唯一下载门面 |
| data_root | `PathBuf` | 是 | 受管根 |
| assets | `StaticRuntimeCatalog` | 是 | 当前构建固定包目录 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<ManagedRuntimes, TaskError>` | 无网络副作用的运行时服务 |

- 错误返回：runtime_configuration_invalid
- 校验规则：白盒校验：根/目标/固定目录一致；不提前下载 CUDA。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 固定 RuntimeAsset 清单 |
| 被调用 | 应用组合根 |

========================================

### F50 ModelPreparations::new

- 函数名称：`ModelPreparations::new`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：普通
- 函数归属：ModelPreparations 方法
- 所属模块或包：`server::model_artifacts`
- 职责说明：建立模型作业登记，下载与验证结果按完整身份共享。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| downloads | `Arc<DownloadManager>` | 是 | 唯一下载器 |
| data_root | `PathBuf` | 是 | 受管模型根 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `ModelPreparations` | 模型共享作业所有者 |

- 错误返回：无
- 校验规则：白盒校验：组合根已保证绝对受管路径与唯一下载器。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 无 |
| 被调用 | DeploymentTasks::new |

========================================

### F51 DeploymentClient::new

- 函数名称：`DeploymentClient::new`
- 公开属性：公有函数（pub(super)）
- 多线程属性：普通
- 函数归属：DeploymentClient 方法
- 所属模块或包：`client::domains::chat::deployment`
- 职责说明：建立短请求任务客户端和服务器身份，复用目录客户端的配置约定。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| base | `&str` | 是 | 主服务根地址 |
| timeout | `Duration` | 是 | 单次请求超时 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DeploymentClient, ChatError>` | 任务客户端；无请求副作用 |

- 错误返回：ChatError
- 校验规则：黑盒校验：地址 HTTP(S)、无嵌入凭证/非法路径、非零超时；任务 POST/DELETE 禁自动重试和跨域重定向。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | reqwest::ClientBuilder |
| 被调用 | HttpBackend 构造 |

========================================

### F52 DeploymentTasks::shutdown

- 函数名称：`DeploymentTasks::shutdown`
- 公开属性：公有函数（pub(crate)）
- 多线程属性：异步
- 函数归属：DeploymentTasks 方法
- 所属模块或包：`server::model_deployment`
- 职责说明：停止接受任务、通知父/共享作业并逐个等待清理，最后释放本会话拥有的模型实例和单实例锁。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| 无 | — | — | 无业务入参；方法接收者按所属类型借用 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), TaskError>` | 正常停机结果；有失败如实返回/记录，不谎报清理完成 |

- 错误返回：cleanup_failed；deployment_storage_failed
- 校验规则：白盒校验：服务生命周期只结算一次；不停止他人代次；异常 Drop 只能发取消信号，不冒充异步清理完成。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | worker JoinHandle；stop_owned；共享准备结算 |
| 被调用 | start_server 的正常退出路径；测试应用生命周期 |

========================================

### F53 build_application

- 函数名称：`build_application`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`server::lib`
- 职责说明：在既有 create_app 封装之下组合一次 AppState，同时向实际启动入口保留自动部署生命周期句柄。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| db | `Arc<Database>` | 是 | 现有连接 |
| enable_liveview | `bool` | 是 | 既有 Web 配置 |
| jwt_secret | `JwtSecret` | 是 | 既有认证 |
| internal_secret | `InternalSecret` | 是 | 既有内部密钥 |
| node_request_state | `NodeRequestState` | 是 | 既有节点状态 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(Router, Arc<DeploymentTasks>), anyhow::Error>` | 路由与同一套服务生命周期；create_app 公有签名不变 |

- 错误返回：原初始化错误；任务服务失败不接受 202
- 校验规则：不校验；前置：既有公有构造器完成参数边界校验；数据根由组合根推导。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | ManagedRuntimes::new；DeploymentTasks::new；原 AppState/路由初始化 |
| 被调用 | create_app_with_node_request_state；start_server |

========================================

### F54 DownloadManager::new

- 函数名称：`DownloadManager::new`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：保留无参初始化，改用原生传输而非 aria2。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| 无 | — | — | 无业务入参；方法接收者按所属类型借用 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DownloadManager, DownloadError>` | 初始化完成的管理器 |

- 错误返回：沿既有 DownloadError，Transport/Io 保留来源
- 校验规则：白盒校验：初始化环境和数据库可用；恢复外部记录重新核对。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DownloadDB::new；with_store |
| 被调用 | 现有调用方及下载器示例 |

========================================

### F55 DownloadManager::with_database

- 函数名称：`DownloadManager::with_database`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：使用主服务已打开连接初始化唯一下载器。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| db | `Arc<Database>` | 是 | 现有共享连接 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DownloadManager, DownloadError>` | 不另开数据库的管理器 |

- 错误返回：沿既有 DownloadError
- 校验规则：白盒校验：已初始化连接；不在客户端重复创建下载器。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DownloadDB::new_shared_db；with_store |
| 被调用 | 现有服务 AppState 初始化 |

========================================

### F56 DownloadManager::start_download

- 函数名称：`DownloadManager::start_download`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：保留稳定任务登记/复用接口，选择原生 HTTP worker。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| url | `&str` | 是 | 文件 HTTP(S) URL |
| dir | `&str` | 是 | 旧模型根或明确的受管子根 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DownloadTaskHandle, DownloadError>` | 稳定数字 ID、当前不透明 gid 与状态 |

- 错误返回：InvalidUrl；InvalidDirectory；InvalidFilename；Database；Transport；Io
- 校验规则：黑盒校验：URL/DNS/根/文件名边界完成后转严格 HttpTransfer；内部调用不能跳过该公共安全边界。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | 原 start_or_join；run_http_transfer |
| 被调用 | 旧下载 HTTP；prepare_model；install_runtime |

========================================

### F57 DownloadManager::get_task

- 函数名称：`DownloadManager::get_task`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：按稳定数字 ID 读取原生传输快照。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `i64` | 是 | 下载稳定 ID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<Option<DownloadTaskSnapshot>, DownloadError>` | 不存在为 None；已有字段保持兼容 |

- 错误返回：Database；InvalidRecord
- 校验规则：白盒校验：正数 ID，数据库字段/字节计数/路径按外部存储边界解码；不询问旧 aria2 GID。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | DownloadDB::get_by_id；本管理器传输状态 |
| 被调用 | 新旧模型接口；下载进度订阅 |

========================================

### F58 DownloadManager::set_task_identity

- 函数名称：`DownloadManager::set_task_identity`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：保留原始模型身份与期望摘要持久化。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `i64` | 是 | 稳定下载 ID |
| model | `&str` | 是 | 仓库模型 |
| source | `&str` | 是 | 来源 |
| revision | `&str` | 是 | 修订 |
| variant_id | `&str` | 是 | 所选文件 |
| artifact_uri | `&str` | 是 | 上层已经验证的来源地址 |
| checksum | `&str` | 是 | 期望 SHA |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), DownloadError>` | 身份登记结果；此函数不表示文件实际通过 SHA |

- 错误返回：TaskNotFound；Database；InvalidRecord
- 校验规则：白盒校验：ID、规范身份和 SHA/URI 与任务一致；不接受在途换文件。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | get_task；DownloadDB 既有身份更新 |
| 被调用 | 旧下载注册；prepare_model |

========================================

### F59 DownloadManager::mark_task_verified

- 函数名称：`DownloadManager::mark_task_verified`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：将上层全量计算出的实际 SHA 登记为验证成功。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `i64` | 是 | 稳定下载 ID |
| actual_sha256 | `&str` | 是 | 真实计算结果 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), DownloadError>` | 任务 Succeeded；与取消/失败终态串行 |

- 错误返回：TaskNotFound；Database；InvalidRecord
- 校验规则：白盒校验：ID/摘要规范、实际与已登记期望一致、允许阶段且未取消；不能复制期望值假验。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | get_task；DownloadDB::mark_verified |
| 被调用 | 真实模型/运行包验证调用方 |

========================================

### F60 DownloadManager::mark_task_verification_failed

- 函数名称：`DownloadManager::mark_task_verification_failed`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：持久化真实校验失败和可得的实际摘要。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `i64` | 是 | 稳定下载 ID |
| actual_sha256 | `Option<&str>` | 否 | 可得时的真实摘要 |
| error | `&str` | 是 | 明确失败原因 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<(), DownloadError>` | 失败终态；不登记为可信缓存 |

- 错误返回：TaskNotFound；Database；InvalidRecord
- 校验规则：白盒校验：ID/摘要格式与状态合理；不覆盖已取消/已成功终态。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | get_task；DownloadDB::mark_verification_failed |
| 被调用 | 真实模型/运行包验证调用方 |

========================================

### F61 DownloadManager::cancel_task

- 函数名称：`DownloadManager::cancel_task`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：保留旧下载取消语义，原生令牌仅针对该下载任务。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `i64` | 是 | 下载稳定 ID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<DownloadTaskSnapshot, DownloadError>` | 取消快照或原已取消快照 |

- 错误返回：TaskNotFound；TaskNotCancellable；Database；Compensation
- 校验规则：白盒校验：ID/当前状态，先持久化再取消；不把父任务取消直接转此调用以免影响共享下载。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | get_task；DownloadDB 取消写入；本下载令牌 |
| 被调用 | 原下载 HTTP DELETE；不由新父任务取消直接调用 |

========================================

### F62 DownloadManager::wait_for_task

- 函数名称：`DownloadManager::wait_for_task`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：等待已登记下载文件落盘，保留等待者通知与错误传播。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| task_id | `i64` | 是 | 稳定下载 ID |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PathBuf, DownloadError>` | 传输完成的文件；不替代上层全量 SHA |

- 错误返回：TaskNotFound；Failed；NotificationClosed；Database
- 校验规则：白盒校验：正数 ID/允许状态，返回路径确属任务；等待者断开不取消共享传输。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | get_task；现有 oneshot 等待机制 |
| 被调用 | DownloadManager::download；prepare_model；install_runtime |

========================================

### F63 DownloadManager::download

- 函数名称：`DownloadManager::download`
- 公开属性：公有函数（保持 pub）
- 多线程属性：异步
- 函数归属：DownloadManager 方法
- 所属模块或包：`download::operations`
- 职责说明：保留原 start+wait 便捷调用。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| url | `&str` | 是 | 外部文件地址 |
| dir | `&str` | 是 | 允许下载根 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| result | `Result<PathBuf, DownloadError>` | 文件落盘；仍由上层验证 SHA |

- 错误返回：沿 start_download/wait_for_task 错误
- 校验规则：黑盒校验委托同一 start_download 公共入口完成，不引入第二套宽松下载路径。
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用 | start_download；wait_for_task |
| 被调用 | 现有 CLI/示例调用方 |

## 六、公开清单、调用关系与资源边界

### 6.1 Rust 可见性清单

| 项目 | 本期允许的可见性 | 约束 |
| --- | --- | --- |
| 新 HTTP 请求/响应 DTO、处理器 | 私有 | serde 不要求 pub；只有 routes 为 pub(super) |
| 第五节标注 pub(crate)/pub(super) 的函数与跨文件参数类型 | 严格限本 crate/父模块 | 无跨文件需要的字段仍私有；不添加 pub use 泄露这些类型 |
| DeploymentService::stop_model_generation | pub | 唯一新增跨 crate 操作方法；目的与输入/代次语义见对应函数条目；必须中文 rustdoc、独立测试及文档示例 |
| DownloadManager 既有所有公有方法/字段 | 保持原可见性/签名 | 不删除或重命名 task_id/gid，不新增任意路径或 URL 的 HTTP 接口 |
| DownloadError::Transport(reqwest::Error) | 公有枚举新增变体 | 保留源错误链；不删除既有 Aria2/Failed 等变体；更新 workspace 内穷尽匹配并记入兼容说明 |
| DownloadError::Io(std::io::Error) | 公有枚举新增变体 | 保留 IO 源；失败清理仍可用现有 Compensation 保留两个原因 |
| RuntimeRequest/PreparedRuntime/RuntimePreparer/RuntimeAdapter | 保持既有 public 结构/trait 形状 | 不为 backend 给既有公共结构强加必填字段；详细元数据留在 PreparedRuntimeInfo |
| ChatBackend 新方法、PreparedModel 新字段及客户端新方法 | pub(super) 只在 chat 内 | 不是外部 Rust SDK，不新增 HTTP 端点；不向前端下发任意可执行命令 |

HTTP 对外 error.code 必须来自契约第八节，Transport/Io 等 Rust 错误只由适配层映射已有阶段码，不泄露内部路径、秘钥或底层堆栈。每个公有 API 的实现和文档测试单独文件组织，私有单元测试用同模块独立 tests 文件接入，不扩大可见性。

### 6.2 完整调用链

```text
HTTP POST
  -> create_deployment_task（黑盒）
  -> DeploymentTasks::submit（在途原子去重并持久化）
  -> run_task
       -> confirm_artifact -> discover_manifest_at（权威身份）
       -> estimate_deployment（按后端/同文件系统预检）
       -> 并行订阅：prepare_model / ManagedRuntimes::prepare
       -> 持有实际 deployment_key 锁
       -> NodeDeploymentState::start_verified
            -> LlamaCppRuntimeAdapter::for_backend
            -> DeploymentService::deploy_prepared
            -> 既有 ProcessManager / ReadinessProbe / HealthProbe / EventPublisher
            -> 既有 LocalRouteAttacher
       -> 观察同代次健康与 routable，不能只看 deploy_prepared 的 Ok
       -> finish_task -> 仓储 CAS（取消优先判定）
HTTP GET -> snapshot -> 持久化记录 + 当前同代次视图 -> ready_to_chat
HTTP DELETE -> cancel -> 仓储取消位 -> 唤醒父任务 -> 精确代次/路由清理 -> 终态
Chat UI -> DeploymentClient -> 上述三个接口；成功 model_id -> 原 BurnCloud 数据面
```

- 部署服务事件中的进程状态、generation 和路由 attachment 要与 task.ownership 对照。route attachment 失败或尚不可路由时最多等待 30 秒（每秒观察），到期 route_attachment_failed；不是永远等，也不是“已启动即可对话”。
- 健康检查沿现有 HttpProbe 600 秒 readiness/单请求 10 秒；本机探测客户端强制 no_proxy 并禁止重定向，避免探测经外部代理失真，该设置不应用于模型源下载；程序确定退出或明确 GGUF 不兼容时可提前失败。只在有明确加载诊断时用 model_incompatible，其余为 process_start_failed/health_check_failed，不把所有超时都臆断为模型不兼容。
- GET 的实时视图同时检查本会话/同代次/Ready/routable 及现有事件存储新鲜度，不把 last ready 持久化当实时健康。既有 route_status=lost 等新契约外字符串映射 null，ready_to_chat=false。
- 停止走原接口。自动清理只用精确代次方法；不先 status 后调用按名称 stop，避免检查与停止之间换代。route 清理同样比较 binding generation/attachment。

### 6.3 客户端展示和真实推理

- 后端选项提供 auto/cpu，仅 Windows 展示可选 cuda。切换后端重新 resolve，展示预计新增网络量、完整 CUDA 双包量/缓存增量、磁盘空间和依赖/驱动要求，用户确认后才 POST。
- 卡片同时展示“模型：下载/校验”和“运行时：离线解压/在线双包/探测”。解压、探测、健康、路由只显示文字阶段；下载总量未知时不显示百分比，安全重下允许字节回退。
- 页面刷新优先恢复 task_id 后 GET；关闭卡片/断开 WebSocket 只停止 UI 订阅，主动点取消才 DELETE。旧 run_id 响应、旧服务器任务和旧 task_id 不修改新选择。
- 新目录链路没有 model_path 响应，因此 PreparedModel.local_path 改为 Option，仅旧分支使用；目录分支依据真实 actual_sha256、route_model_id 和 ready_to_chat 构造就绪凭据，不填入虚构路径以通过旧 is_valid。
- 现有 HttpBackend.generate 不能继续对目录模型调用 LocalBackend.generate。目录会话必须以任务返回 model_id 向主服务 `/v1/chat/completions` 发出真实请求，不能直连进程端口。这个数据面端点已存在，不在本轮增加新公共 HTTP 入口。
- 数据面原有令牌验证不能绕过：聊天界面提供现有 BurnCloud API Token 的内存输入/选择，用户仍通过原令牌管理能力创建令牌；不新建自动发令牌体系、不硬编码万能 Token。缺少/无效令牌明确提示使用既有管理入口，不能回退模拟回复。令牌不属于运行时部署前置，不写部署任务/恢复记录/日志，不要求用户配置运行时环境变量。
- SSE 必须支持分块 UTF-8/多行数据/半帧、错误与完成标记；流截断视为失败。没有真实 usage/TTFT/tokens 数据时不显示模拟指标或费用；本期不引入新计费或认证语义。

### 6.4 私有辅助类型约定

以下类型没有独立业务生命周期，不扩为新状态或公共接口：

- `TaskError` 仅 code/message，内容为契约 4.4；HTTP DTO 使用契约 DeploymentTaskData/DeploymentView，全字段按接口文档，无额外路径/程序字段。
- `DeploymentStore` 只共享 Arc<Database>；新建时不连接别的数据库。`ModelPreparations` / `ManagedRuntimes` 分别持有自身 HashMap 作业登记和 Arc<DownloadManager>，结果只在各自模块内共享。
- `HttpTransfer` 独占 task_id、gid、Url、根内目标路径、ResumeMetadata 和进度发送端，不持有用户给的可执行命令。`ResponseHeaders`/`TransferMode` 是 HTTP 内部解析结果，不序列化为部署状态。
- `CancelDecision` 表示已接受/已取消/未找到/不允许的内部返回分支，`TaskOutcome` 表示准备/启动结果或原始失败。它们不是新的父任务状态；不放入 HTTP status 字段。
- `DeploymentView` 是查询时临时投影，process_ready、route_status、observed_at 不作永久可信证明。新会话无可靠视图时保持 null/false。

## 七、固定资源、构建和四目标发行

### 7.1 固定源包登记

以下数值来自前序阶段已核对的 b8581 发行资产记录；它们是源包字节数与 SHA-256，不是 BurnCloud 最终发行体积，也不是四平台实机验收证明。发行脚本必须再次对实际取得字节核对，任何不符均失败，不自行更新为“最新摘要”。

| 目标/用途 | 固定源包 | 字节数 | SHA-256 |
| --- | --- | ---: | --- |
| Windows x64 CPU | llama-b8581-bin-win-cpu-x64.zip | 38707858 | 71fffc57588b12142a1f0b37f8a1f0a1b122d14f70435f3689dd6399ba4317e7 |
| macOS ARM64 原生 | llama-b8581-bin-macos-arm64.tar.gz | 39574927 | 0c79d0c1370c81137d58cdc720881dfbab67cae02769d9eaf895f127d0785056 |
| macOS x64 原生 | llama-b8581-bin-macos-x64.tar.gz | 100650354 | e03188ae51dc1c3ba8d2a5d6220ac0d6639a47db64af4d0c93bc05b494a98d0b |
| Ubuntu 24.04 x64 CPU | llama-b8581-bin-ubuntu-x64.tar.gz | 31371639 | 8424fecb5a260f15a3aa90016a47242621881b78dc5ebb4cb418d0134d5cbe43 |
| Windows CUDA 12.4 服务 | llama-b8581-bin-win-cuda-12.4-x64.zip | 232525426 | 0be0ef75d1544e836ba8e90c4fe02cad253cb8983ebcb68ac774183a28f400d5 |
| Windows CUDA 12.4 cudart | cudart-llama-bin-win-cuda-12.4-x64.zip | 391443627 | 8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6 |

CUDA 固定客户端下载前缀为 `https://gh-proxy.com/https://github.com/ggml-org/llama.cpp/releases/download/b8581/`，仅拼上述两个已知文件名；不访问 GitHub API。代理不可用或返回错误页必须显式失败且保留安全断点，不承诺该第三方永久可用，也不临时用 Gitee 源码仓库冒充预编译运行包。

### 7.2 构建输入与签名封装

- 根 Cargo 增加 `bundled-runtime = ["burncloud-server/bundled-runtime"]`，server 增加同名 feature 和 build.rs。普通开发默认不启用，官方发行必须启用。
- `deploy/runtime/manifest.json` 登记固定源包版本、精确目标、压缩 SHA/大小、展开上限、server 相对路径、依赖及许可清单；`deploy/runtime/README.md` 说明获取/校验和再分发流程。大运行包放 CI 明确资源目录，不提交用户本机资源，不扩展根目录写入范围。
- 发行资源阶段使用 `deploy/scripts/prepare_runtime_assets.py`；构建通过 `BURNCLOUD_RUNTIME_ASSET_DIR` 显式指定已准备资源根。build.rs 仅读文件和校验，不包含 HTTP、curl、包管理器或网络回退。无资源、错误 target/version/name/长度/SHA/许可记录即构建失败。
- 基础包签名封装与源包校验分开：若 macOS 源包不能直接满足签名/公证要求，发行环境先核验源包，再对包含的 Mach-O/dylib 按由内到外顺序签名，制作本平台派生 tar.gz，并将签名、公证/验证证据与派生包 SHA 固定到本次资源锁定清单。不能在用户端签名、删除隔离属性，也不能重签后继续沿用源包 SHA。
- 发布资源阶段在目标基线上审计动态依赖。Ubuntu 基线以外但可合法再分发的必要 so 由发行环境并入同一基础包，配置 `$ORIGIN`/相邻 lib 等相对加载路径；macOS 相对加载元数据修正必须先于签名。不能等用户首次部署再要求 apt/Homebrew/编译器补依赖，不能打包替换系统 glibc。处理后的包同样登记派生摘要、完整库存与许可证，并用干净系统实际执行探测后才能发行。
- 资源锁定清单包含 source_sha256、embedded_sha256、压缩大小、展开库存和版本/target，随发行记录归档；build.rs 对清单与实际唯一基础包做一致性检查，将实际 embedded_sha256 及只读库存一并编入客户端。无需派生时两摘要相等；macOS 合法重封装时 RuntimeAsset.sha256 指实际内嵌包摘要，源摘要另保留为溯源证据。CUDA 在线包仍使用 7.1 的固定下载摘要，不接受客户端替换。
- Windows 的微软 VC++ x64 安装程序作为独立受审依赖资源随本平台程序内嵌，摘要/签名/授权依据单独锁定。不把不含 VC++ 的上游 ZIP 当作依赖齐全；授权、固定安装程序摘要或签名证据不具备时官方 Windows 发行失败，不填占位假摘要。
- 运行时资源和依赖遵循 b8581/平台固定选择。macOS 重签重封装不是更新 llama.cpp 版本或增加第二个平台包；源包大小与派生包大小的差异记入体积报告。
- 未填写的发行事实不是待选架构：设计确定为“缺证据阻断”。展开库存、签名凭据、授权文件和实机证据必须从真实准备步骤取得；本开发文档不伪造它们，也不提供跳过门禁开关。

### 7.3 四目标便携发行

| Rust target | 内嵌基础类型 | 实机最低候选基线 | GUI |
| --- | --- | --- | --- |
| x86_64-pc-windows-msvc | CPU ZIP | Windows 10/11 x64 | 沿现有桌面 |
| aarch64-apple-darwin | 原生 tar.gz | macOS 14+ | 沿现有桌面 |
| x86_64-apple-darwin | 原生 tar.gz | macOS 13.3+ | 沿现有桌面 |
| x86_64-unknown-linux-gnu | Ubuntu x64 tar.gz | Ubuntu 24.04 LTS x64 | 既有服务端/Web |

- CI 在原生目标/相应架构环境构建，不用仅链接成功充当实机部署测试。macOS x64 需要可用的 Intel runner/签名资源，无资源就不发布该目标，不伪装在 ARM 上运行通过。
- 外层保持匹配现有 self_update 命名的 `burncloud-<version>-<target>.zip`，根部含可更新主二进制和许可证/说明；Unix 主程序执行位在 ZIP 和解压验证中保留。基础归档仍内嵌，CUDA 不在普通发行 ZIP 或 exe 中。
- 不改变通用 updater 协议或引入 MSI/DMG/AppImage。对当前 updater 的四目标替换/执行位做兼容测试，未通过不能发相应自动更新。许可证/NOTICE 的必要内容还必须内嵌到主程序可导出的资源中，避免单文件更新丢掉合规说明。
- 每个目标用相同代码/优化设置分别构建带/不带 bundled-runtime 的基准，报告主程序和 ZIP 绝对大小及差值、源/派生压缩资源、微软依赖资源增量。不宣称客户端体积不变，不将四平台资源一起打包。
- 发布前检查软件许可证、必要再分发授权、BurnCloud 主程序和平台二进制签名/公证结果、归档路径/权限/依赖完整性。流水线只能验证证据，不能发明证据或自动接受无权限再分发。

## 八、验证要求与验收映射

本节定义开发验收要求，不是测试执行结果，不替代后续 test-planner 的三类测试流程。生产源和测试源码必须分文件；私有测试使用独立 `*_tests.rs`/测试目录，无需扩大 pub。

| 需求 | 独立测试/场景 | 必须观察的结果 |
| --- | --- | --- |
| 原生下载 | downloader 的公开 API 独立测试；200/206/416/ETag变化/无强ETag/截断/重定向/DNS/取消/恢复 | 不依赖 aria2，不错拼；安全重下可进度回退；未知总量不造百分比 |
| 旧记录 | 原数字 ID+旧 GID、aria2 稀疏片段、旧完整 GGUF、坏路径/JSON | ID 不变；无可信连续证据不续传；完整文件重算 SHA 才复用 |
| 内嵌构建 | 四目标选包、缺 feature/资源、错误摘要/target/version、CI 无网络构建 | 正式缺资源失败；每个主程序仅本平台基础包，默认无 CUDA |
| 解压 | zip/tar 穿越、绝对/ADS/设备名、重复/大小写冲突、链接链/循环、炸弹、权限 | 拒绝越界且无外部写入；安全内部链接保留；暂存清理失败可见 |
| 显式运行时/缓存 | 有效/无效显式路径、版本不符、坏缓存、并发复用、安装失败 | 不覆盖用户运行时；共享作业只执行一次；失败不破坏旧可用安装 |
| CUDA | 缺任一包、SHA错、VC++缺、驱动/设备探测错、重定向 GitHub | 明确失败；不静默 CPU；两包都准备成功才 runtime.succeeded |
| 后端/磁盘 | CPU无GPU、CUDA资源、MacMetal/CPU、同文件系统总量、空间未知/ENOSPC | 不清空原配置；CPU不套用独立GPU显存门槛；传输前确定检查 |
| 任务 API | 三新接口有效/非法JSON/重复字段/越限/UUID/其他owner/404/终态取消/存储错 | 精确 HTTP 契约；202 非完成；GET failed 查询仍 success:true |
| 任务竞争 | 同请求并发、auto/cpu实际同键、取消与成功CAS、迟到进度/共享作业 | 在途复用；不重复启动；不取消其他使用者；终态不被覆盖 |
| 进程与路由 | Ok(Unhealthy)、健康超时、路由失败/饱和、精确代次停止、新代次替换、清理失败 | 不假报 succeeded/ready；只清理本任务新建代次；保留两个错误 |
| 恢复 | 双实例锁、共享PG两owner、重启任务、刷新/服务切换/停止后成功快照 | 仅本机未完成任务中断；不自动拉模型进程；实时不可用时 ready=false |
| 客户端 | 准备双进度、过期run_id、刷新GET、取消202、无路径就绪凭据、真实SSE/认证失败 | 不再同步手动部署；不伪造路径/回复/指标；所有目录对话走BurnCloud路由 |
| 文档与更新 | 公有方法中文rustdoc/示例、README/OpenAPI、四目标单文件更新 | 契约一致；许可不丢；执行位与内嵌资源仍在 |

### 8.1 必须分别完成的真实部署验收

1. 每个目标使用无预装 llama.cpp、无受管缓存的干净系统；记录真实 OS/架构、发行 SHA 和资源锁定清单。候选系统不等于已支持，Linux 首期只以 Ubuntu 24.04 x64 验收。
2. 防火墙/代理规则禁止直连 GitHub 与其资源域名，保存网络证据；基础运行时离线准备成功，模型从项目支持源下载固定版本/摘要的轻量 GGUF。
3. 通过新接口及客户端完成真实 SHA、启动、健康、路由，然后用合法 BurnCloud Token 经 BurnCloud 数据面进行一次真实对话。记录请求使用 model_id 和响应，不能访问裸 llama-server 代替。
4. 第二个兼容模型证明运行时缓存复用；覆盖刷新、取消、重试、服务重启、模型/运行包损坏恢复。每项给真实命令/输出，不以 HTTP202 或文件下载成功验收。
5. Windows NVIDIA 环境单独验证双包代理下载、VC++、驱动、cudart、实际设备使用和真实对话；CPU 通过不代表 CUDA 通过，未测加速不能标为可用。
6. 检查完整许可证、Windows 授权、macOS 签名/公证和体积报告；四基础目标与 CUDA 独立记录发布门禁。硬件/签名资源缺失时仅标未验证，不降低标准或伪造通过。

### 8.2 工程与文档门禁

- 按任务实现后以及每次提交前/后执行 `cargo fmt`、`cargo clippy`、`cargo test`，明确所用 workspace/feature/target 与失败原因；Conventional Commits。单纯文档阶段尚未运行这些命令，不宣称测试通过。
- 新 stop_model_generation、既有下载 API 的变更语义及错误变体必须有独立测试和中文 `///`；关键模块提供 `# Examples` 文档测试。修复旧错误追加回归，不将测试与生产源码放同一文件。
- 同步根 README、server/download/node/client 对应 README（有公开变化者）、`server/src/api/openapi.rs`、平台/资源说明及接口类型文档。不得覆盖用户未跟踪的 hardware-model-resolution-api.md。
- 确认迁移两端均登记并真实执行；缺 PostgreSQL 资源时不能只凭 SQLite 通过宣称双端兼容。无 unsafe、外部输入无 unwrap/expect、不吞 Result，不为过检删功能/降校验。

## 九、阶段交付与确认

本设计以固定目标、单一下载实现和现有部署服务为边界，没有设计待选项。真实签名/授权/展开库存/跨平台及 CUDA 实机证据是编码/发行阶段需要取得的事实，不在本设计中冒充完成。

- 已完成：需求登记，已确认流程图，已确认 HTTP 契约，源码落点核对；用户于 2026-11-24 确认开发文档定稿。
- 尚未开始：任务拆分、业务编码、运行包集成、Cargo 检查、四目标/Windows CUDA 实机验收、提交/推送与 PR。
- 下一阶段按既定流程生成原子任务和执行顺序，随后编码；本确认只批准设计，不代表任务、代码或测试完成。不重复要求已经确认的范围/流程图/接口，不创建虚假测试或 PR 留档。

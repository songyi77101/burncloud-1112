# BurnCloud 受管运行时资源（llama.cpp b8581）

[`manifest.json`](manifest.json) 是固定**源包登记**，不是可直接用于发行的锁定清单。它不含归档库存、安装程序摘要、授权或签名结果。不允许通过部署请求、本机 `target`、系统 `PATH` 或 `ready.json` 替换登记值。

| Cargo `TARGET` | 唯一离线基础包 | `auto` 的候选顺序 |
| --- | --- | --- |
| `x86_64-pc-windows-msvc` | Windows x64 CPU ZIP | CPU |
| `aarch64-apple-darwin` | macOS ARM64 原生 tar.gz | Metal（实测可用时）、CPU |
| `x86_64-apple-darwin` | macOS x64 原生 tar.gz | Metal（实测可用时）、CPU |
| `x86_64-unknown-linux-gnu` | Ubuntu 24.04 x64 CPU tar.gz | CPU |

`auto_backends` 只是候选策略，不证明任何设备可用；显式 `cpu` 不使用 GPU。只有 Windows x64 显式选择 `cuda` 才在线获取 CUDA 12.4 服务 ZIP 和 cudart ZIP，地址仅由 `targets.x86_64-pc-windows-msvc.cuda.download_base_url` 与两个固定文件名拼接。客户端不查询 GitHub API、不回退直连 GitHub 或其他镜像，也不因 CUDA 失败降到 CPU。CUDA 两个 ZIP 都不内嵌，且不得改签换包或用不同摘要。cudart ZIP 不提供可启动程序。

## 发行资源准备（任务 006/007 接入）

1. 在显式指定的 CI 资源目录中按目标准备归档。发行环境可以用 `source_base_url` 下载固定源包；先验证长度和 SHA-256，再枚举内容。HTML、缺文件或校验不符都失败。源包和准备后的归档分开存放；不把大文件、源包缓存或生成的发行锁提交到仓库，不读取或覆盖用户 `target`。
2. 在目标系统基线上查出实际动态依赖，记录由基线、同包或受管依赖满足的来源。Ubuntu 额外的合法再分发 `.so` 并入同一基础包并审核 `$ORIGIN` 等相对加载路径，不替换系统 glibc。macOS 如需修正相对加载路径，先修正，再对 Mach-O/dylib 由内到外签名并验证签名、公证；不在用户端重签或禁用 Gatekeeper。Windows 必须取得独立的 `vc_redist.x64.exe`，锁定实际字节、微软 Authenticode 签名及再分发授权；提取给用户确认安装，不由 BurnCloud 静默执行。
3. 核对归档及附带内容的全部再分发权。`license_components` 是源包至少应覆盖的审计项，不是授权结论或完整的传递依赖清单。凡归档中缺少的必要 LICENSE / NOTICE，须以独立、经核验的资源提供并随发行物保留。Windows 的 CPU ZIP 与 CUDA 服务 ZIP 均含 `libomp140.x86_64.dll`，不能拿 llama.cpp 的许可证或 VC++ 安装程序授权代替对该 DLL 的审核。依赖修正引入的新内容同样要有证据。
4. 对发行实际使用的归档重新散列、测量压缩长度并生成库存。原样使用时源 SHA 与内嵌 SHA 相同；派生归档不得使用源摘要或源库存冒充。将最终资源及 `manifest.lock.json` 交给 `BURNCLOUD_RUNTIME_ASSET_DIR`。启用 `bundled-runtime` 的构建只读本地资源，内嵌一个基础归档和本目标所需依赖/许可；不在构建期下载，也不接受没有锁定证据的资源。

## 发行锁定清单 v1（每个目标一份）

锁文件由发行准备工具生成，不提供可被误用的示例库存、空摘要或「已通过」占位记录。所有 `sha256` 值为实际内容的 64 个小写十六进制字符；所有字节数为非负整数。文件名必须等于登记名；派生包可保留相同 basename，但不得覆盖源包。

| `manifest.lock.json` 根字段 | 内容 |
| --- | --- |
| `schema_version`, `version`, `target` | 格式版本 `1`、固定 `b8581`、本次构建的精确 Cargo `TARGET`。 |
| `base` | 唯一基础归档的锁定项，字段见下。 |
| `dependencies` | 键为 `required_dependencies` 中的文件名；条目包含实际文件记录、`signature_evidence` 和 `licenses`。Windows 必须有 VC++ 安装程序项，其他三个目标此字段为空对象。 |
| `cuda` | 仅 Windows 的独立 CUDA 发布证据可含此项。包含固定 `server` 和 `cudart` 的锁定项及 `approved_collisions`；缺项不是已认证 CUDA。普通发行不内嵌 CUDA ZIP。 |

| 归档锁定项字段 | 内容 |
| --- | --- |
| `filename`, `package_kind`, `source_size_bytes`, `source_sha256` | 对应登记项，绑定上游源身份。 |
| `embedded_size_bytes`, `embedded_sha256` | `base` 必填，指实际内嵌归档。CUDA 为在线固定资源，只使用 `source_size_bytes` / `source_sha256`，不写 `embedded_*`。 |
| `unpacked_bytes`, `server_relative_path`, `inventory` | 受审展开字节上限、唯一固定服务端相对路径（cudart 不写）、最终归档完整库存。展开上限同时受 manifest 的全局资源限制约束。 |
| `dependency_audit` | 目标基线上的动态依赖审计文件记录，列出依赖库来源、相对加载规则和核验结果；基础包和 CUDA 组合分别核对。 |
| `licenses` | 逐组件记录：`component`、覆盖的归档相对路径 `paths`、随发行保留的 `notices` 文件记录；有独立再分发条件时，还必须提供对应的 `redistribution_evidence` 文件记录。VC++ 安装程序必须提供该授权证据。 |
| `signature_evidence`, `notarization_evidence` | macOS 基础包必须有绑定最终归档的签名验证和公证文件记录；Windows VC++ 项必须有绑定实际安装程序的签名验证记录。仅填名称、版本或布尔值不算证据。 |

文件记录形状为 `path`, `size_bytes`, `sha256`。`path` 是相对于本次 CI 资源根的受限路径；逐项核文件实际长度和摘要。包含许可证据的文件记录不能代替对许可证适用范围和签名结果的检查。

库存按规范化相对路径排序、去重。`inventory` 的每项包含 `path`, `kind`, `mode`；摘要和大小只用于 `file`，目标路径只用于链接：

| `kind` | 必填附加字段 | 含义 |
| --- | --- | --- |
| `file` | `size_bytes`, `sha256` | 最终安装的普通文件字节。 |
| `directory` | 无 | 必要目录，包括安装后需核对权限的隐式目录。 |
| `symlink` | `target` | 相对于链接所在目录的目标，解析后必须在同一安装根内。 |
| `hardlink` | `target` | 相对于安装根的已审核普通文件路径。 |

`mode` 是审核后允许的 Unix 权限位整数，不保留 suid/sgid 或多余写权限；Windows 条目和不单独审计权限的符号链接记录 `null`。路径使用 `/`，拒绝绝对路径、越界、重名和大小写冲突；链接不能成环，也不能经由链接写入条目。CUDA 合并前两个包各自必须通过压缩摘要和库存核验；同路径同内容可复用。同路径不同内容默认拒绝，`approved_collisions` 只能列出经审查的冲突，每项包含 `path`, `server_sha256`, `cudart_sha256`, `selected`（`server` 或 `cudart`），且散列须与两份库存对应。未看到冲突时不补白名单条目。

## 放行与信任边界

缺真实字节、完整库存、依赖核验、适用授权或签名/公证时，阻断相应目标或 CUDA 的发布，不填占位或提供跳过开关。缓存重用以编入的归档 SHA 和经审核的只读库存为依据，重新校验文件、路径、链接和权限；`ready.json` 只记状态。归档 SHA、锁文件和构建成功都不等于干净系统上的真实启动、设备探测或 BurnCloud 数据面回答；这些证据需独立验收。

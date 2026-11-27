# Gitee 运行期获取 llama.cpp 开发文档

> 开发序号：5；标题短名：gitee-runtime-download；日期：2026-11-26。
> 设计来源：用户已确认的方案二技术书、`gitee-runtime-download-flowchart.md` 与 `gitee-runtime-download-interface-contract.md`。
> 设计起点：基于现有实现继续开发。本文只定义运行时来源和准备链路的增量，不重写旧自动部署文档中已经有效的模型、任务、数据库、进程与路由设计。
> 当前状态：技术设计已定稿；代码、测试、真实部署和发布门禁尚未因本文自动完成。

## 一、目标与范围

### 1.1 目标

为当前 BurnCloud 本机模型自动部署增加一个受管 `llama.cpp` 运行时准备器，使用户首次部署或本地缓存缺失时能够：

1. 根据服务端固定 manifest 选择当前 target/backend 的 Gitee 归档；
2. 使用原生 `reqwest` 流式下载，支持安全断点恢复、取消和有限重试；
3. 按大小和 SHA-256 校验归档，避免错误实体进入缓存；
4. 在用户数据目录的 staging 中安全解压 ZIP/TAR.GZ；
5. 定位并探测固定的 `llama-server`，确认依赖和 macOS Metal 能力；
6. 原子提交可共享缓存，并把已验证的可执行路径交给现有 `DeploymentService`；
7. 通过既有自动部署 POST/GET/DELETE 接口展示 `preparing_runtime` 和 `RuntimeProgress`，不新增 runtime HTTP API。

### 1.2 本期边界

- 来源固定：`https://gitee.com/songyi777/llama-cpp`，commit `51d01f6e526bc6d45b9d252b1d2d487efb6625d8`。
- 受管版本：`b12757`，以四个固定文件名、大小、SHA-256 和探测结果绑定。
- 支持 target：`x86_64-pc-windows-msvc`、`x86_64-apple-darwin`、`aarch64-apple-darwin`、`x86_64-unknown-linux-gnu`。
- backend：Windows/Linux `cpu`；macOS `cpu`/`metal`；`cuda` 当前明确返回 `unsupported_backend`。
- 来源优先级：`BURNCLOUD_LLAMA_SERVER` → 有效用户缓存 → Gitee 固定 commit 下载。
- 不以内嵌运行时为默认回退；旧 b8581 内嵌方案只作为历史设计保留。
- 不改变模型 GGUF 下载、摘要、模型路由和 `DeploymentService` 的进程生命周期。

### 1.3 非目标

- 不追踪 Gitee `master`，不在生产部署时动态调用 Gitee API 发现版本；
- 不使用 `git clone` 或 `contents` Base64 传输大文件；
- 不启动或依赖 aria2；
- 不实现 Gitee CUDA 包、Windows CUDA 双包下载或 Linux CUDA/ARM64；
- 不安装系统驱动、Toolkit、VC++ 或修改系统 PATH；
- 不把 `xcframework` 用作桌面 `llama-server`；
- 不创建独立 runtime 管理 HTTP API；
- 不把本地绝对路径、URL、版本、commit、SHA 或解压目录交给客户端控制。

## 二、现有实现复用与模块职责

### 2.1 已有实现复用

| 现有位置 | 继续复用的职责 | 方案二改动 |
| --- | --- | --- |
| `crates/interfaces/server/src/node_runtime.rs` | `LlamaCppRuntimePreparer`、运行时适配器和 `PreparedRuntime` 接入点 | 将默认运行时解析改为 manifest/cache/Gitee 顺序；不把下载逻辑放入进程生命周期 |
| `crates/interfaces/server/src/node_deployment.rs` | 创建运行时请求、等待准备器、提交部署任务和错误映射 | 传递固定 target/backend 与运行时进度；不新增客户端可控 runtime 字段 |
| `crates/platform/node/src/deployment/` | 进程创建、端口、readiness、health、`/v1/models`、`/slots`、路由和停止 | 不重新实现进程管理；只消费 `PreparedRuntime` 的可执行路径 |
| `crates/platform/storage/download/` | 用户数据根和下载错误/进度模型可参考 | 新增独立运行时 HTTP 传输模块，不复用 aria2、模型目录和模型任务身份 |
| `deploy/runtime/manifest.json` | 既有运行时清单位置和资源限制表达 | 由任务 026 扩展为 Gitee 固定 commit、四目标资源登记 |
| `deploy/runtime/README.md` | 运行时资源、许可证和发行说明 | 由任务 035 更新为运行期下载、缓存和来源优先级 |

### 2.2 推荐增量文件边界

编码时优先采用以下边界；若现有模块组织需要调整，必须保持职责和公开范围不变：

```text
crates/platform/storage/download/
├── src/runtime_http.rs              # 原生 reqwest 运行时归档传输，私有模块
└── src/runtime_http/                # 如实现需要拆分

crates/interfaces/server/src/
├── node_runtime.rs                  # 现有接入点，保留外部 trait 适配
└── node_runtime/
    ├── manifest.rs                  # manifest 解析、target/backend 映射
    ├── cache.rs                     # 用户缓存、锁、原子提交
    ├── archive.rs                   # ZIP/TAR.GZ 安全解压
    ├── probe.rs                     # 版本/依赖/设备探测
    ├── progress.rs                  # RuntimeProgress 映射
    └── tests/                       # 独立测试文件
```

实际是否拆目录由实现阶段决定；不能因为拆分而新增客户端可调用的公共接口。

### 2.3 来源和路径配置

- `BURNCLOUD_LLAMA_SERVER`：管理员级显式 `llama-server` 路径。必须是绝对路径；存在但无效时返回 `runtime_configuration_invalid`，不继续回退。
- `BURNCLOUD_RUNTIME_CACHE_DIR`：管理员级缓存根覆盖。只影响缓存根，不改变 manifest、URL、摘要、版本或解压路径；未设置时使用当前平台用户数据根。
- 缓存根必须经过路径规范化，不能来自 HTTP 请求；安装目录、项目目录和 `target` 不得作为默认缓存位置。

## 三、可信 manifest 设计

### 3.1 资源登记

manifest 的服务端配置至少表达：

```json
{
  "schema_version": 2,
  "provider": "gitee",
  "repository": "songyi777/llama-cpp",
  "commit": "51d01f6e526bc6d45b9d252b1d2d487efb6625d8",
  "version": "b12757",
  "archive_limits": {
    "max_path_bytes": 4096,
    "max_entries": 100000,
    "max_file_bytes": 2147483648,
    "max_unpacked_bytes": 4294967296
  },
  "targets": {
    "x86_64-pc-windows-msvc": {
      "supported_backends": ["cpu"],
      "package": {
        "package_kind": "cpu",
        "filename": "llama-b12757-bin-win-cpu-x64.zip",
        "archive_format": "zip",
        "size_bytes": 25262139,
        "sha256": "5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d",
        "server_relative_path": "llama-server.exe"
      }
    }
  }
}
```

完整四目标登记值以 `gitee-runtime-download-interface-contract.md` 第二节为准。示例只展示 schema 形状，不允许把示例包扩展为新的目标或把客户端提交值合并进 manifest。

### 3.2 可信校验

`validate_manifest` 必须在应用启动读取和每次准备选择时执行以下校验：

- provider/repository/commit/version 与固定策略一致；
- target 是当前编译 target，且存在对应包；
- 请求 backend 在 `supported_backends` 内；
- 文件名为单一基名，不能包含路径分隔符、空字节或控制字符；
- archive format 与 target/扩展名匹配；
- size 非零且不超过下载预算上限；
- sha256 是 64 位小写十六进制；
- server_relative_path 为安全相对路径，不含 `..`、绝对前缀或驱动器前缀；
- limits 为正数且不超过服务端硬上限；
- `xcframework` 不出现在 `llama-server` target 映射中。

manifest 校验失败是服务端配置错误，必须在任务中表现为 `runtime_manifest_invalid`，不能选择另一个 target、另一个 backend 或旧版本掩盖错误。

## 四、数据结构

========================================

- 名称：`RuntimeManifest`
- 类型：代码结构
- 说明：服务端可信运行时清单的内存表示；不从客户端 JSON 反序列化合并字段。
- 结构：

```python
class RuntimeManifest:
    schema_version: uint32                 # manifest schema 版本
    provider: string                        # 固定 gitee
    repository: string                      # 固定 songyi777/llama-cpp
    commit: string                          # 固定 40 位 commit
    version: string                         # 当前 b12757
    archive_limits: RuntimeArchiveLimits   # 归档展开限制
    targets: Map[Target, RuntimeTarget]     # 四目标资源表
```

========================================

- 名称：`RuntimeTarget`
- 类型：代码结构
- 说明：单一 Cargo target 的运行时包和有效 backend 映射。
- 结构：

```python
class RuntimeTarget:
    target: string                          # Cargo target triple
    supported_backends: List[Backend]       # cpu 或 cpu/metal
    package: RuntimePackage                 # 当前唯一受管归档
```

========================================

- 名称：`RuntimePackage`
- 类型：代码结构
- 说明：固定 Gitee 归档的可信身份。
- 结构：

```python
class RuntimePackage:
    package_kind: string                    # cpu 或 native
    filename: string                        # 归档基名
    archive_format: string                  # zip 或 tar.gz
    size_bytes: uint64                      # 归档实际字节数
    sha256: string                          # 64 位小写摘要
    server_relative_path: string            # 解压根下的服务端相对路径
    dependencies: List[string]              # 可选依赖/许可证标识
    license_components: List[string]        # 可选许可证审计标识
```

========================================

- 名称：`RuntimeArchiveLimits`
- 类型：代码结构
- 说明：拒绝路径穿越和归档炸弹的上限集合。
- 结构：

```python
class RuntimeArchiveLimits:
    max_path_bytes: uint64                  # 单条 entry 路径最大字节数
    max_entries: uint64                      # entry 总数上限
    max_file_bytes: uint64                   # 单文件展开上限
    max_unpacked_bytes: uint64               # 总展开字节上限
```

========================================

- 名称：`RuntimeCacheKey`
- 类型：代码结构
- 说明：运行时共享和目录隔离的唯一身份。
- 结构：

```python
class RuntimeCacheKey:
    version: string                          # 受管版本
    target: string                           # Cargo target
    package_kind: string                     # 包类型
    archive_sha256: string                   # 归档摘要
```

========================================

- 名称：`RuntimePartialMetadata`
- 类型：代码结构
- 说明：`.part` 文件的可恢复断点侧车；不替代最终完整 SHA-256。
- 结构：

```python
class RuntimePartialMetadata:
    schema_version: uint32                  # sidecar schema
    url: string                              # 固定 commit raw URL
    provider: string                         # 固定 gitee
    repository: string                       # 固定仓库
    commit: string                           # 固定 commit
    target: string                           # 目标 triple
    filename: string                         # 归档基名
    expected_size: uint64                    # manifest 登记大小
    expected_sha256: string                  # manifest 登记摘要
    strong_etag: string | None               # 仅允许强 ETag
    total_bytes: uint64 | None               # 远端确认的总长度
    committed_bytes: uint64                  # 本地连续已同步长度
    updated_at: string                       # RFC 3339 UTC
```

========================================

- 名称：`RuntimeProgress`
- 类型：代码结构
- 说明：父自动部署任务中可查询的运行时子进度；字段必须与接口契约一致。
- 结构：

```python
class RuntimeProgress:
    status: TaskStatus                        # queued/running/succeeded/failed/cancelled/interrupted
    stage: DeploymentStage | None             # 运行时仅为 preparing_runtime
    phase: RuntimePreparationPhase | None     # 详细运行时 phase
    detail: string | None                     # 展示文本
    task_id: string | None                    # 固定为空，不暴露共享作业 API
    completed_bytes: uint64                   # 真实连续下载字节
    total_bytes: uint64 | None                # manifest 大小或未知
    download_speed: uint64                    # 非下载阶段为 0
    actual_sha256: string | None              # 完整实际摘要
    origin: RuntimeOrigin | None              # explicit/cache/downloaded
    version: string | None                    # 受管版本或显式探测版本
    target: string | None                     # 实际 target
    backend: string | None                    # 实际 backend
    package_kind: string | None               # cpu/native
    archive_sha256: string | None             # 期望摘要展示
    error: TaskError | None                   # 运行时错误
```

========================================

- 名称：`PreparedRuntime`
- 类型：代码结构
- 说明：通过所有校验、解压和探测后交给现有部署服务的只读运行时凭据。
- 结构：

```python
class PreparedRuntime:
    executable: AbsolutePath                  # 服务端内部使用，不直接返回 HTTP
    target: string                             # 已验证目标
    backend: string                            # 实际 backend
    version: string                            # b12757 或显式版本
    origin: RuntimeOrigin                      # explicit/cache/downloaded
    package_kind: string | None                # 受管包类型
    archive_sha256: string | None              # 受管归档摘要
    runtime_root: AbsolutePath                 # 用户缓存目录或显式程序父目录
    probe_summary: RuntimeProbeSummary         # 版本、依赖、设备探测结果
}
```

========================================

- 名称：`RuntimeProbeSummary`
- 类型：代码结构
- 说明：运行时探测的受控结果，供日志、缓存元数据和部署诊断使用。
- 结构：

```python
class RuntimeProbeSummary:
    version_output: string                     # 受限长度的 --version 输出摘要
    help_probe_passed: bool                    # --help/身份探测是否通过
    dependencies_loaded: bool                  # 动态依赖是否成功加载
    metal_device_count: uint32 | None         # macOS Metal 实测设备数
    effective_backend: string                  # cpu 或 metal
    probed_at: string                          # RFC 3339 UTC
}
```

========================================

## 五、函数定义

> 下列函数是实现边界和调用关系的设计名。除现有公开 trait 方法外，默认全部私有；不因测试而扩大 crate 或模块可见性。函数签名以 Rust 实现时的错误类型和现有 trait 为准，但不得改变本契约的输入、输出和错误语义。

========================================

- 函数名称：`validate_manifest`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`crates/interfaces/server/src/node_runtime/manifest.rs`
- 职责说明：校验服务端 manifest 的固定 provider、仓库、commit、target、backend、文件名、大小、摘要、路径和资源限制。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `manifest` | `&RuntimeManifest` | 是 | 待校验清单 |
| `current_target` | `&str` | 是 | 当前编译/运行 target |
| `requested_backend` | `Backend` | 是 | preflight 已解析的 backend |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `target_entry` | `&RuntimeTarget` | 通过校验的目标登记 |

- 错误返回：

| 错误类型 | 错误码 | 错误场景 |
| --- | --- | --- |
| `RuntimePrepareError` | `runtime_manifest_invalid` | 固定身份、字段格式、大小、摘要或路径不合法 |
| `RuntimePrepareError` | `unsupported_platform` | 当前 target 不在 manifest |
| `RuntimePrepareError` | `unsupported_backend` | backend 不在目标支持列表 |

- 校验规则：白盒断言
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用（内部函数） | `validate_sha256`、`validate_relative_path`、`validate_archive_limits` |
| 被调用（调用方） | `resolve_runtime_entry` |

========================================

- 函数名称：`resolve_runtime_entry`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`node_runtime::manifest`
- 职责说明：把部署请求中的 target/backend 解析为唯一可信运行时包；不接受客户端版本、URL、摘要或路径。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `request` | `&RuntimeRequest` | 是 | 内部运行时请求 |
| `manifest` | `&RuntimeManifest` | 是 | 服务端清单 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `entry` | `ResolvedRuntimeEntry` | 含固定 URL、缓存 key、包和有效 backend |

- 错误返回：`unsupported_platform`、`unsupported_backend`、`runtime_manifest_invalid`。
- 校验规则：白盒断言
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用（内部函数） | `validate_manifest`、`build_gitee_raw_url`、`build_cache_key` |
| 被调用（调用方） | `LlamaCppRuntimePreparer::prepare` |

========================================

- 函数名称：`build_gitee_raw_url`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`node_runtime::manifest`
- 职责说明：用服务端固定仓库、commit 和 manifest 文件名构造 raw URL，并拒绝任何外部覆盖。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `manifest` | `&RuntimeManifest` | 是 | 固定来源配置 |
| `package` | `&RuntimePackage` | 是 | 已通过字段校验的包 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `url` | `Url` | 固定 commit raw URL |

- 错误返回：`runtime_manifest_invalid`。
- 校验规则：白盒断言
- 调用关系：内部调用无；被 `resolve_runtime_entry` 调用。

========================================

- 函数名称：`read_runtime_cache_root`
- 公开属性：私有函数
- 多线程属性：普通
- 函数归属：独立函数
- 所属模块或包：`node_runtime::cache`
- 职责说明：解析管理员级缓存根覆盖或平台默认用户数据根，并拒绝项目目录、安装目录和 `target` 作为默认运行时目录。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `installation_root` | `&Path` | 是 | 当前应用安装/项目根，用于排除错误位置 |
| `target_dir` | `&Path` | 是 | 当前构建 target 目录，用于排除错误位置 |
| `override_value` | `Option<&str>` | 否 | `BURNCLOUD_RUNTIME_CACHE_DIR` 的值 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `root` | `PathBuf` | 可信用户数据缓存根 |

- 错误返回：`runtime_configuration_invalid`、`runtime_cache_invalid`。
- 校验规则：白盒断言
- 调用关系：被 `LlamaCppRuntimePreparer::prepare` 调用。

========================================

- 函数名称：`check_cached_runtime`
- 公开属性：私有函数
- 多线程属性：并发
- 函数归属：独立函数
- 所属模块或包：`node_runtime::cache`
- 职责说明：按 `RuntimeCacheKey` 查找缓存，检查 ready、runtime.json、路径边界、摘要和再次探测；无效缓存只隔离本 key，不删除其他版本。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `cache_root` | `&Path` | 是 | 用户数据缓存根 |
| `key` | `&RuntimeCacheKey` | 是 | 版本、target、包类型和摘要 |
| `request` | `&RuntimeRequest` | 是 | backend 和取消上下文 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `runtime` | `Option<PreparedRuntime>` | 有效命中返回运行时，否则为 None |

- 错误返回：`runtime_cache_invalid`、`runtime_probe_failed`、I/O 错误映射为运行时准备错误。
- 校验规则：白盒断言
- 调用关系：调用 `read_runtime_metadata`、`verify_cached_archive`、`probe_runtime`；被 `prepare` 调用。

========================================

- 函数名称：`acquire_runtime_install_lock`
- 公开属性：私有函数
- 多线程属性：并发
- 函数归属：独立函数
- 所属模块或包：`node_runtime::cache`
- 职责说明：确保同一 `RuntimeCacheKey` 在跨线程和跨进程范围内只有一个安装作业写入。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `cache_root` | `&Path` | 是 | 缓存根 |
| `key` | `&RuntimeCacheKey` | 是 | 共享身份 |
| `cancellation` | `&CancellationToken` | 是 | 等待锁时可取消 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `guard` | `RuntimeInstallLock` | 持锁句柄，生命周期覆盖整个安装/提交过程 |

- 错误返回：`runtime_install_conflict`、`cancelled`、`interrupted`。
- 校验规则：白盒断言
- 调用关系：调用内部锁文件/平台锁实现；被 `prepare` 调用。

========================================

- 函数名称：`download_runtime_archive`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`crates/platform/storage/download/src/runtime_http.rs`
- 职责说明：使用原生 reqwest 流式获取固定 Gitee 归档，维护 `.part` 和 sidecar，支持安全 Range 续传、取消、有限重试和真实字节进度。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `entry` | `&ResolvedRuntimeEntry` | 是 | 固定 URL、大小和摘要 |
| `download_dir` | `&Path` | 是 | `runtimes/downloads/<version>/<target>` |
| `progress` | `&RuntimeProgressSink` | 是 | 内部进度发布器 |
| `cancellation` | `&CancellationToken` | 是 | 取消信号 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `archive` | `VerifiedArchive` | 已完成流式写入且大小、SHA-256 通过的归档 |

- 错误返回：`runtime_source_unavailable`、`runtime_download_failed`、`runtime_size_mismatch`、`runtime_checksum_mismatch`、`cancelled`。
- 校验规则：白盒断言
- 调用关系：调用 `load_partial_metadata`、`request_range_or_full`、`append_stream`、`verify_archive_digest`；被 `prepare` 调用。

========================================

- 函数名称：`resume_or_restart_download`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`runtime_http`
- 职责说明：根据 `.part`、sidecar、强 ETag、Range 起点和 Content-Range 决定续传或安全重下。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `partial` | `&PartialDownload` | 是 | 已存在的 `.part` 和 sidecar |
| `entry` | `&ResolvedRuntimeEntry` | 是 | 固定实体身份 |
| `client` | `&reqwest::Client` | 是 | 原生 HTTP 客户端 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `download_mode` | `ResumeMode` | `Append` 或 `Restart` |

- 错误返回：`runtime_download_failed`；协议不一致时不追加，返回可重试的 restart 结果。
- 校验规则：白盒断言
- 调用关系：被 `download_runtime_archive` 调用。

========================================

- 函数名称：`extract_runtime_archive`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`node_runtime::archive`
- 职责说明：将 ZIP/TAR.GZ 解压到同文件系统 staging，并拒绝路径穿越、绝对路径、链接 entry 和资源限制超限。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `archive` | `&VerifiedArchive` | 是 | 已通过大小和摘要校验的归档 |
| `staging_root` | `&Path` | 是 | 本次安装的 staging 目录 |
| `limits` | `&RuntimeArchiveLimits` | 是 | manifest 登记限制 |
| `cancellation` | `&CancellationToken` | 是 | 解压期间取消 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `extracted_root` | `PathBuf` | 安全展开目录 |

- 错误返回：`runtime_archive_invalid`、`cancelled`、I/O 错误映射为运行时准备错误。
- 校验规则：白盒断言
- 调用关系：调用 `validate_archive_entry_path`、`validate_entry_kind`、`write_entry_with_limits`；被 `prepare` 调用。

========================================

- 函数名称：`probe_runtime`
- 公开属性：私有函数
- 多线程属性：异步
- 函数归属：独立函数
- 所属模块或包：`node_runtime::probe`
- 职责说明：定位 manifest 指定的 `server_relative_path`，执行版本/帮助探测、依赖加载探测，并按 target/backend 实测设备能力。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `extracted_root` | `&Path` | 是 | 安全 staging 根 |
| `entry` | `&ResolvedRuntimeEntry` | 是 | server path、target、backend |
| `requested_backend` | `Backend` | 是 | 用户选择或 auto 解析结果 |
| `cancellation` | `&CancellationToken` | 是 | 进程探测取消 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `prepared` | `PreparedRuntimeCandidate` | 已定位可执行文件和探测摘要 |

- 错误返回：`runtime_archive_invalid`、`runtime_probe_failed`、`runtime_device_unavailable`、`cancelled`。
- 校验规则：白盒断言
- 调用关系：调用 `locate_server`、`run_version_probe`、`run_help_probe`、`probe_metal_devices`；被 `prepare` 调用。

========================================

- 函数名称：`commit_runtime_install`
- 公开属性：私有函数
- 多线程属性：并发
- 函数归属：独立函数
- 所属模块或包：`node_runtime::cache`
- 职责说明：在 staging 中写入 `runtime.json`，最后写入 ready，并以同文件系统原子重命名提交 `version/target/package_kind/sha256` 目录。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `candidate` | `PreparedRuntimeCandidate` | 是 | 已通过探测的暂存运行时 |
| `cache_root` | `&Path` | 是 | 用户缓存根 |
| `key` | `&RuntimeCacheKey` | 是 | 目标缓存身份 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `runtime` | `PreparedRuntime` | 原子提交后的只读运行时 |

- 错误返回：`runtime_install_conflict`、`runtime_cache_invalid`、I/O 错误映射为运行时准备错误。
- 校验规则：白盒断言
- 调用关系：调用 `write_runtime_metadata`、`write_ready_marker`、`atomic_rename_install`；被 `prepare` 调用。

========================================

- 函数名称：`LlamaCppRuntimePreparer::prepare`
- 公开属性：公有接口（现有 `RuntimePreparer` trait 方法）
- 多线程属性：异步、并发安全
- 函数归属：`LlamaCppRuntimePreparer` 的 trait 实现
- 所属模块或包：`crates/interfaces/server/src/node_runtime.rs`
- 职责说明：按照显式配置、缓存、Gitee 下载的优先级返回已验证 `PreparedRuntime`；不管理 llama-server 进程生命周期。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `request` | `RuntimeRequest` | 是 | 由服务端从部署任务生成的 target/backend 请求 |
| `progress` | `RuntimeProgressSink` | 是 | 父任务运行时进度发布器 |
| `cancellation` | `CancellationToken` | 是 | 父任务取消/服务停止信号 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `runtime` | `PreparedRuntime` | 已验证的 `llama-server` 路径和运行时元数据 |

- 错误返回：本文件定义的全部 `preflight/preparing_runtime` 错误；不得把错误转换成 CPU 成功。
- 校验规则：黑盒校验；对外错误不暴露完整路径和堆栈
- 调用关系：

| 方向 | 函数 |
| --- | --- |
| 调用（内部函数） | `resolve_runtime_entry`、`read_runtime_cache_root`、`check_cached_runtime`、`acquire_runtime_install_lock`、`download_runtime_archive`、`extract_runtime_archive`、`probe_runtime`、`commit_runtime_install` |
| 被调用（调用方） | 现有 `node_deployment` 任务执行器；随后交给 `DeploymentService::deploy_prepared` |

========================================

- 函数名称：`publish_runtime_progress`
- 公开属性：私有函数
- 多线程属性：并发
- 函数归属：独立函数
- 所属模块或包：`node_runtime::progress`
- 职责说明：将共享运行时作业的阶段、真实字节、来源和错误映射为父任务 `RuntimeProgress`，防止迟到事件覆盖终态快照。
- 入参列表：

| 名称 | 类型 | 是否必填 | 含义 |
| --- | --- | --- | --- |
| `task_id` | `&TaskId` | 是 | 父部署任务 ID |
| `event` | `RuntimeProgressEvent` | 是 | 共享作业事件 |
| `store` | `&DeploymentStore` | 是 | 现有任务仓储 |

- 返回值列表：

| 名称 | 类型 | 含义 |
| --- | --- | --- |
| `snapshot` | `RuntimeProgress` | 可持久化的运行时子快照 |

- 错误返回：`deployment_storage_failed`；迟到事件不得覆盖已经提交的父任务终态。
- 校验规则：白盒断言
- 调用关系：被下载器、解压器、探测器和部署任务执行器调用。

## 六、运行时准备主流程

### 6.1 preflight

1. 从现有部署任务的 platform/backend 解析 `RuntimeRequest`；客户端没有 runtime 字段。
2. 读取固定 manifest，验证 provider、repository、commit、target、backend 和摘要格式。
3. `cuda` 在当前四目标映射中直接返回 `unsupported_backend`；不进入下载。
4. 计算用户数据根、运行时下载目录、最终缓存 key 和本文件系统的模型/运行时合并预算。
5. 发布 `phase=cache_checking`，父 stage 保持 `preparing_runtime`。

### 6.2 显式路径

1. 读取 `BURNCLOUD_LLAMA_SERVER`；存在时要求绝对路径且文件为普通文件。
2. 执行 `--version`/`--help`，确认它是可执行的 llama-server，加载依赖成功。
3. 按请求 backend 探测：CPU 只需要可执行和依赖通过；显式 Metal 需要实际设备能力。
4. 成功后 `origin=explicit`，不创建受管缓存，不覆盖用户文件；失败返回 `runtime_configuration_invalid` 或 `runtime_device_unavailable`，不回退。

### 6.3 缓存路径

缓存目录固定为：

```text
<user-data-root>/runtimes/
├── downloads/
│   └── <version>/<target>/<filename>.part
│       └── <filename>.part.json
├── <version>/
│   └── <target>/<package_kind>/<sha256>/
│       ├── archive/<filename>
│       ├── extracted/<归档展开内容>
│       ├── runtime.json
│       └── ready
└── .staging/<deployment_task_id>/
```

`BURNCLOUD_RUNTIME_CACHE_DIR` 只替换 `<user-data-root>/runtimes` 的根，不允许请求体指定任意目录。`<deployment_task_id>` 是服务端为本次共享安装作业保留的内部 staging 标识，不是可由客户端操作的 runtime HTTP 资源。最终目录的摘要段必须等于 manifest SHA-256；版本、target、package_kind 和摘要任何一项不匹配都不是缓存命中。

### 6.4 Gitee 下载

- URL 由固定仓库、commit 和登记文件名拼成；reqwest 使用流式 response body，禁止 `bytes()`/整体内存缓存归档。
- `.part` 和 sidecar 先写临时文件、flush/sync 后更新连续长度；sidecar 只能描述已确认的连续区间。
- 续传请求必须同时满足强 ETag、`Range` 起点、`Content-Range` 起点、远端总长度和本地长度一致；否则清空 `.part` 后完整 GET。
- 服务器对 Range 返回 200 时重新开始；返回 206 但范围不匹配、ETag 改变、ETag 缺失或 416 时不追加旧实体。
- 连接超时、无进展超时、5xx 允许有限重试；403/404 固定来源错误不切换来源。默认连接超时 15 秒、无进展超时 60 秒、最多 3 次重试，退避 1/2/4 秒。
- 下载完成后重新计算大小和 SHA-256；只有两者通过才把归档转为已验证输入。失败不能覆盖已有有效 cache key。

### 6.5 安全解压

- ZIP 只在 Windows target 使用；TAR.GZ 用于 macOS/Linux target；格式不匹配为 `runtime_archive_invalid`。
- 每个 entry 先做 UTF-8/平台路径规范化，拒绝空字节、绝对路径、驱动器/UNC 前缀和 `..` 段。
- v1 直接拒绝符号链接、硬链接及其他特殊文件 entry，避免链接链、循环和跨平台权限差异；普通目录和普通文件才可展开。
- 逐 entry 累计计数和字节；超过 max entries、max path、max file 或 max unpacked 立即终止并删除 staging。
- 只允许写入 staging 根；使用规范化后的目标路径再次确认前缀仍在 staging 内。
- 解压后必须按 manifest 的 `server_relative_path` 精确定位普通文件，不能在归档中模糊搜索。

### 6.6 探测和提交

1. 在 staging 中执行 `llama-server --version`；读取输出时限制长度，避免把子进程无限输出写入日志或内存。
2. 执行 `--help` 或等价的身份探测，确认程序可以加载动态库并正常退出。
3. Windows/Linux CPU 记录 `effective_backend=cpu`。
4. macOS 对 Metal 执行实际设备探测；`backend=metal` 无设备时返回 `runtime_device_unavailable`；`backend=auto` 可按既有策略选择 Metal 或 CPU，但实际结果必须记录。
5. 写入 `runtime.json`：manifest identity、实际 archive 摘要、server relative path、探测摘要、创建时间、有效 backend 和许可证/依赖引用。
6. `ready` 只在元数据成功后最后写入；然后在同一文件系统内原子重命名 staging 到最终 key 目录。若已有相同 key 有效目录，保留旧目录并返回它。
7. 返回 `PreparedRuntime`，随后由现有 `DeploymentService` 负责启动参数、随机端口、健康检查和路由。

## 七、并发、取消和恢复

### 7.1 共享作业

- 进程内以 `RuntimeCacheKey` 去重，共享 `RuntimeInstallJob` 和进度广播；每个父任务持有一个订阅引用。
- 跨进程以缓存根下的 key 锁互斥写入；第二个进程等待 ready 或读取已完成元数据，不重复解压和提交。
- 共享作业不对外生成独立 HTTP task ID；父任务的 `runtime.task_id` 固定为空。
- 共享作业持有者必须在下载、解压、探测和提交期间保持锁；不能只锁下载而允许两个进程同时写同一 staging/最终目录。

### 7.2 取消

- 父任务 DELETE 先 CAS 记录 `cancel_requested`，再由执行器解除当前订阅。
- 若共享作业仍有订阅者，下载/解压继续；取消的父任务将未完成运行时子分支标为 `cancelled`，不得删除共享缓存。
- 若没有订阅者，可以取消网络/解压 token，但保留已确认 `.part`；不删除已有有效 cache key。
- 运行时已经 `ready` 后，取消不回滚或删除共享缓存；父任务是否可取消仍由旧父任务状态规则决定。

### 7.3 服务重启

- 启动恢复仅把本机 queued/running 父任务标为 `interrupted`，不自动拉起 `llama-server`。
- 未完成的 runtime job 不标记 ready；新任务重新取得锁，验证 `.part`/staging 身份后继续或从头开始。
- 有效缓存不因服务重启删除；旧版本在新版本失败时保持可用。

## 八、错误映射和日志

- 对外错误使用接口契约中的稳定 code；完整 URL、绝对路径、底层堆栈和环境变量值只进入受控日志，不能放入 `TaskError.message`。
- `runtime_source_unavailable` 只表示固定来源不可达/不存在；不能提示客户端改 URL。
- `runtime_download_failed` 表示网络或重试耗尽；保留可恢复 `.part`，但不声明已下载归档可复用。
- `runtime_size_mismatch`/`runtime_checksum_mismatch` 发生时删除或隔离当前可疑归档，保留其他有效版本。
- `runtime_archive_invalid` 发生时 staging 必须清理或将清理失败写入受控日志；不能提交 ready。
- 原始阶段错误与清理错误分离；清理失败时父任务 `failed`，`cleanup_error.code=cleanup_failed`，不伪造 `cancelled`。
- 进度写入失败按 `deployment_storage_failed` 处理；不能仅记录日志后继续报成功。

## 九、现有部署链路接入

1. `node_deployment` 继续创建 `RuntimeRequest`，但版本字段为服务端固定值/None，不从 HTTP 透传。
2. `LlamaCppRuntimePreparer::prepare` 返回 `PreparedRuntime` 后，调用现有 `DeploymentService::deploy_prepared`。
3. `LlamaCppRuntimeAdapter` 继续生成 `llama-server` 的模型参数、loopback host、随机端口、alias 和 parallel slots；本修订不把模型下载地址混入运行时 URL。
4. `DeploymentService` 继续负责启动子进程、等待 `/health`、检查 `/v1/models`、检查 `/slots`、绑定路由、监控和停止。
5. 运行时错误在 `preparing_runtime` 阶段写入任务错误，模型分支未开始时不得伪造模型成功；两分支汇合规则不变。

### 9.1 运行时准备完成后的启动命令保持不变

运行时准备器只返回已验证的 `llama-server` 可执行路径。现有 `LlamaCppRuntimeAdapter` 继续生成模型部署命令，不由 Gitee manifest、客户端或下载器拼接新的启动参数：

```text
llama-server
  -m <model.gguf>
  --host 127.0.0.1
  --port <随机端口>
  --alias <model_id>
  --parallel <slots>
```

`DeploymentService` 仍负责进程创建、`/health`、`/v1/models`、`/slots`、路由绑定、监控和停止。运行时缓存目录不能作为模型路径，也不能把 Gitee 归档中的其他可执行文件自动替换为服务端程序。
## 十、测试设计与验收

### 10.1 单元白盒

- 四个 target 到文件名、格式、大小、SHA、server path 的映射；`xcframework` 被排除。
- provider/repository/commit/version、backend、target、摘要和路径字段校验。
- 固定 raw URL 生成；拒绝客户端覆盖 URL/版本/摘要。
- 文件大小和 SHA-256 校验；期望摘要不能写成实际摘要。
- ZIP/TAR.GZ 绝对路径、`..`、驱动器前缀、符号链接、硬链接、entry/单文件/总展开限制。
- cache key、ready、runtime.json、坏缓存隔离、旧版本保留、原子提交。
- 同 key 并发锁和共享订阅取消隔离。
- `.part` sidecar、强 ETag、Range/Content-Range、远端实体变化和 200 重下。
- CUDA 当前 Gitee 来源返回 `unsupported_backend`。

### 10.2 集成白盒

使用本地测试 HTTP 服务模拟：

- 200 全量下载、206 Range 续传、强 ETag；
- ETag 变化、无 ETag、Range 起点错误、Content-Range 错误、服务器对 Range 返回 200；
- 连接中断、无进展、取消、403、404、重定向、5xx 重试；
- 摘要不匹配、长度不匹配、有效旧缓存不被覆盖；
- 解压后缺少 `llama-server`、探测进程立即退出、动态依赖加载失败；
- 多个部署任务共享同一安装作业，取消一个不影响另一个；
- 服务重启后 `.part` 可恢复，未完成父任务标记 `interrupted`。

### 10.3 真实部署黑盒

四个平台必须分别在无预装 llama.cpp、无旧运行时缓存的干净环境验收：

- Windows x64 CPU；
- macOS Intel；
- macOS Apple Silicon；
- Ubuntu x64。

每个平台记录 OS 版本、CPU 架构、Gitee commit、归档大小/SHA、缓存路径、解压路径、`--version`/`--help`、实际 backend、模型加载、`/health`、`/v1/models`、`/slots`、真实对话和流式响应。

另外必须记录取消、停止、服务重启恢复和第二个兼容模型的缓存复用。未形成实机证据的平台只能标记为“未验收”，不能写“已支持”。CUDA 当前任务只能验收 `unsupported_backend`，不能以 CPU 通过替代 CUDA 通过。

## 十一、文档、代码质量和提交门禁

- 新增/修改的公共 Rust API 必须同步中文 `///`，包含用途、`# Examples` 和注意事项；模块 README、根 README、运行时说明和接口/OpenAPI 类型同步更新。
- 新函数、结构体字段默认私有；不为测试导出内部实现；跨文件可见性只在确有调用关系时提高。
- 不使用 `unsafe`；外部输入不得使用 `unwrap/expect`；可恢复错误使用 `Result`，不静默丢弃错误。
- 测试放在独立测试文件；关键模块提供 doc-test；固定资源、下载、归档和探测均有回归测试。
- 编码和测试提交前后执行 `cargo fmt`、`cargo clippy`、`cargo test`，真实记录 workspace、feature、target 和失败原因；提交信息遵循 Conventional Commits。
- 本轮文档阶段不修改 `dev-context.md`、旧设计文档、历史任务报告或用户未跟踪文件。

## 十二、阶段交付和替代关系

- 本文替代旧开发文档中“b8581 基础包内嵌、Windows CUDA GitHub 下载、bundled 来源和 bundled 缺失错误”的运行时部分。
- 旧数据库、模型下载、部署任务、客户端会话、路由、停止和发行门禁设计继续有效；若与本修订的运行时来源冲突，以本修订为准。
- 任务 026–036 是增量实施清单；`task-list/gitee-runtime-download-execution-order.md` 是本修订的独立拓扑顺序，不覆盖历史 `task-list/execution-order.md`。
- 本文不宣称 manifest 已替换、代码已实现、测试已通过或四平台已发布；这些结论必须来自后续任务报告和真实验收证据。

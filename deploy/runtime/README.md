# BurnCloud 受管运行时资源（方案二：Gitee 运行期获取）

`manifest.json` 是服务端可信运行时清单。它只登记固定 Gitee 资源的身份和安全边界，不接受部署请求、本机 Cargo target、系统 `PATH` 或缓存元数据覆盖。

## 固定来源

| 字段 | 固定值 |
| --- | --- |
| provider | `gitee` |
| repository | `songyi777/llama-cpp` |
| commit | `51d01f6e526bc6d45b9d252b1d2d487efb6625d8` |
| version | `b12757` |
| Release URL 模板 | `https://gitee.com/songyi777/llama-cpp/releases/download/{version}/{filename}` |
| 生产期动态发现 | 禁止；Gitee API 只用于资源登记或人工刷新 |

生产下载只使用 manifest 中固定的 Release 标签和文件名；Release 附件经 HTTPS 重定向到 Gitee 附件域名时，只允许 `gitee.com` 与 `foruda.gitee.com` 的精确附件路径及匹配文件名，最多两次重定向。客户端不能提交版本、commit、URL、SHA-256、文件名或解压路径，也不使用 `git clone`、`contents` Base64 或 aria2。

## 受管目标资源

| Cargo `TARGET` | 有效 backend | 包类型 | 文件名 | 格式 | 字节数 | SHA-256 | 服务端路径 |
| --- | --- | --- | --- | --- | ---: | --- | --- |
| `x86_64-pc-windows-msvc` | `cpu` | `cpu` | `llama-b12757-bin-win-cpu-x64.zip` | `zip` | `25262139` | `5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d` | `llama-server.exe` |
| `aarch64-apple-darwin` | `cpu`, `metal` | `native` | `llama-b12757-bin-macos-arm64.tar.gz` | `tar.gz` | `16862081` | `873b413c5d4a336ae623bedde8f755a3ed81b18bc6ae4b01f5a0f6fcea63754c` | `llama-b12757/llama-server` |
| `x86_64-apple-darwin` | `cpu`, `metal` | `native` | `llama-b12757-bin-macos-x64.tar.gz` | `tar.gz` | `12591660` | `f330d7bf9baff5cc401c06524618f9eaf8962439dcb9e11fa638f8e3b0bee14b` | `llama-b12757/llama-server` |
| `x86_64-unknown-linux-gnu` | `cpu` | `cpu` | `llama-b12757-bin-ubuntu-x64.tar.gz` | `tar.gz` | `23425964` | `99e34cc6a39f3afda735d43d76d15d6ead2297406cc9c0cf421b771fd3a1a84a` | `llama-b12757/llama-server` |

上述大小和摘要是归档实体的字节数与完整 SHA-256，不是 Git blob id、Gitee API 的 Git `sha`、解压目录大小或发行物体积。登记或刷新资源时必须对实际下载字节重新计算，不能复制旧版本摘要。

本清单只包含四个桌面/Ubuntu CPU 或 Metal 目标：

- Windows x64 只登记 CPU；
- macOS x64 和 ARM64 登记 CPU、Metal 两种请求能力，但共享同一平台归档；
- Ubuntu x64 只登记 CPU；
- `xcframework` 不属于桌面 `llama-server` target 映射；
- Gitee 当前没有 CUDA 归档，不生成 CUDA 条目。请求 CUDA 时由运行时准备器返回 `unsupported_backend`，不得下载 CPU 包冒充 CUDA 或静默降级。

## 可信校验契约

读取 manifest 和选择目标时，服务端必须拒绝以下情况，并将配置错误映射为 `runtime_manifest_invalid`；backend 不在目标允许集合时映射为 `unsupported_backend`：

- `provider`、`repository`、`commit` 或 `version` 偏离固定值；
- target 不在四个登记 target 中，或目标与当前编译平台不匹配；
- 文件名不是单一基名，含路径分隔符、空字节或控制字符；
- `archive_format` 与扩展名、目标平台不匹配；
- 文件大小为零、超出下载预算，或与实际归档长度不一致；
- SHA-256 不是 64 位连续小写十六进制，或完整归档重算结果不匹配；
- `server_relative_path` 为绝对路径、包含 `..`、驱动器前缀或越出解压根；
- 归档限制不是正数，或超过服务端硬上限；
- 运行时请求要求 CUDA、其他未登记 backend 或未登记目标。

manifest 是服务端配置，不是客户端可扩展的 JSON。校验失败不能通过选择旧版本、其他 target 或另一种 backend 来掩盖。

## 解压与运行时安全边界

默认展开限制登记在 `manifest.json`：最大路径 4096 字节、最大 entry 数 100000、单文件最大 2 GiB、总展开最大 4 GiB。运行时实现还必须拒绝绝对路径、`..` 穿越、越出 staging 的符号链接/硬链接和超出资源限制的归档；解压后必须找到 manifest 指定的普通文件 `server_relative_path`。

`ready` 标记只用于缓存加速，不能代替再次检查文件存在、路径边界、版本、摘要、依赖和 backend 探测。运行时缓存写入用户数据目录，不写入安装目录、项目目录或 `target`。

## 来源优先级与接口边界

运行时准备顺序固定为：

1. 管理员显式配置 `BURNCLOUD_LLAMA_SERVER`；
2. 已校验的用户缓存；
3. 固定 Release 标签 `b12757` 的 Gitee 附件下载，并核对固定 commit 来源、字节数及 SHA-256。

方案二继续复用自动部署的 POST/GET/DELETE 接口，运行时下载、校验、解压、探测和缓存提交作为部署任务的 `preparing_runtime` 子阶段展示；不增加 runtime 管理 HTTP API。模型 GGUF 下载、`DeploymentService` 进程生命周期、健康检查和路由绑定不由本清单改变。

## 资源登记复核

资源登记或人工刷新必须对固定 Release 附件的实际归档字节执行：

```powershell
$file = Get-Item '.\llama-b12757-bin-win-cpu-x64.zip'
$file.Length
Get-FileHash $file.FullName -Algorithm SHA256
```

登记工具应比较实际长度与 `size_bytes`、比较完整 SHA-256 与 `sha256`，并在不匹配时失败；不得使用 Git blob id 代替归档 SHA-256。2026-11-27 已通过 reqwest 下载并核对四个 Release 附件的大小和 SHA-256；Windows x64 的 `LlamaCppRuntimePreparer` 实测走完下载、校验、解压、探测和缓存提交，生成 `ready`/`runtime.json`，第二次准备复用缓存成功；`llama-server.exe --version` 与 `--help` 均成功。本机默认缓存根为 `%LOCALAPPDATA%\BurnCloud\runtimes`。尚未完成有效 GGUF 模型加载、服务健康接口及真实对话，也未在 macOS Intel、macOS Apple Silicon、Ubuntu 目标机执行，因此这些平台仍标记为“未验收”，不得据附件下载成功宣称已支持。

## 历史方案说明

旧的 b8581 内嵌运行时清单和发行说明由历史任务 005–025 保留，不作为方案二的默认运行时回退。若后续需要离线发行，应另立增强任务，不得在本清单中偷偷恢复内嵌回退。

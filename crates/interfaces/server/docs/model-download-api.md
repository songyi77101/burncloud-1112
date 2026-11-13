# Node 模型下载与摘要校验接口

本文档描述模型目录选择、硬件/模型解析和 Node 文件下载之间的前后端契约。接口只负责把已选中的 GGUF 工件下载到运行 BurnCloud Server/Node 的主机并校验 SHA-256；它不启动 Runtime、不部署模型、不做健康检查，也不执行真实推理。

## 1. 调用顺序

前端必须先完成模型目录选择和模型解析，再提交解析结果中的完整候选版本：

```text
GET /api/models/presets
或 GET /api/models/search?q=...
        ↓
用户确认 model + source + revision
        ↓
POST /api/node/models/resolve
        ↓
服务端检测 Node 硬件、发现 GGUF、返回 candidates
        ↓
用户选择一个 candidate
        ↓
POST /api/node/models/download
        ↓
Node 下载或复用文件，读取文件计算 SHA-256
        ↓
返回 verified，前端进入 ArtifactReady
```

`/api/node/models/resolve` 的请求和响应不因本接口而改变。前端必须使用解析响应中的同一个 `model_name`、`variant_id`、`artifact_uri` 和 `checksum`，不能根据展示名称或本地路径重新拼接下载参数。

## 2. 接口

### 2.1 基本信息

- **方法**：`POST`
- **路径**：`/api/node/models/download`
- **访问**：公开访问，不要求 JWT；与 `/api/node/hardware`、`/api/node/models/resolve` 一致。
- **响应**：JSON；成功和错误响应均带 `Cache-Control: no-store`。
- **等待方式**：一次请求等待下载完成；本次不提供任务轮询、暂停、恢复或取消接口。

### 2.2 请求 JSON

```json
{
  "model": "Qwen/Qwen3-8B",
  "source": "modelscope",
  "revision": "master",
  "variant_id": "Q4_K_M.gguf",
  "artifact_uri": "https://modelscope.cn/models/Qwen/Qwen3-8B/resolve/master/Q4_K_M.gguf",
  "checksum": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}
```

字段契约：

| 字段 | 类型 | 必填 | 校验与含义 |
|---|---|---:|---|
| `model` | `string` | 是 | `owner/name` 形式；两段均为安全模型标识，不能包含路径跳转。 |
| `source` | `string` | 是 | 只能是 `modelscope`、`hf-mirror` 或 `huggingface`。 |
| `revision` | `string` | 是 | 分支、标签或提交修订号；只能是单一安全路径段。 |
| `variant_id` | `string` | 是 | 解析接口返回的 GGUF 相对路径/版本标识；不能为空、包含路径跳转或反斜杠，必须以 `.gguf` 结尾。 |
| `artifact_uri` | `string` | 是 | 支持仓库的 HTTPS `resolve` 工件地址；地址中的仓库、模型、修订号和工件路径必须与其他字段一致。 |
| `checksum` | `string` | 是 | 必须为 `sha256:` 加 64 位十六进制摘要。大小写不影响比较，服务端以小写 `sha256:` 形式保存。 |

支持的工件地址格式：

```text
https://modelscope.cn/models/{owner}/{name}/resolve/{revision}/{variant_id}
https://hf-mirror.com/{owner}/{name}/resolve/{revision}/{variant_id}
https://huggingface.co/{owner}/{name}/resolve/{revision}/{variant_id}
```

服务端还会拒绝带用户名/密码、URL 片段、非 HTTPS、非 `.gguf`、仓库主机不匹配或与请求身份不匹配的地址。下载器自身会再次拒绝回环、私有、链路本地等非公网目标地址；无效请求不会初始化 aria2。

### 2.3 成功响应

HTTP `200`：

```json
{
  "success": true,
  "data": {
    "status": "verified",
    "model": "Qwen/Qwen3-8B",
    "source": "modelscope",
    "revision": "master",
    "variant_id": "Q4_K_M.gguf",
    "local_path": "C:/BurnCloud/server/models/modelscope/Qwen_Qwen3-8B/master/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/Qwen3-8B",
    "actual_sha256": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  }
}
```

`status` 目前只有 `verified`。`local_path` 是 Node/Server 主机上的文件路径，不是浏览器或桌面客户端的本地路径。前端只能展示该路径并保存到当前准备卡片，不能尝试在客户端读取、复制或删除它。

### 2.4 错误响应

所有错误均使用如下结构：

```json
{
  "success": false,
  "error": {
    "code": "checksum_mismatch",
    "message": "下载文件摘要与请求摘要不一致"
  }
}
```

| HTTP 状态 | `error.code` | 触发条件 | 前端处理 |
|---:|---|---|---|
| `400` | `invalid_download_request` | 模型、来源、修订号、版本标识、地址或摘要格式无效；请求体不是合法 JSON；下载器拒绝地址/目录/文件名。 | 转为初始化参数错误，保留用户草稿。 |
| `422` | `checksum_mismatch` | 文件下载完成，但实际 SHA-256 与请求摘要不一致。 | 转为摘要校验错误，不进入 `ArtifactReady`。 |
| `502` | `download_failed` | aria2 任务、网络或上游仓库下载失败。 | 转为下载执行错误，可重试。 |
| `503` | `download_service_unavailable` | 首次请求懒加载 `DownloadManager` 或 aria2 初始化失败。 | 转为下载服务不可用，可重试。 |
| `500` | `download_storage_failed` | 服务端无法读取文件或计算 SHA-256。 | 转为摘要/存储错误，不进入 `ArtifactReady`。 |

## 3. 下载、校验和幂等规则

1. Server 使用懒加载的 `tokio::sync::OnceCell<DownloadManager>`。服务启动时不会因为挂载下载路由而立即启动 aria2；第一个**通过请求校验**的下载请求才会初始化下载管理器。
2. 服务端保存目录由 `source`、模型标识、`revision` 和期望摘要组成，形式为：

   ```text
   models/{source}/{model-with-slash-replaced}/{revision}/{sha256-without-prefix}/
   ```

   `DownloadManager` 再根据 `artifact_uri` 的文件名生成最终文件路径。目录和文件均位于运行可执行文件同级的 `models` 目录内。
3. `DownloadManager` 对同一 URL、目录和文件名的并发请求进行等待者合并；已完成且仍存在的文件会直接复用。
4. 服务端即使复用已下载文件，也会重新读取文件并计算 SHA-256；只有摘要一致才返回 `verified`。
5. 幂等身份是请求中的 `model`、`source`、`revision`、`variant_id`、`artifact_uri` 和 `checksum`。重复请求不会因为之前已经下载而跳过摘要校验。
6. 摘要不一致不能生成成功凭据；客户端不得把 `local_path` 当作可推理模型路径。

## 4. 前端状态流转

目录模型的状态由 `ChatState` 驱动：

```text
AwaitingStart
  → Detecting
  → SelectingVariant
  → Downloading
  → Verifying
  → ArtifactReady
```

- `Downloading`：前端只发送 `DownloadProgress { completed: 0, total: None }`，因为当前接口没有下载进度流，不伪造百分比。
- `Downloaded`：服务端成功返回文件后发送。
- `Verified`：使用服务端返回的 `actual_sha256` 和 `local_path`，并核对模型、来源、修订号、`variant_id`。
- `ArtifactReady`：表示文件已下载并完成摘要校验，但尚未加载、部署或健康检查。
- `ArtifactReady` 不写入 `ready_models`，`can_send()` 保持关闭，不能启动生成任务。
- 下载成功后保留 Node 文件路径、实际摘要和用户草稿；不会把下载结果伪装成部署失败。

本地占位模型（`catalog_selection == None`）仍沿用旧的本地模拟链路：

```text
Downloading → Verifying → LoadingPackage → Deploying → HealthChecking → Reporting → Ready
```

只有本地占位模型进入 `Ready` 后才记录为可推理模型并允许发送消息。本次改动不实现目录模型的 Runtime 启动、部署、健康检查和真实推理。

## 5. 最小前端调用示例

```ts
type DownloadRequest = {
  model: string;
  source: "modelscope" | "hf-mirror" | "huggingface";
  revision: string;
  variant_id: string;
  artifact_uri: string;
  checksum: string;
};

type DownloadData = DownloadRequest & {
  status: "verified";
  local_path: string;
  actual_sha256: string;
};

async function downloadNodeModel(request: DownloadRequest): Promise<DownloadData> {
  const response = await fetch("/api/node/models/download", {
    method: "POST",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
    },
    cache: "no-store",
    body: JSON.stringify(request),
  });
  const body = await response.json();
  if (!response.ok || body.success !== true) {
    throw new Error(body.error?.message ?? "模型下载失败");
  }
  return body.data as DownloadData;
}
```

真实前端还必须像当前 Rust `CatalogClient::download_model` 一样逐项核对 `status`、模型身份、量化版本、非空 Node 路径和摘要，不能只判断 HTTP `200`。

## 6. 对应代码和文档

| 能力 | 位置 |
|---|---|
| 下载 Handler 与参数校验 | `crates/interfaces/server/src/api/model_download.rs` |
| 下载管理器懒加载状态 | `crates/interfaces/server/src/lib.rs` |
| 路由注册 | `crates/interfaces/server/src/api/mod.rs` |
| OpenAPI 路由和 Schema | `crates/interfaces/server/src/api/openapi.rs` |
| 前端下载客户端 | `crates/interfaces/client/src/domains/chat/catalog.rs` |
| 前端目录模型初始化分流 | `crates/interfaces/client/src/domains/chat/backend.rs` |
| 前端准备状态与 `ArtifactReady` | `crates/interfaces/client/src/domains/chat/model.rs`、`state.rs` |
| 前端下载卡片 | `crates/interfaces/client/src/domains/chat/page.rs` |
| 上一个功能的解析契约 | `docs/hardware-model-resolution-api.md` |


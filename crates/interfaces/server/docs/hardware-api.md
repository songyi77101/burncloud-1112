# Node 硬件检测接口

本文档描述 BurnCloud Server 暴露给前端的 Node 主机硬件检测接口。接口读取的是**运行 BurnCloud Node/Server 的服务端主机**，不是浏览器或桌面客户端所在机器。

## 1. 接口概览

| 项目 | 约定 |
|---|---|
| URL | `GET /api/node/hardware` |
| 认证 | 公开只读接口，不要求 JWT |
| 请求体 | 无 |
| 响应格式 | 一次性 JSON，不使用 SSE 或 WebSocket |
| 缓存 | 所有响应返回 `Cache-Control: no-store` |
| OpenAPI | `/api-docs/openapi.json` 中的 `getNodeHardware` |

请求示例：

```http
GET /api/node/hardware HTTP/1.1
Accept: application/json
Cache-Control: no-cache
```

```bash
curl -i -H "Accept: application/json" http://127.0.0.1:3000/api/node/hardware
```

## 2. 成功响应

HTTP 状态码为 `200 OK`，响应体为：

```json
{
  "success": true,
  "data": {
    "platform": "windows",
    "arch": "x86_64",
    "cpu": "AMD Ryzen 9 7950X 16-Core Processor",
    "memory_bytes": 34359738368,
    "disk_available_bytes": 512110190592,
    "accelerators": [
      {
        "kind": "nvidia",
        "name": "NVIDIA GeForce RTX 4090",
        "memory_bytes": 25769803776
      }
    ]
  }
}
```

### 2.1 字段说明

| 字段 | 类型 | 必填 | 说明 |
|---|---|---:|---|
| `success` | `boolean` | 是 | 成功响应固定为 `true`。 |
| `data.platform` | `string` | 是 | `linux`、`macos`、`windows` 或 `other`。 |
| `data.arch` | `string` | 是 | Rust 编译目标架构，例如 `x86_64`、`aarch64`。 |
| `data.cpu` | `string` | 是 | 操作系统报告的 CPU 型号；无法读取时可能为 `Unknown CPU`。 |
| `data.memory_bytes` | `number` | 是 | 系统物理内存**总量**，单位为字节，不是当前剩余内存。 |
| `data.disk_available_bytes` | `number` | 是 | 当前工作目录所在文件系统的可用空间，单位为字节，不代表所有磁盘总和。 |
| `data.accelerators` | `array` | 是 | 已识别的 GPU 或其他计算加速器；没有可识别设备时为空数组。 |
| `accelerators[].kind` | `string` | 是 | `nvidia`、`amd`、`intel`、`apple` 或 `other`。 |
| `accelerators[].name` | `string` | 是 | 驱动或操作系统报告的设备名称。 |
| `accelerators[].memory_bytes` | `number \| null` | 是 | 设备显存容量，单位为字节；无法获取时为 `null`。 |

服务端 DTO 会将内部 Rust 枚举 `Nvidia`、`Amd` 等转换为前端约定的小写值，不应依赖内部类型的默认序列化结果。

### 2.2 典型响应

无任何可识别加速器时：

```json
{
  "success": true,
  "data": {
    "platform": "linux",
    "arch": "x86_64",
    "cpu": "Intel Xeon",
    "memory_bytes": 17179869184,
    "disk_available_bytes": 107374182400,
    "accelerators": []
  }
}
```

检测到设备但显存未知时：

```json
{
  "success": true,
  "data": {
    "platform": "windows",
    "arch": "x86_64",
    "cpu": "Intel Core i7",
    "memory_bytes": 17179869184,
    "disk_available_bytes": 107374182400,
    "accelerators": [
      {
        "kind": "intel",
        "name": "Intel Graphics",
        "memory_bytes": null
      }
    ]
  }
}
```

Apple Silicon 使用统一内存模型，Apple GPU 的 `memory_bytes` 使用系统物理内存总量表示。例如：

```json
{
  "success": true,
  "data": {
    "platform": "macos",
    "arch": "aarch64",
    "cpu": "Apple M3 Pro",
    "memory_bytes": 17179869184,
    "disk_available_bytes": 268435456000,
    "accelerators": [
      {
        "kind": "apple",
        "name": "Apple M3 Pro",
        "memory_bytes": 17179869184
      }
    ]
  }
}
```

## 3. 错误响应

### 3.1 不支持的平台

当当前编译目标没有硬件探测实现时，返回 `501 Not Implemented`：

```json
{
  "success": false,
  "error": {
    "code": "unsupported_platform",
    "message": "当前平台暂不支持硬件检测"
  }
}
```

### 3.2 通用硬件检测失败

当 CPU、物理内存或当前工作目录所在文件系统等通用硬件信息无法读取时，返回 `500 Internal Server Error`：

```json
{
  "success": false,
  "error": {
    "code": "hardware_probe_failed",
    "message": "硬件检测失败"
  }
}
```

详细的系统调用、目录路径、PowerShell 或驱动错误只写入服务端日志，不直接返回给未认证前端。GPU 驱动查询失败不会单独导致接口失败；探测器会尽量返回无 GPU 或部分加速器结果。

## 4. TypeScript 接入

```ts
type AcceleratorKind =
  | "nvidia"
  | "amd"
  | "intel"
  | "apple"
  | "other";

interface AcceleratorProfile {
  kind: AcceleratorKind;
  name: string;
  memory_bytes: number | null;
}

interface HardwareSnapshot {
  platform: "linux" | "macos" | "windows" | "other";
  arch: string;
  cpu: string;
  memory_bytes: number;
  disk_available_bytes: number;
  accelerators: AcceleratorProfile[];
}

type HardwareApiResponse =
  | {
      success: true;
      data: HardwareSnapshot;
    }
  | {
      success: false;
      error: {
        code: "unsupported_platform" | "hardware_probe_failed";
        message: string;
      };
    };

async function fetchHardware(): Promise<HardwareSnapshot> {
  const response = await fetch("/api/node/hardware", {
    method: "GET",
    headers: {
      Accept: "application/json",
    },
    cache: "no-store",
  });

  const body = (await response.json()) as HardwareApiResponse;

  if (!response.ok || !body.success) {
    const message = body.success
      ? `硬件检测失败：HTTP ${response.status}`
      : body.error.message;
    throw new Error(message);
  }

  return body.data;
}
```

建议在用户打开本地模型准备页面、点击“检测硬件/重新检测”或选定模型后展示量化版本前调用一次，并在同一准备流程内复用快照；不要在每次页面渲染时重复调用。

## 5. 资源判断规则

```ts
function maxKnownVramBytes(
  accelerators: AcceleratorProfile[],
): number | null {
  const values = accelerators
    .map((item) => item.memory_bytes)
    .filter((value): value is number => value !== null);

  return values.length > 0 ? Math.max(...values) : null;
}
```

前端必须遵守以下规则：

- 不要把多张 GPU 的显存相加；多卡场景使用已知显存最大的单卡。
- `accelerators: []` 表示没有已识别设备，不等同于驱动一定不存在。
- GPU 存在但所有 `memory_bytes` 都为 `null` 时显示“显存未知”，不要显示为 `0`。
- `memory_bytes` 显示为“内存总量”，不要标记为“可用内存”。
- `accelerators[].memory_bytes` 显示为“显存容量”，不保证是实时剩余显存。
- 字节值展示时建议使用 `1024³` 换算为 GiB。
- 快照只用于前端预览和初步筛选，最终模型兼容性以服务端模型解析结果为准。

## 6. 平台实现摘要

- Windows 使用 NVML 检测 NVIDIA，并使用 PowerShell CIM 查询 AMD、Intel 等其他显卡；NVIDIA 不会由 CIM 重复返回。
- Linux 使用 NVML 检测 NVIDIA，并从 `/sys/class/drm` 查询 AMD、Intel 和其他 DRM 设备；NVIDIA DRM 设备会跳过。
- macOS 使用 `system_profiler SPDisplaysDataType -json`，Apple GPU 使用统一内存模型。
- CPU、物理内存或当前目录磁盘无法读取时，整体接口可能返回 `hardware_probe_failed`。
- GPU 查询失败时会降级为无 GPU 或部分结果。

接口实现位置：

- `crates/interfaces/server/src/api/hardware.rs`：HTTP handler 与 DTO 转换。
- `crates/platform/node/src/contracts.rs`、`types.rs`：硬件探测内部契约。
- `crates/interfaces/server/src/api/openapi.rs`：OpenAPI 路径与 schema。
- `crates/interfaces/client/src/domains/chat/catalog.rs`：前端 Rust 客户端解析接口响应。
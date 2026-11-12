// 本地联调用的模拟模型服务：补上 /api/models/artifacts，其余请求原样转发到真实模型服务。
// 仅用于开发验证，没有鉴权，只监听 127.0.0.1。
//
// 用法：
//   node scripts/mock-artifacts-server.mjs
//   $env:BURNCLOUD_MODEL_SELECTION_URL = 'http://127.0.0.1:4174'
//   cargo run -p burncloud-client
//
// 环境变量：
//   MOCK_PORT          监听端口，默认 4174
//   UPSTREAM_URL       真实模型服务地址，默认 http://127.0.0.1:4173
//   MOCK_ARTIFACT_FILE 可选，指定一个本地文件作为下载工件；不填则生成 8 MiB 测试文件
//   MOCK_FORCE_DOWNLOAD=1  可选，让所有版本都显示“下载”（改写 status/resource-status）

import { createServer, request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { basename } from "node:path";

const PORT = Number(process.env.MOCK_PORT ?? 4174);
const UPSTREAM = new URL(process.env.UPSTREAM_URL ?? "http://127.0.0.1:4173");
const FORCE_DOWNLOAD = process.env.MOCK_FORCE_DOWNLOAD === "1";

// 工件内容一次性读入内存，大小和摘要在启动时算好，保证与实际下载字节一致。
const artifactPath = process.env.MOCK_ARTIFACT_FILE;
const artifact = artifactPath
  ? readFileSync(artifactPath)
  : Buffer.alloc(8 * 1024 * 1024, "burncloud-mock-artifact\n");
const fileName = artifactPath ? basename(artifactPath) : "mock-q4_k_m.gguf";
const sha256 = "sha256:" + createHash("sha256").update(artifact).digest("hex");
const MIB = 1024 * 1024;

function sendJson(res, status, body) {
  const text = JSON.stringify(body);
  res.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(text),
  });
  res.end(text);
}

// 客户端会校验 model/version/revision/source，缺任何一个都按无效请求处理。
function handleArtifacts(url, res) {
  const missing = ["model", "version", "revision", "source"].filter(
    (key) => !url.searchParams.get(key)
  );
  if (missing.length) {
    sendJson(res, 400, { message: `缺少参数：${missing.join(", ")}` });
    return;
  }
  sendJson(res, 200, {
    type: "artifacts",
    items: [
      {
        id: "q4_k_m",
        quantization: "Q4_K_M",
        runtime: "llama.cpp",
        url: `http://127.0.0.1:${PORT}/files/${encodeURIComponent(fileName)}`,
        sha256,
        size_bytes: artifact.length,
        // 资源要求远低于本地替身报告的硬件，确保能进入量化选择。
        ram_bytes: 512 * MIB,
        vram_bytes: 0,
      },
    ],
  });
}

function handleFile(res) {
  res.writeHead(200, {
    "Content-Type": "application/octet-stream",
    "Content-Length": artifact.length,
  });
  res.end(artifact);
}

// 流式转发，保证搜索接口的 NDJSON 快照能逐条到达客户端。
function proxy(req, res, url) {
  const target = new URL(url.pathname + url.search, UPSTREAM);
  const send = target.protocol === "https:" ? httpsRequest : httpRequest;
  const upstream = send(
    target,
    { method: req.method, headers: { ...req.headers, host: target.host } },
    (upstreamRes) => {
      res.writeHead(upstreamRes.statusCode ?? 502, upstreamRes.headers);
      upstreamRes.pipe(res);
    }
  );
  upstream.on("error", (error) => {
    console.error(`[mock] 转发失败 ${target}: ${error.message}`);
    if (!res.headersSent) sendJson(res, 502, { message: "上游模型服务不可用" });
    else res.destroy(error);
  });
  req.pipe(upstream);
}

const server = createServer((req, res) => {
  const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "localhost"}`);
  console.log(`[mock] ${req.method} ${url.pathname}${url.search}`);
  if (url.pathname === "/api/models/artifacts") return handleArtifacts(url, res);
  if (url.pathname.startsWith("/files/")) return handleFile(res);
  if (FORCE_DOWNLOAD && url.pathname === "/api/models/status") {
    return sendJson(res, 200, { status: "developing" });
  }
  if (FORCE_DOWNLOAD && url.pathname === "/api/models/resource-status") {
    return sendJson(res, 200, { resource_status: 200 });
  }
  proxy(req, res, url);
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`[mock] 监听 http://127.0.0.1:${PORT}，上游 ${UPSTREAM.origin}`);
  console.log(`[mock] 工件 ${fileName}，${artifact.length} 字节，${sha256}`);
  if (FORCE_DOWNLOAD) console.log("[mock] 已强制所有版本显示“下载”");
});

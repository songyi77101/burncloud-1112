{
  "编号": "035",
  "依赖": ["032", "033"]
}

# 035 同步运行时 rustdoc、README 与接口说明

- 任务：同步方案二实现涉及的公共 rustdoc、模块 README、根 README、运行时资源说明和 OpenAPI/接口类型文档。
- 工作区：server（读写）——服务端公共类型/README/OpenAPI；download（读写）——下载模块 README/rustdoc；project-root（读写）——根 README；runtime-assets（读写）——`deploy/runtime/README.md`。
- 产出：与实际实现一致的中文文档、示例和支持边界说明。
- 前置任务：032、033。
- 后置任务：036。
- 验收标准：
  - 公共 API 的 `///` 文档包含用途、`# Examples` 和注意事项；不虚构尚未支持的 CUDA/Gitee API。
  - README 明确四目标、Gitee 固定 commit、缓存位置、来源优先级、无内嵌默认回退和 CUDA `unsupported_backend`。
  - 接口/OpenAPI 字段与 `RuntimeProgress`、错误码、请求禁止字段一致。
  - 文档不修改 `dev-context.md`、历史任务报告和用户未跟踪文件；未验收平台不标记已支持。

## 实现内容

本任务不新增业务函数；只同步开发文档中已经确定的公共 API、README、运行时说明和接口类型。

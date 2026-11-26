# dev-context

> 本文件由 spec-manager 独占写入；用户于 2026-11-24 确认开发序号、分支、工作区与基于现有实现继续开发。

- 开发序号：5
- 分支名：codex/local-model-auto-deploy
- 项目根目录：E:/project/burncloud1.1
- 已确认 Issue：https://github.com/songyi77101/burncloud-1112/issues/13

## 工作区

| 名称 | 类型 | 目录 |
| --- | --- | --- |
| download | 读写 | crates/platform/storage/download/ |
| database | 读写 | crates/platform/storage/database/ |
| database-sys | 读写 | crates/platform/storage/database-sys/ |
| node | 读写 | crates/platform/node/ |
| server | 读写 | crates/interfaces/server/ |
| client | 读写 | crates/interfaces/client/ |
| release | 读写 | .github/workflows/ |
| runtime-assets | 读写 | deploy/ |
| project-root | 读写 | . |

根工作区只允许 Cargo.toml、Cargo.lock、README.md 及本次开发文档；不覆盖用户未跟踪的 server/docs/hardware-model-resolution-api.md，不写 .env、系统配置或用户现有 llama.cpp。

# 项目目录架构规范

> 本文件只保留项目当前最新的目录架构；历史变更见 `docs/dev-standards/migration-info/`。
> 本次修正沿用现有目录，不新增业务层级；临时开发文档按开发序号归档。

## 目录树

```text
<项目根>/
├── crates/                 # Rust workspace 的领域、平台和接口 crate
│   ├── commerce/           # 计费与商业领域
│   ├── identity/           # 用户与身份领域
│   ├── interfaces/         # CLI、服务端、客户端和集成测试入口
│   ├── platform/           # 配置、生命周期、节点、存储与可观测性基础设施
│   ├── supply/             # 模型、供应和渠道领域
│   ├── traffic/            # 路由与数据面流量处理
│   └── trust/              # 推理与信任边界能力
├── deploy/                 # 发布、运行时资源和部署脚本
│   ├── runtime/            # 运行时资源登记与说明
│   └── scripts/            # 部署辅助脚本
├── docs/                   # 开发规范、开发文档与研究记录
│   ├── dev-standards/      # Rust 编码与目录架构规范
│   ├── dev-docs/           # 按开发序号归档的需求、设计、任务和报告
│   └── research/           # 研究性资料，不作为生产配置来源
├── .github/                # CI、发布和仓库自动化配置
├── Cargo.toml              # workspace、依赖和全局 lint 配置
├── README.md               # 项目说明、运行方式和质量门禁
└── AGENTS.md               # 本地开发代理强制约束
```

## 目录职责

| 目录 | 职责 | 内容类型 | 依赖方向 |
| --- | --- | --- | --- |
| `crates/` | 承载全部 Rust workspace 包及业务、接口、平台实现 | Rust 源码、模块 README、单元和集成测试 | 子 crate 按 workspace 依赖方向组织；生产代码禁止依赖测试目录 |
| `crates/interfaces/server/` | 构建 Axum 服务和管理接口 | 服务端 Rust 源码、接口文档、测试 | 可依赖平台、领域和公共契约；禁止反向依赖客户端实现 |
| `crates/interfaces/client/` | 提供 Dioxus 客户端与自动部署交互 | 客户端 Rust 源码、资源、测试 | 通过公共契约访问服务端能力；禁止承载服务端生命周期 |
| `crates/platform/node/` | 管理本机节点、模型部署和运行时生命周期 | 节点与部署 Rust 源码、测试 | 可依赖存储、配置、领域契约；不依赖 UI |
| `crates/platform/storage/download/` | 提供原生 HTTP 下载、校验和下载任务能力 | 下载 Rust 源码、测试、README | 可依赖公共配置和网络库；不依赖接口层 |
| `crates/platform/storage/database/` | 提供持久化数据库访问和迁移 | 数据库 Rust 源码、迁移、测试 | 作为基础设施被服务和平台层调用 |
| `deploy/` | 管理发行期和运行期部署资产 | 运行时清单、说明和脚本 | 只被构建、部署或运行时准备逻辑读取，不承载业务代码 |
| `docs/dev-standards/` | 维护当前编码和目录架构规范 | Markdown 规范与迁移信息 | 流程管理输入，不被生产 crate 依赖 |
| `docs/dev-docs/` | 归档每次开发的需求、设计、任务、测试和报告 | Markdown 文档 | 流程阶段读取，不被生产 crate 依赖 |
| `docs/research/` | 保存外部资料核验和研究结论 | Markdown 研究记录 | 仅供设计参考，不作为运行时动态配置 |
| `.github/` | 执行 CI、发布和仓库自动化 | YAML、脚本、说明 | 依赖仓库构建入口，不被 Rust crate 依赖 |

## 命名与组织约定

- Rust 包和源码目录使用小写短横线或 Rust 社区约定；模块实现按职责归入既有 `crates/<领域>/<模块>/` 层级。
- 新增运行时资产放入 `deploy/runtime/`，运行时用户缓存不得写入仓库目录。
- 每次开发文档放入 `docs/dev-docs/<开发序号>/`；任务清单、测试流程、任务报告和测试报告使用既有子目录。
- 研究资料放入 `docs/research/`，不得冒充生产 manifest、接口契约或部署配置。

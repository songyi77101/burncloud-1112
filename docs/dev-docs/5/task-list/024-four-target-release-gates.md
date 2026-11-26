```json
{
  "编号": "024",
  "依赖": [
    "007",
    "010",
    "019",
    "023"
  ]
}
```

# 024 扩展四目标便携发行与证据门禁

- 任务：扩展四目标便携发行与证据门禁
- 工作区：release（读写）——.github/workflows/cd-release.yml 及必要同目录工作流校验
- 工作区：runtime-assets（读写）——deploy/runtime/README.md 与资源/体积/许可证据说明
- 产出：四目标原生构建矩阵、便携ZIP与独立发布检查
- 前置任务：007 实现本平台唯一运行包离线内嵌；010 实现共享受管运行时准备与原子启用；019 实现异步部署任务 HTTP 契约；023 接通 BurnCloud 路由真实对话与 SSE
- 后置任务：025 同步本机自动部署用户说明与支持边界
- 设计依据：第七节 7.2、7.3；第八节 8.1、8.2
- 验收标准：
  - 矩阵仅四个已批准target；各自原生/相应架构构建并启用bundled-runtime，Linux使用既有服务端/Web不混入原生GUI改造。
  - 先资源工具生成真实锁定清单再离线Cargo构建；每发行物只含本目标基础包，默认没有CUDA，许可/依赖完整并可随单文件更新保留。
  - 外层burncloud-<version>-<target>.zip兼容现有self_update，主程序位于既定位置；Unix执行位在归档/解压/更新验证后保留，不新建安装器或通用更新协议。
  - 同代码/优化分别带与不带feature构建，报告主程序/ZIP绝对体积及差值、源/派生包和Windows依赖增量，不宣称体积不变。
  - 许可证/授权、mac签名公证、Intel runner或展开库存等证据缺失即阻断对应目标发布，不发明证据或提供跳过开关。
  - 四基础平台与WindowsNVIDIA CUDA各有独立实机门禁：干净缓存、阻断直连GitHub、固定SHA GGUF、BurnCloud路由真实对话及复用/刷新/取消/重启/损坏恢复；未执行明确未验证，CPU通过不能替CUDA。
  - 工作流静态/构建负例测试覆盖缺资源和证据；CI链接成功/202/下载成功不充当实机成功。

> 来源：开发序号5、用户于2026-11-24确认的 `../local-model-auto-deploy-dev-doc.md`、同目录流程图与接口契约；分支 `codex/local-model-auto-deploy`，工作区以 `../dev-context.md` 为准。任务定义不代表业务代码或实机验收已完成；实际执行次序只采用脚本生成的 `execution-order.md`。

## 范围与执行约束

- 本任务实现发行检查与打包，不把缺硬件/签名授权的实测项标通过；真实黑盒测试流程与执行仍由后续测试阶段负责。
- 只实现本任务摘录的定稿设计；新函数/方法/字段默认私有，跨文件和跨crate可见性不得超过开发文档第六节公开清单，不为测试导出内部实现。
- 注释与rustdoc使用中文；公共边界黑盒校验，内部公有函数白盒契约检查；私有函数不重复校验已保证参数，但新读取的网络/磁盘/归档/进程结果仍须验证。
- 测试写入独立文件；每个公有API/内部公有方法有独立测试，关键公共模块提供可编译# Examples，修复有回归；不使用unsafe、外部输入unwrap/expect或静默丢弃Result。
- 只写上述登记工作区和具体范围；project-root仅登记允许的Cargo.toml/Cargo.lock/README.md/本轮文档。不得覆盖用户target运行时、未跟踪server/docs文件、开发规范或dev-context.md。
- 编码/测试/提交状态由后续任务报告如实记录；提交使用Conventional Commits，提交前后均执行cargo fmt、cargo clippy、cargo test并记录实际feature/target与结果。真实硬件、许可、签名或测试资源缺失不填假证据。

## 实现内容

### 实现约束原文摘录

#### 7.3 四目标便携发行

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

---

#### 8.1 必须分别完成的真实部署验收

1. 每个目标使用无预装 llama.cpp、无受管缓存的干净系统；记录真实 OS/架构、发行 SHA 和资源锁定清单。候选系统不等于已支持，Linux 首期只以 Ubuntu 24.04 x64 验收。
2. 防火墙/代理规则禁止直连 GitHub 与其资源域名，保存网络证据；基础运行时离线准备成功，模型从项目支持源下载固定版本/摘要的轻量 GGUF。
3. 通过新接口及客户端完成真实 SHA、启动、健康、路由，然后用合法 BurnCloud Token 经 BurnCloud 数据面进行一次真实对话。记录请求使用 model_id 和响应，不能访问裸 llama-server 代替。
4. 第二个兼容模型证明运行时缓存复用；覆盖刷新、取消、重试、服务重启、模型/运行包损坏恢复。每项给真实命令/输出，不以 HTTP202 或文件下载成功验收。
5. Windows NVIDIA 环境单独验证双包代理下载、VC++、驱动、cudart、实际设备使用和真实对话；CPU 通过不代表 CUDA 通过，未测加速不能标为可用。
6. 检查完整许可证、Windows 授权、macOS 签名/公证和体积报告；四基础目标与 CUDA 独立记录发布门禁。硬件/签名资源缺失时仅标未验证，不降低标准或伪造通过。

#### 8.2 工程与文档门禁

- 按任务实现后以及每次提交前/后执行 `cargo fmt`、`cargo clippy`、`cargo test`，明确所用 workspace/feature/target 与失败原因；Conventional Commits。单纯文档阶段尚未运行这些命令，不宣称测试通过。
- 新 stop_model_generation、既有下载 API 的变更语义及错误变体必须有独立测试和中文 `///`；关键模块提供 `# Examples` 文档测试。修复旧错误追加回归，不将测试与生产源码放同一文件。
- 同步根 README、server/download/node/client 对应 README（有公开变化者）、`server/src/api/openapi.rs`、平台/资源说明及接口类型文档。不得覆盖用户未跟踪的 hardware-model-resolution-api.md。
- 确认迁移两端均登记并真实执行；缺 PostgreSQL 资源时不能只凭 SQLite 通过宣称双端兼容。无 unsafe、外部输入无 unwrap/expect、不吞 Result，不为过检删功能/降校验。

本任务不新增或修改函数实现；仅覆盖上述表结构、固定资源、发行配置或说明文件，不为满足模板虚构函数。

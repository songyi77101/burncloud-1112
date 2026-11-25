```json
{
  "编号": "005",
  "依赖": []
}
```

# 005 登记固定运行包与可信资源库存

- 任务：登记固定运行包与可信资源库存
- 工作区：runtime-assets（读写）——deploy/runtime/manifest.json、deploy/runtime/README.md 及固定资源锁定清单格式
- 产出：六个固定源包登记、四目标选择与受审库存/派生摘要规则
- 前置任务：无
- 后置任务：006 实现发行环境固定资源准备工具
- 设计依据：第七节 7.1、7.2；第四节 RuntimeAsset；第三节 3.4
- 验收标准：
  - 四基础包与 Windows CUDA12.4 双包的版本、文件名、源大小、源 SHA 与 7.1 完全一致；auto 不暗含 CUDA，不登记未批准目标。
  - 清单能表达 source_sha256 与实际 embedded_sha256、展开上限、server 相对路径、文件/摘要/权限/链接、许可证及依赖证据；不以可改 ready.json 为信任根。
  - 签名/修正依赖后的包必须使用真实派生 SHA/大小/库存；未取得库存、授权或签名事实时明确阻断，不填可通过的占位值。
  - 大运行包仅进入 CI 显式资源根，不提交或覆盖用户 target 资源；CUDA 地址仅固定代理加两个已知文件名，不查询 GitHub API。

> 来源：开发序号5、用户于2026-11-24确认的 `../local-model-auto-deploy-dev-doc.md`、同目录流程图与接口契约；分支 `codex/local-model-auto-deploy`，工作区以 `../dev-context.md` 为准。任务定义不代表业务代码或实机验收已完成；实际执行次序只采用脚本生成的 `execution-order.md`。

## 范围与执行约束

- 不把 Gitee 源码镜像当作预编译发行包，不追踪最新版，不在此任务运行用户已有 llama.cpp。
- 只实现本任务摘录的定稿设计；新函数/方法/字段默认私有，跨文件和跨crate可见性不得超过开发文档第六节公开清单，不为测试导出内部实现。
- 注释与rustdoc使用中文；公共边界黑盒校验，内部公有函数白盒契约检查；私有函数不重复校验已保证参数，但新读取的网络/磁盘/归档/进程结果仍须验证。
- 测试写入独立文件；每个公有API/内部公有方法有独立测试，关键公共模块提供可编译# Examples，修复有回归；不使用unsafe、外部输入unwrap/expect或静默丢弃Result。
- 只写上述登记工作区和具体范围；project-root仅登记允许的Cargo.toml/Cargo.lock/README.md/本轮文档。不得覆盖用户target运行时、未跟踪server/docs文件、开发规范或dev-context.md。
- 编码/测试/提交状态由后续任务报告如实记录；提交使用Conventional Commits，提交前后均执行cargo fmt、cargo clippy、cargo test并记录实际feature/target与结果。真实硬件、许可、签名或测试资源缺失不填假证据。

## 实现内容

### 数据结构

- 名称：RuntimeAsset
- 类型：只读发行清单项
- 说明：编入构建，不能由部署请求覆盖。
- 结构：

```python
class RuntimeAsset:
    version: str  # 共享；固定 b8581
    target: str  # 共享；精确 Rust target
    package_kind: str  # 共享；cpu/native/cuda12.4
    filename: str  # 共享；固定归档名
    size_bytes: u64  # 独占拥有；压缩字节数
    sha256: Sha256  # 独占拥有；实际内嵌/下载包摘要，签名派生时不是源摘要
    source_sha256: Sha256  # 独占拥有；7.1 固定上游源包摘要，供溯源
    unpacked_bytes: u64  # 独占拥有；受审归档展开上界
    server_relative_path: str  # 共享；清单定位而非搜索系统 PATH
    download_url: Optional[str]  # 共享；CUDA 代理路径；基础内嵌不在线取包
    bundled_bytes: Optional[StaticBytes]  # 共享；只读内嵌压缩资源
    inventory: ArchiveInventory  # 共享；受审路径/大小/摘要/权限/链接库存
```

### 实现约束原文摘录

#### 7.1 固定源包登记

以下数值来自前序阶段已核对的 b8581 发行资产记录；它们是源包字节数与 SHA-256，不是 BurnCloud 最终发行体积，也不是四平台实机验收证明。发行脚本必须再次对实际取得字节核对，任何不符均失败，不自行更新为“最新摘要”。

| 目标/用途 | 固定源包 | 字节数 | SHA-256 |
| --- | --- | ---: | --- |
| Windows x64 CPU | llama-b8581-bin-win-cpu-x64.zip | 38707858 | 71fffc57588b12142a1f0b37f8a1f0a1b122d14f70435f3689dd6399ba4317e7 |
| macOS ARM64 原生 | llama-b8581-bin-macos-arm64.tar.gz | 39574927 | 0c79d0c1370c81137d58cdc720881dfbab67cae02769d9eaf895f127d0785056 |
| macOS x64 原生 | llama-b8581-bin-macos-x64.tar.gz | 100650354 | e03188ae51dc1c3ba8d2a5d6220ac0d6639a47db64af4d0c93bc05b494a98d0b |
| Ubuntu 24.04 x64 CPU | llama-b8581-bin-ubuntu-x64.tar.gz | 31371639 | 8424fecb5a260f15a3aa90016a47242621881b78dc5ebb4cb418d0134d5cbe43 |
| Windows CUDA 12.4 服务 | llama-b8581-bin-win-cuda-12.4-x64.zip | 232525426 | 0be0ef75d1544e836ba8e90c4fe02cad253cb8983ebcb68ac774183a28f400d5 |
| Windows CUDA 12.4 cudart | cudart-llama-bin-win-cuda-12.4-x64.zip | 391443627 | 8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6 |

CUDA 固定客户端下载前缀为 `https://gh-proxy.com/https://github.com/ggml-org/llama.cpp/releases/download/b8581/`，仅拼上述两个已知文件名；不访问 GitHub API。代理不可用或返回错误页必须显式失败且保留安全断点，不承诺该第三方永久可用，也不临时用 Gitee 源码仓库冒充预编译运行包。

#### 7.2 构建输入与签名封装

- 根 Cargo 增加 `bundled-runtime = ["burncloud-server/bundled-runtime"]`，server 增加同名 feature 和 build.rs。普通开发默认不启用，官方发行必须启用。
- `deploy/runtime/manifest.json` 登记固定源包版本、精确目标、压缩 SHA/大小、展开上限、server 相对路径、依赖及许可清单；`deploy/runtime/README.md` 说明获取/校验和再分发流程。大运行包放 CI 明确资源目录，不提交用户本机资源，不扩展根目录写入范围。
- 发行资源阶段使用 `deploy/scripts/prepare_runtime_assets.py`；构建通过 `BURNCLOUD_RUNTIME_ASSET_DIR` 显式指定已准备资源根。build.rs 仅读文件和校验，不包含 HTTP、curl、包管理器或网络回退。无资源、错误 target/version/name/长度/SHA/许可记录即构建失败。
- 基础包签名封装与源包校验分开：若 macOS 源包不能直接满足签名/公证要求，发行环境先核验源包，再对包含的 Mach-O/dylib 按由内到外顺序签名，制作本平台派生 tar.gz，并将签名、公证/验证证据与派生包 SHA 固定到本次资源锁定清单。不能在用户端签名、删除隔离属性，也不能重签后继续沿用源包 SHA。
- 发布资源阶段在目标基线上审计动态依赖。Ubuntu 基线以外但可合法再分发的必要 so 由发行环境并入同一基础包，配置 `$ORIGIN`/相邻 lib 等相对加载路径；macOS 相对加载元数据修正必须先于签名。不能等用户首次部署再要求 apt/Homebrew/编译器补依赖，不能打包替换系统 glibc。处理后的包同样登记派生摘要、完整库存与许可证，并用干净系统实际执行探测后才能发行。
- 资源锁定清单包含 source_sha256、embedded_sha256、压缩大小、展开库存和版本/target，随发行记录归档；build.rs 对清单与实际唯一基础包做一致性检查，将实际 embedded_sha256 及只读库存一并编入客户端。无需派生时两摘要相等；macOS 合法重封装时 RuntimeAsset.sha256 指实际内嵌包摘要，源摘要另保留为溯源证据。CUDA 在线包仍使用 7.1 的固定下载摘要，不接受客户端替换。
- Windows 的微软 VC++ x64 安装程序作为独立受审依赖资源随本平台程序内嵌，摘要/签名/授权依据单独锁定。不把不含 VC++ 的上游 ZIP 当作依赖齐全；授权、固定安装程序摘要或签名证据不具备时官方 Windows 发行失败，不填占位假摘要。
- 运行时资源和依赖遵循 b8581/平台固定选择。macOS 重签重封装不是更新 llama.cpp 版本或增加第二个平台包；源包大小与派生包大小的差异记入体积报告。
- 未填写的发行事实不是待选架构：设计确定为“缺证据阻断”。展开库存、签名凭据、授权文件和实机证据必须从真实准备步骤取得；本开发文档不伪造它们，也不提供跳过门禁开关。

本任务不新增或修改函数实现；仅覆盖上述表结构、固定资源、发行配置或说明文件，不为满足模板虚构函数。

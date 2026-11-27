# 027 原生 Gitee 运行时下载与断点恢复

## 修改的信息

- 新增独立的原生 `reqwest` 运行时归档传输，不复用模型下载的 aria2 任务，也不改变现有 GGUF 下载路径。
- 固定来源为 Gitee 仓库 `songyi777/llama-cpp` 与 commit `51d01f6e526bc6d45b9d252b1d2d487efb6625d8`；四个平台只允许对应 manifest 文件名，调用方不能传入任意 URL，重定向不跟随。
- 运行时归档通过 response stream 写入 `.part`。JSON sidecar 固定记录 provider、repository、commit、target、文件名、URL、期望长度与 SHA-256、强 ETag 及已同步连续字节数。
- 续传要求本地 sidecar 身份、连续长度、强 ETag、Range 起点、Content-Range 起点与总长度一致；可选 Content-Length 若无效或不匹配，则拒绝追加并回到从零下载。额外先验证范围算术，避免不可信 Content-Range 导致整数下溢。
- `.part` 在 sidecar 提交前先 `sync_data`；取消与连接中断保留有强 ETag 证明的连续前缀，无强 ETag 时清空不可恢复内容。重试次数、连接等待、无进展等待均有限。
- 完成后重新读取文件长度并流式重算 SHA-256，校验成功才提升为 `VerifiedArchive`；校验失败不生成已验证归档。缓存命中同样重新检查大小与摘要。
- 修正真实进度：开始请求前发布现有断点长度，实体变化、重下或校验失败时发布回退后的进度；速率仅按当前传输尝试新增的字节计算。
- 更新模块 rustdoc 与 README，明确常规 aria2 下载和运行时 HTTP 下载的职责边界、服务端路径要求及错误码使用方式。

## 修改的文件与文件内位置

- `crates/platform/storage/download/Cargo.toml`：增加 SHA-256、流处理、sidecar、运行时与测试依赖。
- `crates/platform/storage/download/src/lib.rs`：更新 crate 级职责说明，导出运行时归档下载所需接口和数据类型。
- `crates/platform/storage/download/src/runtime_http.rs`：新增 `RuntimeArchive`、`RuntimePartialMetadata`、`VerifiedArchive`、`RuntimeDownloadProgress` 与 `RuntimeDownloadError`；实现固定 URL、流式下载、Range/ETag 判断、断点同步与恢复、取消、有限重试、摘要校验和进度发布。
- `crates/platform/storage/download/src/runtime_http_tests.rs`：新增分离的运行时下载测试，含四目标身份、响应头校验、全量下载、正确续传、200 重下、状态错误、摘要失败、活动下载取消以及模拟连接中断后的 Range 恢复。
- `crates/platform/storage/download/README.md`：区分模型 aria2 下载与 llama.cpp 运行时原生 HTTP 传输，并说明安全边界与进度/错误语义。
- `docs/dev-docs/5/task-report/027-runtime-http-download.md`：本任务报告。

## 检查结果

| 验收项 | 结果 |
| --- | --- |
| 固定 commit、四目标文件名与服务端大小/SHA 身份 | 通过；身份构造拒绝未知 target/文件名，四个已登记 target 均有单测。 |
| response stream 与 aria2 隔离 | 通过；实现由 reqwest `bytes_stream` 逐块写入 `.part`，未接入 aria2 下载流程。 |
| 强 ETag + Range/Content-Range 续传 | 通过；匹配时追加；范围异常、ETag 变化或 200 响应时不拼接旧实体并从头请求。 |
| 错误 Content-Length 修正 | 通过；无效值不会再被视为头缺失；新增回归测试覆盖解析层，实现将该响应标为续传不匹配并全量重下。 |
| 连接中断、取消与重试 | 通过；本地 TCP 测试模拟 chunked 响应中断，验证自动重试携带最后同步的 Range/If-Range；活动取消测试验证 `.part` 与 sidecar 只保留已同步前缀。 |
| 固定来源错误处理 | 通过；403、404 和 302 映射为来源不可用，不跟随重定向、不回退其他来源。 |
| 文件长度与 SHA-256 | 通过；全量下载按块哈希，摘要错误有限重试且不提升坏包；有效归档可从缓存命中。 |
| 进度和错误码 | 通过；覆盖续传初始进度、实体变化回退代码路径与稳定错误码映射。 |
| 格式与静态检查 | `cargo fmt --all -- --check` 通过；`cargo clippy --workspace --all-targets -j 1` 通过。 |
| 下载模块测试 | `cargo test -p burncloud-download -j 1` 通过：37 个单测、2 个集成测试目标（共 3 项断言，含错误来源和公开面测试）、20 个 doctest 均通过。 |
| 全工作区测试 | `cargo test --workspace -j 1` 未通过：全工作区测试执行到 `burncloud-node-runtime` doctest 时，37/49 个既有文档示例因 Windows `windows_implement` / `windows_interface` staticlib 与 rlib 元数据不匹配（`E0462`、`E0460`）失败；失败位置不在本任务 download 模块。未修改无关节点文档或构建配置。 |
| 真实 Gitee 与真实模型部署 | 未在本任务执行；当前环境以本地 HTTP/TCP 测试验证协议行为。Gitee 资源实下载与各平台真实服务验收仍属后续任务 036。 |

## 后续边界

- 缓存根目录策略、跨线程/跨进程安装锁、共享准备订阅及新旧版本原子切换由任务 028/031 接入。
- 安全 ZIP/TAR.GZ 解压由任务 029 实现；服务端进程与 backend 能力探测由任务 030 实现。
- 本任务不表示 Windows、macOS 或 Ubuntu 已完成真实部署验收。

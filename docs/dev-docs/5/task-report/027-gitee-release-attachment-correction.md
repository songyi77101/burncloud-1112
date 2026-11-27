# 027 Gitee Release 附件下载修订与实测补充

> 补充记录：保留原 `027-runtime-http-download.md` 的历史实现结果；本报告记录用户发布 `b12757` Release 附件后，对 raw URL 下载失败问题的修订和实测，不将其计作任务 034、035 或 036 全部完成。

## 修订原因

原下载器请求固定 commit 的 Gitee raw 大文件地址时，匿名访问被 Gitee 返回 `403 large file require login for access`。用户已提供 `b12757` Release 标签及其附件；Release 附件可由匿名 HTTPS 下载，因此将生产下载入口改为固定 Release URL，并继续以服务端 manifest 的固定字节数和 SHA-256 校验归档实体。

下载地址形态：

```text
https://gitee.com/songyi777/llama-cpp/releases/download/b12757/<manifest-file-name>
```

代码仍固定来源仓库和 commit，不接收客户端 URL、版本、SHA、文件名或解压路径。仅允许 HTTPS `gitee.com` 仓库附件路径和 `foruda.gitee.com` 附件路径，路径文件名必须与目标资源一致，最多跟随两次重定向；不启动 aria2、不回退到其他来源。

## 实现内容

- `crates/platform/storage/download/src/runtime_http.rs`：改用固定 Release 标签生成下载 URL；增加严格 HTTPS 附件重定向 allowlist 和跳数限制；保留流式下载、断点续传、大小与 SHA-256 校验。
- `crates/platform/storage/download/src/runtime_http_tests.rs`：增加 Release URL、允许/拒绝的附件 URL、最终 response URL 与重定向跳数边界测试；增加需要人工触发的真实四目标下载测试，并在错误时输出归档名和不匹配详情。
- `deploy/runtime/manifest.json` 与 `crates/interfaces/server/src/node_runtime/manifest.rs`：可信来源模板使用固定 Release 标签；保留 commit、四目标资源身份、字节数和摘要校验。
- `crates/platform/storage/download/README.md`、`deploy/runtime/README.md`、下载 API rustdoc：同步 Release 附件来源、受限重定向和当前实测边界。

## 下载实测

执行日期：2026-11-27。真实网络测试通过 Rust `reqwest` 下载器下载了四个 Release 附件，代码按 manifest 重新校验大小与完整 SHA-256：

| 目标 | 归档大小（字节） | SHA-256 |
| --- | ---: | --- |
| Windows x64 | 25,262,139 | `5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d` |
| macOS ARM64 | 16,862,081 | `873b413c5d4a336ae623bedde8f755a3ed81b18bc6ae4b01f5a0f6fcea63754c` |
| macOS x64 | 12,591,660 | `f330d7bf9baff5cc401c06524618f9eaf8962439dcb9e11fa638f8e3b0bee14b` |
| Ubuntu x64 | 23,425,964 | `99e34cc6a39f3afda735d43d76d15d6ead2297406cc9c0cf421b771fd3a1a84a` |

Windows x64 经项目 `LlamaCppRuntimePreparer` 在 Release 实际下载后完成大小/SHA 校验、解压、`--version`/`--help` 探测及缓存原子提交；第二次 prepare 返回相同可执行文件并标记 `runtime_cache_reuse=verified`。manifest 指定路径 `llama-server.exe` 存在；探测输出：

```text
version: 1.1.0-dev (build 12757, commit 2d5b3ddde)
built with Clang 22.1.8 for Windows x86_64
--version exit: 0
--help exit: 0
```

## 检查结果

- `cargo fmt --all -- --check`：通过。
- `cargo test -p burncloud-download --lib -j 1 -- --test-threads=1`：41 passed、0 failed、1 ignored（真实网络测试单独运行）。
- `cargo test -p burncloud-download gitee_release_artifacts_download_and_match_manifest_hashes --lib -j 1 -- --ignored --nocapture --test-threads=1`：通过；四个 Release 附件均完成下载和 manifest 大小/SHA-256 校验。
- `cargo clippy -p burncloud-download --all-targets -j 1 -- -D warnings`：通过。
- `cargo test -p burncloud-server live_gitee_runtime_preparer_downloads_extracts_probes_and_commits_cache --lib -j 1 -- --ignored --nocapture --test-threads=1`：1 passed；真实执行项目运行时准备器完成下载、校验、解压、探测、ready 提交，并在同一测试中通过第二次缓存复用。测试缓存由 `BURNCLOUD_RUNTIME_CACHE_DIR` 指向临时目录；另一次未覆盖默认目录的成功 prepare 生成 `%LOCALAPPDATA%\BurnCloud\runtimes` 下的 Windows ready 缓存。

## 未完成与支持门禁

- 本机可见的 `q4_k_m-mock-q4_k_m.gguf` 为 8 MiB 测试占位文件，文件头不是 GGUF magic；当前没有可用于真实推理的有效模型。因此未执行服务加载、`/health`、`/v1/models`、`/slots`、真实对话或流式对话验收。
- 当前机器是 Windows x64；macOS Intel、macOS Apple Silicon、Ubuntu 包仅完成下载与归档摘要校验，未在对应操作系统解压、启动或做设备/backend 探测。
- 因此本报告只证明真实 Gitee 下载链路和 Windows 二进制基本探测成功，不代表四平台部署通过；按任务 036 的门禁，未完成目标环境验收的平台必须保持“未验收”。
## 补充环境检查

- `cargo test -p burncloud-server node_runtime::manifest --lib -j 1 -- --test-threads=1`：5 passed。
- `cargo clippy -p burncloud-server --all-targets -j 1 -- -D warnings`：通过。
- WSL 仅列出 `docker-desktop` 发行版（非 Ubuntu）；Docker CLI 无法连接 `dockerDesktopLinuxEngine`，因此没有可用的 Ubuntu 目标环境。
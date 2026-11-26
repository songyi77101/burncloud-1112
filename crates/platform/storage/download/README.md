# 下载管理

`DownloadManager` 管理模型等常规下载任务，并继续通过既有 aria2 流程执行；本次增加的 Gitee llama.cpp 运行时传输是彼此独立的原生 HTTP 通道，不会替换或改变模型 GGUF 的下载行为。

## 运行时归档下载

- `download_runtime_archive` 只接受服务端 manifest 中四个已登记目标的固定归档身份，并根据代码固定的仓库与 commit 生成 Gitee raw 地址；调用方不能提交 URL、commit 或重定向目标。
- 下载器使用 `reqwest` response stream 将内容写入运行时下载目录的 `.part` 文件，不启动 aria2，也不把整个压缩包载入内存。
- 断点 sidecar 只记录固定资源身份、强 ETag 和已同步的连续字节数。只有 Range 起点、Content-Range、实体 ETag、总长度和本地前缀一致时才续传；不一致时先清理旧实体，再从头下载。
- 取消或连接中断时，带强 ETag 的有效连续前缀会同步后保留；无强 ETag 时不保留可恢复断点。下载完成还会重新校验 manifest 字节数和完整 SHA-256，校验通过前不会提升为已验证归档。
- `RuntimeDownloadProgress` 通过 `tokio::sync::watch` 发布；续传时初始值反映已同步前缀，实体变化或校验重试时允许回退到零。
- 运行时下载错误提供稳定 `code()`，供自动部署任务映射为既有错误响应；对外提示不应暴露原始 URL、ETag 或本地绝对路径。

运行时缓存布局、共享准备作业、跨进程安装锁和解压探测由后续部署任务负责。下载器要求调用方传入由服务端缓存管理器解析出的绝对运行时下载目录，不能直接使用 HTTP 请求中的路径。

## 常规下载错误

`DownloadError` 保留已有错误变体（包括 `Aria2`），新增两种有来源的错误：

- `Transport(reqwest::Error)`：常规下载 HTTP 请求或响应传输出错。
- `Io(std::io::Error)`：下载文件或断点侧车读写、同步、替换失败。

可用 `std::error::Error::source` 读取底层错误。现有模型下载断点与任务行为保持不变。

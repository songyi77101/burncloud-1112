# 下载管理

`DownloadManager` 管理可持久化的下载任务。当前下载门面仍使用既有 aria2；原生 HTTP 传输接入前，断点检查点逻辑不会改变现有任务行为。

`DownloadError` 保留已有错误变体（包括 `Aria2`），新增两种有来源的错误：

- `Transport(reqwest::Error)`：HTTP 请求或响应传输出错。对外提示不应包含原始地址或签名参数。
- `Io(std::io::Error)`：下载文件或断点侧车读写、同步、替换失败。

可用 `std::error::Error::source` 读取底层错误。新断点只记录已同步的连续文件长度；恢复时仍须匹配强 ETag 和实体，侧车不替代完整文件的 SHA-256 校验。

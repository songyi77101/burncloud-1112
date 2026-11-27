# 执行顺序

> 由 `scripts/topo_sort.py` 对任务文件的「依赖」做拓扑排序生成，禁止手工排序。

1. 026 登记方案二四目标 Gitee 运行时资源和可信字段校验。
2. 027 实现不依赖 aria2 的 reqwest 流式运行时归档下载、`.part` sidecar、Range/ETag 恢复、取消和有限重试。
3. 028 实现用户数据目录缓存布局、`BURNCLOUD_RUNTIME_CACHE_DIR`、key 隔离、跨线程/跨进程安装锁和共享订阅。
4. 029 实现运行时归档的路径、链接和展开资源限制，并将合法内容写入同文件系统 staging。
5. 030 在安全 staging 或显式路径上探测 `llama-server` 身份、依赖、版本和目标 backend 能力。
6. 031 将 manifest、下载、缓存、解压、探测和原子提交串成 `LlamaCppRuntimePreparer`，返回既有 `PreparedRuntime` 并复用部署服务生命周期。
7. 032 将运行时共享准备事件接入现有自动部署任务快照、POST/GET/DELETE 接口和 resolve 预算，不新增 runtime HTTP API。
8. 033 使现有自动部署客户端消费方案二 RuntimeProgress，不把运行时共享作业当作独立可取消任务。
9. 034 覆盖 manifest、HTTP 断点、缓存并发、安全解压、探测、接口进度和取消共享语义。
10. 035 同步方案二实现涉及的公共 rustdoc、模块 README、根 README、运行时资源说明和 OpenAPI/接口类型文档。
11. 036 在四个目标环境完成运行期 Gitee 运行时下载、缓存复用和真实对话验收，生成可追溯证据。

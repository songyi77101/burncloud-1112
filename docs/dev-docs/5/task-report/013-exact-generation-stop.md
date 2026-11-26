# 013 补充原子精确代次停止接口

## 修改的信息

- 节点新增 `DeploymentService::stop_model_generation(&str, u64) -> Result<bool, DeploymentError>`；空白标识及零代次拒绝，不存在或已换代返回 `false`。旧 `stop_model` 保留按名称停止的入口。
- 在同一 coordinator 临界区核对代次、取得独占停止槽位；成功发布停止进度后，同一次加锁内再次核对并移走精确监控句柄。槽位保护同模型替换直至停止事件或失败恢复结算，期间不持全局锁等待进程、监控或事件。调用方取消等待不会中断已取得槽位的结算。
- 停止进度发布失败时继续原监控；进程停止失败时重新核对存活。仍活着的进程以原代次和原 PID 恢复监控，确认已经退出则不报告保留就绪状态。成功停止事件标记实际停止的代次。

## 修改的文件与文件内位置

- `crates/platform/node/src/deployment/service.rs`：`StopReservation`、`StopMode`、两个停止入口、`stop_old_runtime`、`stop_reserved_runtime`、`publish_stopped`、`recover_failed_stop`。
- `crates/platform/node/tests/deployment_unit.rs`、`crates/platform/node/tests/deployment_unit/generation_stop.rs`：注册并覆盖独立 API 和并发回归。
- `crates/platform/node/README.md`：「按代次停止本机模型」记录调用契约和结算行为。

## 检查结果

| 验收项 | 结果 |
| --- | --- |
| 非法参数、无句柄、代次失配、停止事件精确归属 | 独立测试覆盖，核对事件 PID、端点、代次及重复停止无副作用。 |
| 与替换竞争、调用方取消及旧入口 | 门控测试覆盖两种槽位顺序，替换等待终态；旧入口仍可按名称停止替换后实例。 |
| 停止失败及发布失败 | 回归测试核对同代次心跳、同 PID 重试、进度发布失败保留原监控周期，以及进程已退出时不声称仍就绪。 |
| 文档及可见性 | 唯一新增跨 crate 方法，含中文 rustdoc 和文档测试、同步节点 README。 |

- 验证目标为 Windows x64，使用 `--no-default-features`。本任务只核对节点停止契约，不代表发行包、干净新机或聊天页真实推理通过验收。
- 提交前：`cargo fmt --all -- --check` 和 `cargo clippy -p burncloud-node-runtime --all-targets --no-default-features -- -D warnings` 通过；节点单元和集成测试通过（含 6 项新增回归，1 项已有手动用例忽略），49 项文档测试通过。
- 全工作区首跑在既有 Vertex 测试 `adaptor::vertex::tests::test_overrides` 失败：同邮箱的测试共享令牌缓存，模拟 `/auth` 收到 0 次请求。上一版与本版路由测试二进制均可在串行模式复现；未改路由测试或跳过门禁。
- 原门禁并行复跑（`RUST_TEST_THREADS=4`）：全工作区 2054 通过、0 失败、147 忽略，严格 Clippy 通过。该次提交因 RustSec 下载连接被重置，未落地；随后重新从上游刷新 RustSec，并单独运行 `cargo deny check` 通过。

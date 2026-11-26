# BurnCloud Node Runtime Skeleton

This crate owns only local machine/runtime orchestration primitives.

## Reconcile rail

```text
Absent
  -> Resolving
  -> PreparingArtifact
  -> ArtifactReady
  -> PreparingRuntime
  -> Starting
  -> WaitingReady
  -> Ready
  -> Routable
```

`Starting` means a process is being spawned. `WaitingReady` means a process exists but readiness has not been proven. `Ready` requires explicit readiness evidence. A fixed sleep is not readiness evidence.

## Boundary

The reconciler may request actions, but it does not own the business implementation of model resolution, artifact selection, Provider selection, or routing policy. Those contracts and decisions must remain with their owning BurnCloud domains.

The Node runtime must not create a second HTTP server, listener, or ModelRouter. `Routable` means attachment to the existing BurnCloud routing path.

## 按代次停止本机模型

从部署事件取得模型标识与进程代次后，调用 `DeploymentService::stop_model_generation(model_id, generation)`。
只有匹配的受管进程才会被停止；进程不存在或已经换代时返回 `Ok(false)`。空白模型标识或零代次返回 `InvalidRequest`。
一旦取得停止槽位，即使调用方不再等待，停止也会继续结算。同一模型的替换必须等这次结算结束；
如果停止失败而进程仍活着，节点会继续以原代次监控该进程。只需要按名称停止当前进程时，仍可使用 `stop_model`。

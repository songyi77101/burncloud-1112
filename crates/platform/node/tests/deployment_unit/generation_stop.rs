use std::{sync::Arc, time::Duration};

use burncloud_node_runtime::{
    deployment::{DeploymentErrorKind, DeploymentEvent, DeploymentEventKind, DeploymentStage},
    NodeState,
};

use super::support::{
    deploy, new_service, plan, FixedAdapter, Gate, RecordingPublisher, ScriptedHealthProbe,
    ScriptedProcessManager, ScriptedReadinessProbe,
};

const MODEL: &str = "model-exact-stop";
const STOPPING: DeploymentEventKind =
    DeploymentEventKind::Progress(DeploymentStage::StoppingOldRuntime);

async fn initial_event(publisher: &RecordingPublisher) -> DeploymentEvent {
    publisher
        .received
        .lock()
        .await
        .iter()
        .rev()
        .find(|event| event.kind == DeploymentEventKind::InitialResult(NodeState::Ready))
        .cloned()
        .unwrap()
}

#[tokio::test]
async fn exact_stop_validates_and_only_emits_a_stop_for_the_matching_generation() {
    let processes = Arc::new(ScriptedProcessManager::new());
    let publisher = Arc::new(RecordingPublisher::default());
    let (service, _errors) = new_service(
        Arc::clone(&processes),
        Arc::new(ScriptedReadinessProbe::default()),
        Arc::new(ScriptedHealthProbe::default()),
        Arc::clone(&publisher),
    );

    for (model, generation) in [(" \t", 1), (MODEL, 0)] {
        let error = service
            .stop_model_generation(model, generation)
            .await
            .unwrap_err();
        assert_eq!(error.kind, DeploymentErrorKind::InvalidRequest);
    }
    assert!(!service.stop_model_generation(MODEL, 1).await.unwrap());
    assert!(publisher.attempts.lock().await.is_empty());

    let outcome = deploy(&service, MODEL, Arc::new(FixedAdapter::new(plan())))
        .await
        .unwrap();
    let initial = initial_event(&publisher).await;
    assert!(!service
        .stop_model_generation(MODEL, initial.generation + 1)
        .await
        .unwrap());
    assert_eq!(processes.stop_count().await, 0);
    assert!(service
        .stop_model_generation(MODEL, initial.generation)
        .await
        .unwrap());
    assert!(!service
        .stop_model_generation(MODEL, initial.generation)
        .await
        .unwrap());
    assert_eq!(&*processes.stops.lock().await, &[outcome.process]);

    let events = publisher.received.lock().await;
    let stopped: Vec<_> = events
        .iter()
        .filter(|event| event.kind == DeploymentEventKind::Stopped)
        .collect();
    assert_eq!(stopped.len(), 1);
    assert_eq!(stopped[0].generation, initial.generation);
    assert_eq!(stopped[0].pid, initial.pid);
    assert_eq!(stopped[0].local_endpoint, initial.local_endpoint);
    assert_eq!(stopped[0].state, Some(NodeState::Absent));
}

#[tokio::test]
async fn replacement_claimed_first_cannot_be_stopped_by_the_previous_generation() {
    let processes = Arc::new(ScriptedProcessManager::new());
    let publisher = Arc::new(RecordingPublisher::default());
    let (service, _errors) = new_service(
        Arc::clone(&processes),
        Arc::new(ScriptedReadinessProbe::default()),
        Arc::new(ScriptedHealthProbe::default()),
        Arc::clone(&publisher),
    );
    let old = deploy(&service, MODEL, Arc::new(FixedAdapter::new(plan())))
        .await
        .unwrap();
    let old_generation = initial_event(&publisher).await.generation;
    let gate = Arc::new(Gate::default());
    publisher.block_kind(STOPPING, Arc::clone(&gate)).await;
    let service = Arc::new(service);
    let replacing = Arc::clone(&service);
    let replacement = tokio::spawn(async move {
        deploy(&replacing, MODEL, Arc::new(FixedAdapter::new(plan())))
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(5), gate.wait_until_entered())
        .await
        .unwrap();

    let stale_stop = service.stop_model_generation(MODEL, old_generation);
    tokio::pin!(stale_stop);
    tokio::select! {
        biased;
        result = &mut stale_stop => panic!("替换已持有停止槽位：{result:?}"),
        () = tokio::task::yield_now() => {}
    }
    gate.open();
    let new = tokio::time::timeout(Duration::from_secs(5), replacement)
        .await
        .unwrap()
        .unwrap();
    assert!(!stale_stop.await.unwrap());
    assert_ne!(initial_event(&publisher).await.generation, old_generation);
    assert_eq!(&*processes.stops.lock().await, &[old.process]);

    // 按名称停止仍可命中新代次，并继续提供幂等结果。
    assert!(service.stop_model(MODEL).await.unwrap());
    assert!(!service.stop_model(MODEL).await.unwrap());
    assert_eq!(&*processes.stops.lock().await, &[old.process, new.process]);
    let new_generation = initial_event(&publisher).await.generation;
    assert!(publisher.received.lock().await.iter().any(|event| {
        event.kind == DeploymentEventKind::Stopped
            && event.generation == new_generation
            && event.pid == Some(new.process.pid)
    }));
}

#[tokio::test]
async fn cancelled_stop_waiter_does_not_let_replacement_pass_the_stop_event() {
    let processes = Arc::new(ScriptedProcessManager::new());
    let publisher = Arc::new(RecordingPublisher::default());
    let (service, _errors) = new_service(
        Arc::clone(&processes),
        Arc::new(ScriptedReadinessProbe::default()),
        Arc::new(ScriptedHealthProbe::default()),
        Arc::clone(&publisher),
    );
    let old = deploy(&service, MODEL, Arc::new(FixedAdapter::new(plan())))
        .await
        .unwrap();
    let old_generation = initial_event(&publisher).await.generation;
    let stop_event = Arc::new(Gate::default());
    publisher
        .block_kind(DeploymentEventKind::Stopped, Arc::clone(&stop_event))
        .await;
    let service = Arc::new(service);
    let stopping = Arc::clone(&service);
    let waiter =
        tokio::spawn(async move { stopping.stop_model_generation(MODEL, old_generation).await });
    tokio::time::timeout(Duration::from_secs(5), stop_event.wait_until_entered())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());

    let planning_gate = Arc::new(Gate::default());
    publisher
        .block_kind(
            DeploymentEventKind::Progress(DeploymentStage::Planning),
            Arc::clone(&planning_gate),
        )
        .await;
    let replacing = Arc::clone(&service);
    let replacement = tokio::spawn(async move {
        deploy(&replacing, MODEL, Arc::new(FixedAdapter::new(plan())))
            .await
            .unwrap()
    });
    assert!(tokio::time::timeout(
        Duration::from_millis(50),
        planning_gate.wait_until_entered()
    )
    .await
    .is_err());
    assert_eq!(processes.start_count().await, 1);
    assert_eq!(&*processes.stops.lock().await, &[old.process]);

    stop_event.open();
    tokio::time::timeout(Duration::from_secs(5), planning_gate.wait_until_entered())
        .await
        .unwrap();
    planning_gate.open();
    let new = tokio::time::timeout(Duration::from_secs(5), replacement)
        .await
        .unwrap()
        .unwrap();
    let new_generation = initial_event(&publisher).await.generation;
    {
        let events = publisher.received.lock().await;
        let stopped = events
            .iter()
            .position(|event| {
                event.kind == DeploymentEventKind::Stopped && event.generation == old_generation
            })
            .unwrap();
        let planning = events
            .iter()
            .position(|event| {
                event.kind == DeploymentEventKind::Progress(DeploymentStage::Planning)
                    && event.generation == new_generation
            })
            .unwrap();
        assert!(stopped < planning);
    }
    assert!(!service
        .stop_model_generation(MODEL, old_generation)
        .await
        .unwrap());
    assert!(service
        .stop_model_generation(MODEL, new_generation)
        .await
        .unwrap());
    assert_eq!(&*processes.stops.lock().await, &[old.process, new.process]);
}

#[tokio::test(start_paused = true)]
async fn exact_stop_failure_keeps_the_same_live_generation_and_allows_retry() {
    let processes = Arc::new(ScriptedProcessManager::new());
    let publisher = Arc::new(RecordingPublisher::default());
    let (service, _errors) = new_service(
        Arc::clone(&processes),
        Arc::new(ScriptedReadinessProbe::default()),
        Arc::new(ScriptedHealthProbe::default()),
        Arc::clone(&publisher),
    );
    let old = deploy(&service, MODEL, Arc::new(FixedAdapter::new(plan())))
        .await
        .unwrap();
    let generation = initial_event(&publisher).await.generation;
    *processes.stop_error.lock().await = Some("stop failed".into());
    let error = service
        .stop_model_generation(MODEL, generation)
        .await
        .unwrap_err();
    assert_eq!(error.kind, DeploymentErrorKind::OldRuntimeStopFailed);
    assert_eq!(error.preserved_state, Some(NodeState::Ready));

    let heartbeat = Arc::new(Gate::default());
    let heartbeat_kind = DeploymentEventKind::Heartbeat(NodeState::Ready);
    publisher
        .block_kind(heartbeat_kind, Arc::clone(&heartbeat))
        .await;
    tokio::time::timeout(Duration::from_secs(11), heartbeat.wait_until_entered())
        .await
        .unwrap();
    assert!(publisher.attempts.lock().await.iter().any(|event| {
        event.kind == heartbeat_kind
            && event.generation == generation
            && event.pid == Some(old.process.pid)
    }));
    heartbeat.open();
    assert!(service
        .stop_model_generation(MODEL, generation)
        .await
        .unwrap());
    assert_eq!(&*processes.stops.lock().await, &[old.process, old.process]);
}

#[tokio::test(start_paused = true)]
async fn failure_to_publish_stop_progress_leaves_the_original_monitor_running() {
    let processes = Arc::new(ScriptedProcessManager::new());
    let publisher = Arc::new(RecordingPublisher::default());
    let (service, _errors) = new_service(
        Arc::clone(&processes),
        Arc::new(ScriptedReadinessProbe::default()),
        Arc::new(ScriptedHealthProbe::default()),
        Arc::clone(&publisher),
    );
    deploy(&service, MODEL, Arc::new(FixedAdapter::new(plan())))
        .await
        .unwrap();
    let generation = initial_event(&publisher).await.generation;
    // 在监控周期的中途触发故障；下一次心跳必须使用原来的计时，不得重启监控。
    tokio::time::sleep(Duration::from_secs(5)).await;
    publisher.fail_kind(STOPPING, 2).await;
    let error = service
        .stop_model_generation(MODEL, generation)
        .await
        .unwrap_err();
    assert_eq!(error.kind, DeploymentErrorKind::EventPublishFailed);
    assert_eq!(error.preserved_state, Some(NodeState::Ready));
    assert_eq!(processes.stop_count().await, 0);
    let heartbeat = Arc::new(Gate::default());
    publisher
        .block_kind(
            DeploymentEventKind::Heartbeat(NodeState::Ready),
            Arc::clone(&heartbeat),
        )
        .await;
    tokio::time::timeout(Duration::from_secs(6), heartbeat.wait_until_entered())
        .await
        .unwrap();
    heartbeat.open();
    assert!(service
        .stop_model_generation(MODEL, generation)
        .await
        .unwrap());
}

#[tokio::test]
async fn exact_stop_error_does_not_report_a_dead_process_as_preserved() {
    let processes = Arc::new(ScriptedProcessManager::new());
    let publisher = Arc::new(RecordingPublisher::default());
    let (service, _errors) = new_service(
        Arc::clone(&processes),
        Arc::new(ScriptedReadinessProbe::default()),
        Arc::new(ScriptedHealthProbe::default()),
        Arc::clone(&publisher),
    );
    deploy(&service, MODEL, Arc::new(FixedAdapter::new(plan())))
        .await
        .unwrap();
    let generation = initial_event(&publisher).await.generation;
    *processes.stop_error.lock().await = Some("process exited during stop".into());
    processes.liveness.lock().await.push_back(Ok(false));
    let error = service
        .stop_model_generation(MODEL, generation)
        .await
        .unwrap_err();
    assert_eq!(error.kind, DeploymentErrorKind::OldRuntimeStopFailed);
    assert_eq!(error.preserved_state, None);
    assert!(!service
        .stop_model_generation(MODEL, generation)
        .await
        .unwrap());
    assert!(publisher.received.lock().await.iter().any(|event| {
        event.kind == DeploymentEventKind::StateChanged(NodeState::Failed)
            && event.generation == generation
            && event.error_kind == Some(DeploymentErrorKind::ProcessExited)
    }));
}

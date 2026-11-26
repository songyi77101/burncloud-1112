use super::*;
use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::tempdir;
use tokio::sync::Notify;

fn test_spec(version: &str) -> (RuntimeCacheSpec, Vec<u8>) {
    let archive = b"trusted test runtime archive".to_vec();
    let sha256 = encode_hex(&Sha256::digest(&archive));
    let key = RuntimeCacheKey::new(version, "x86_64-unknown-linux-gnu", "cpu", sha256)
        .unwrap_or_else(|error| panic!("test cache key should be valid: {error}"));
    let spec = RuntimeCacheSpec::new(
        key,
        "llama-test.tar.gz",
        archive.len() as u64,
        "llama-server",
    )
    .unwrap_or_else(|error| panic!("test cache spec should be valid: {error}"));
    (spec, archive)
}

fn write_valid_cache(root: &Path, spec: &RuntimeCacheSpec, archive: &[u8]) -> PathBuf {
    let cache_directory = spec.key.cache_directory(root);
    let archive_directory = cache_directory.join("archive");
    let extracted_directory = cache_directory.join("extracted");
    fs::create_dir_all(&archive_directory).expect("test archive directory should be created");
    fs::create_dir_all(&extracted_directory).expect("test extracted directory should be created");
    fs::write(archive_directory.join(&spec.archive_filename), archive)
        .expect("test archive should be written");
    fs::write(
        extracted_directory.join(&spec.server_relative_path),
        b"test executable",
    )
    .expect("test executable should be written");
    fs::write(
        cache_directory.join("runtime.json"),
        serde_json::to_vec(&spec.metadata()).expect("test metadata should serialize"),
    )
    .expect("test metadata should be written");
    fs::write(cache_directory.join("ready"), ready_marker(&spec.key))
        .expect("test ready marker should be written");
    cache_directory
}

#[test]
fn cache_key_and_layout_separate_every_identity_field() {
    let (spec, _) = test_spec("b12757");
    let root = Path::new("runtime-cache");
    let expected = root
        .join("b12757")
        .join("x86_64-unknown-linux-gnu")
        .join("cpu")
        .join(&spec.key.sha256);
    assert_eq!(spec.key.cache_directory(root), expected);

    let paths = runtime_download_paths(root, &spec.key, &spec.archive_filename)
        .unwrap_or_else(|error| panic!("test download paths should be valid: {error}"));
    assert_eq!(
        paths.partial_archive,
        root.join("downloads")
            .join("b12757")
            .join("x86_64-unknown-linux-gnu")
            .join("llama-test.tar.gz.part")
    );
    assert_eq!(
        paths.sidecar,
        root.join("downloads")
            .join("b12757")
            .join("x86_64-unknown-linux-gnu")
            .join("llama-test.tar.gz.part.json")
    );

    let distinct = RuntimeCacheKey::new(
        "b12758",
        "x86_64-unknown-linux-gnu",
        "cpu",
        spec.key.sha256.clone(),
    )
    .unwrap_or_else(|error| panic!("test cache key should be valid: {error}"));
    assert_ne!(spec.key, distinct);
}

#[test]
fn cache_identity_rejects_path_traversal_and_invalid_digest() {
    assert!(matches!(
        RuntimeCacheKey::new("../version", "target", "cpu", "a".repeat(64)),
        Err(RuntimeCacheError::ConfigurationInvalid(_))
    ));
    assert!(matches!(
        RuntimeCacheKey::new("version", "target", "cpu", "z".repeat(64)),
        Err(RuntimeCacheError::ConfigurationInvalid(_))
    ));
    let (spec, _) = test_spec("b12757");
    assert!(matches!(
        RuntimeCacheSpec::new(spec.key.clone(), "../archive.zip", 10, "llama-server"),
        Err(RuntimeCacheError::ConfigurationInvalid(_))
    ));
    assert!(matches!(
        RuntimeCacheSpec::new(spec.key, "archive.zip", 10, "../llama-server"),
        Err(RuntimeCacheError::ConfigurationInvalid(_))
    ));
}

#[test]
fn runtime_cache_root_uses_user_data_and_rejects_unsafe_overrides() {
    let base = tempdir().expect("temporary test directory should be created");
    let app = base.path().join("installation");
    let project = base.path().join("project");
    let target = project.join("target");
    fs::create_dir_all(&app).expect("test installation directory should be created");
    fs::create_dir_all(&target).expect("test target directory should be created");

    let user_data = base.path().join("user-data");
    let default = runtime_cache_candidate(None, Some(&user_data))
        .unwrap_or_else(|error| panic!("test user data root should be valid: {error}"));
    assert_eq!(default, user_data.join("BurnCloud").join("runtimes"));

    let unsafe_override = project.join("runtime-cache");
    let rejected = read_runtime_cache_root(&app, &target, Some(unsafe_override.as_os_str()));
    assert!(matches!(
        rejected,
        Err(RuntimeCacheError::ConfigurationInvalid(_))
    ));
    assert!(!unsafe_override.exists());

    let relative_override = OsStr::new("relative-cache");
    assert!(matches!(
        runtime_cache_candidate(Some(relative_override), Some(&user_data)),
        Err(RuntimeCacheError::ConfigurationInvalid(_))
    ));

    let safe_override = base.path().join("external-cache");
    let root = read_runtime_cache_root(&app, &target, Some(safe_override.as_os_str()))
        .unwrap_or_else(|error| panic!("safe test cache root should resolve: {error}"));
    assert!(root.is_absolute());
    assert_eq!(
        root,
        fs::canonicalize(safe_override).expect("cache root should canonicalize")
    );
}

#[test]
fn cache_hit_requires_ready_metadata_files_and_matching_archive_digest() {
    let base = tempdir().expect("temporary test directory should be created");
    let root = base.path();
    let (spec, archive) = test_spec("b12757");
    let cache_directory = write_valid_cache(root, &spec, &archive);

    let prepared = check_cached_runtime(root, &spec)
        .unwrap_or_else(|error| panic!("valid cache check should succeed: {error}"));
    assert!(prepared.is_some());
    assert_eq!(
        prepared
            .as_ref()
            .map(|runtime| PathBuf::from(&runtime.executable)),
        Some(
            fs::canonicalize(cache_directory.join("extracted/llama-server"))
                .expect("test executable should canonicalize")
        )
    );

    let (new_spec, new_archive) = test_spec("b12758");
    let bad_cache = write_valid_cache(root, &new_spec, &new_archive);
    let corrupted_archive = vec![b'x'; new_archive.len()];
    fs::write(
        bad_cache.join("archive").join(&new_spec.archive_filename),
        corrupted_archive,
    )
    .expect("corrupted test archive should be written");

    let result = check_cached_runtime(root, &new_spec)
        .unwrap_or_else(|error| panic!("invalid cache should be isolated: {error}"));
    assert!(result.is_none());
    assert!(!bad_cache.exists());
    assert!(root.join(".quarantine").is_dir());
    assert!(check_cached_runtime(root, &spec)
        .unwrap_or_else(|error| panic!("older valid cache should remain: {error}"))
        .is_some());
}

#[test]
fn cache_hit_rejects_tampered_server_path_metadata() {
    let base = tempdir().expect("temporary test directory should be created");
    let (spec, archive) = test_spec("b12757");
    let cache_directory = write_valid_cache(base.path(), &spec, &archive);
    let mut metadata = spec.metadata();
    metadata.server_relative_path = "../../outside".to_string();
    fs::write(
        cache_directory.join("runtime.json"),
        serde_json::to_vec(&metadata).expect("tampered metadata should serialize"),
    )
    .expect("tampered metadata should be written");

    let result = check_cached_runtime(base.path(), &spec)
        .unwrap_or_else(|error| panic!("bad cache should be isolated: {error}"));
    assert!(result.is_none());
    assert!(!cache_directory.exists());
}

#[test]
fn staging_is_cleaned_on_drop_and_committed_atomically() {
    let base = tempdir().expect("temporary test directory should be created");
    let root = base.path();
    let (spec, archive) = test_spec("b12757");
    let stage_path;
    {
        let staging = create_runtime_staging(root, "job-cleanup")
            .unwrap_or_else(|error| panic!("test staging should be created: {error}"));
        stage_path = staging.path().to_path_buf();
        assert!(stage_path.join("archive").is_dir());
        assert!(stage_path.join("extracted").is_dir());
    }
    assert!(!stage_path.exists());

    let staging = create_runtime_staging(root, "job-commit")
        .unwrap_or_else(|error| panic!("test staging should be created: {error}"));
    fs::write(
        staging.path().join("archive").join(&spec.archive_filename),
        archive,
    )
    .expect("test archive should be staged");
    fs::write(
        staging
            .path()
            .join("extracted")
            .join(&spec.server_relative_path),
        b"test executable",
    )
    .expect("test executable should be staged");

    let committed_stage = staging.path().to_path_buf();
    let committed = commit_runtime_staging(&staging, &spec)
        .unwrap_or_else(|error| panic!("test staging should commit: {error}"));
    let expected = fs::canonicalize(spec.key.cache_directory(root))
        .unwrap_or_else(|error| panic!("committed cache should canonicalize: {error}"));
    assert_eq!(committed, expected);
    assert!(!committed_stage.exists());
    assert!(check_cached_runtime(root, &spec)
        .unwrap_or_else(|error| panic!("committed cache should validate: {error}"))
        .is_some());
}

#[test]
fn abandoned_staging_with_ready_marker_is_not_a_cache_hit() {
    let base = tempdir().expect("temporary test directory should be created");
    let root = base.path();
    let (spec, archive) = test_spec("b12757");
    let abandoned = root.join(".staging").join("previous-process");
    let archive_directory = abandoned.join("archive");
    let extracted_directory = abandoned.join("extracted");
    fs::create_dir_all(&archive_directory).expect("abandoned archive directory should exist");
    fs::create_dir_all(&extracted_directory).expect("abandoned extracted directory should exist");
    fs::write(archive_directory.join(&spec.archive_filename), archive)
        .expect("abandoned archive should exist");
    fs::write(
        extracted_directory.join(&spec.server_relative_path),
        b"test executable",
    )
    .expect("abandoned executable should exist");
    fs::write(
        abandoned.join("runtime.json"),
        serde_json::to_vec(&spec.metadata()).expect("abandoned metadata should serialize"),
    )
    .expect("abandoned metadata should exist");
    fs::write(abandoned.join("ready"), ready_marker(&spec.key))
        .expect("abandoned ready marker should exist");

    let hit = check_cached_runtime(root, &spec)
        .unwrap_or_else(|error| panic!("unfinished staging should be ignored: {error}"));
    assert!(hit.is_none());
    assert!(!spec.key.cache_directory(root).exists());
}

#[test]
fn commit_rejects_an_archive_that_does_not_match_the_cache_key() {
    let base = tempdir().expect("temporary test directory should be created");
    let root = base.path();
    let (spec, archive) = test_spec("b12757");
    let staging = create_runtime_staging(root, "bad-archive")
        .unwrap_or_else(|error| panic!("test staging should be created: {error}"));
    fs::write(
        staging.path().join("archive").join(&spec.archive_filename),
        vec![b'x'; archive.len()],
    )
    .expect("bad archive should be staged");
    fs::write(
        staging
            .path()
            .join("extracted")
            .join(&spec.server_relative_path),
        b"test executable",
    )
    .expect("test executable should be staged");

    assert!(matches!(
        commit_runtime_staging(&staging, &spec),
        Err(RuntimeCacheError::CacheInvalid(_))
    ));
    assert!(!spec.key.cache_directory(root).exists());
}
#[tokio::test]
async fn install_lock_serializes_waiters_and_cancels_waiting_subscribers() {
    let base = tempdir().expect("temporary test directory should be created");
    let (spec, _) = test_spec("b12757");
    let first_lock = acquire_runtime_install_lock(base.path(), &spec.key, watch::channel(false).1)
        .await
        .unwrap_or_else(|error| panic!("first test lock should be acquired: {error:?}"));

    let (cancel_sender, cancel_receiver) = watch::channel(false);
    let root = base.path().to_path_buf();
    let key = spec.key.clone();
    let waiter =
        tokio::spawn(
            async move { acquire_runtime_install_lock(&root, &key, cancel_receiver).await },
        );
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(!waiter.is_finished());
    drop(first_lock);
    let acquired = tokio::time::timeout(Duration::from_secs(2), waiter).await;
    let acquired_lock = match acquired {
        Ok(Ok(Ok(lock))) => lock,
        _ => panic!("waiting test lock should be acquired"),
    };
    drop(acquired_lock);

    let held_lock = acquire_runtime_install_lock(base.path(), &spec.key, watch::channel(false).1)
        .await
        .unwrap_or_else(|error| panic!("test lock should be reacquired: {error:?}"));
    let root = base.path().to_path_buf();
    let key = spec.key.clone();
    let waiting_cancel_sender = cancel_sender.clone();
    let waiting = tokio::spawn(async move {
        acquire_runtime_install_lock(&root, &key, waiting_cancel_sender.subscribe()).await
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    cancel_sender.send_replace(true);
    let cancelled = tokio::time::timeout(Duration::from_secs(2), waiting).await;
    assert!(matches!(
        cancelled,
        Ok(Ok(Err(RuntimeInstallLockError::Cancelled)))
    ));
    drop(held_lock);
}
#[tokio::test]
async fn shared_job_cancellation_only_reaches_the_last_subscriber() {
    let registry = RuntimeInstallRegistry::default();
    let (spec, _) = test_spec("b12757");
    let started = Arc::new(Notify::new());
    let run_count = Arc::new(AtomicUsize::new(0));
    let first = {
        let started = Arc::clone(&started);
        let run_count = Arc::clone(&run_count);
        registry
            .subscribe_or_start(
                spec.key.clone(),
                move |_, mut cancellation, progress| async move {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    progress.publish(RuntimeInstallProgress {
                        phase: RuntimeInstallPhase::Downloading,
                        bytes_received: 32,
                        total_bytes: Some(64),
                    });
                    started.notify_one();
                    loop {
                        if *cancellation.borrow() {
                            return Err("cancelled by final subscriber".to_string());
                        }
                        if cancellation.changed().await.is_err() {
                            return Err("shared cancellation channel closed".to_string());
                        }
                    }
                },
            )
            .await
    };
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .expect("shared install should start");
    let second = {
        let run_count = Arc::clone(&run_count);
        registry
            .subscribe_or_start(spec.key.clone(), move |_, _, _| async move {
                run_count.fetch_add(1, Ordering::SeqCst);
                Ok(PathBuf::from("unexpected-second-job"))
            })
            .await
    };
    assert_eq!(first.job_id(), second.job_id());
    assert_eq!(first.key(), second.key());
    assert_eq!(run_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        second.progress(),
        RuntimeInstallProgress {
            phase: RuntimeInstallPhase::Downloading,
            bytes_received: 32,
            total_bytes: Some(64),
        }
    );

    let job = Arc::clone(&first.job);
    drop(first);
    assert!(!*job.cancellation.borrow());
    drop(second);
    assert!(*job.cancellation.borrow());

    let mut status = job.status.subscribe();
    let terminal = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let current = status.borrow().clone();
            if current.is_terminal() {
                break current;
            }
            if status.changed().await.is_err() {
                break RuntimeInstallStatus::Interrupted;
            }
        }
    })
    .await;
    assert!(matches!(terminal, Ok(RuntimeInstallStatus::Cancelled)));
}

#[tokio::test]
async fn shared_install_result_is_published_to_every_subscriber() {
    let registry = RuntimeInstallRegistry::default();
    let (spec, _) = test_spec("b12757");
    let first = registry
        .subscribe_or_start(spec.key.clone(), |_, _, _| async {
            Ok(PathBuf::from("cached/llama-server"))
        })
        .await;
    let expected = first.job_id();
    let state = tokio::time::timeout(Duration::from_secs(2), async {
        let mut subscriber = registry
            .subscribe_or_start(spec.key.clone(), |_, _, _| async {
                Ok(PathBuf::from("duplicate/llama-server"))
            })
            .await;
        assert_eq!(subscriber.job_id(), expected);
        subscriber.wait().await
    })
    .await;
    assert!(matches!(
        state,
        Ok(RuntimeInstallStatus::Ready(path)) if path.as_path() == Path::new("cached/llama-server")
    ));
}

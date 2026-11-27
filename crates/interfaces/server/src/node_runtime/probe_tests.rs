use super::*;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};
use tempfile::tempdir;

fn current_target() -> &'static str {
    if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn server_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

fn server_root(base: &Path, relative_path: &str) -> PathBuf {
    let root = base.join("extracted");
    let executable = root.join(relative_path);
    if let Some(parent) = executable.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create server fixture directory: {error}"));
    }
    let current = std::env::current_exe()
        .unwrap_or_else(|error| panic!("resolve current test executable: {error}"));
    fs::copy(current, executable)
        .unwrap_or_else(|error| panic!("copy test executable as server fixture: {error}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join(relative_path);
        let mut permissions = fs::metadata(&path)
            .unwrap_or_else(|error| panic!("read fixture permissions: {error}"))
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions)
            .unwrap_or_else(|error| panic!("set fixture executable mode: {error}"));
    }
    root
}

#[test]
fn backend_parser_and_selection_are_target_specific() {
    assert_eq!(RequestedBackend::parse("auto"), Ok(RequestedBackend::Auto));
    assert_eq!(RequestedBackend::parse("cpu"), Ok(RequestedBackend::Cpu));
    assert_eq!(
        RequestedBackend::parse("metal"),
        Ok(RequestedBackend::Metal)
    );
    assert_eq!(
        RequestedBackend::parse("cuda")
            .err()
            .map(|error| error.code()),
        Some("unsupported_backend")
    );
    assert_eq!(
        RequestedBackend::parse("other")
            .err()
            .map(|error| error.code()),
        Some("unsupported_backend")
    );

    assert_eq!(
        select_effective_backend("x86_64-pc-windows-msvc", RequestedBackend::Auto, None),
        Ok("cpu")
    );
    assert_eq!(
        select_effective_backend("x86_64-unknown-linux-gnu", RequestedBackend::Cpu, None),
        Ok("cpu")
    );
    assert_eq!(
        select_effective_backend("aarch64-apple-darwin", RequestedBackend::Auto, Some(1)),
        Ok("metal")
    );
    assert_eq!(
        select_effective_backend("x86_64-apple-darwin", RequestedBackend::Auto, Some(0)),
        Ok("cpu")
    );
    assert_eq!(
        select_effective_backend("aarch64-apple-darwin", RequestedBackend::Metal, Some(0))
            .err()
            .map(|error| error.code()),
        Some("runtime_device_unavailable")
    );
    assert_eq!(
        select_effective_backend("x86_64-unknown-linux-gnu", RequestedBackend::Metal, Some(1))
            .err()
            .map(|error| error.code()),
        Some("unsupported_backend")
    );
    assert_eq!(
        select_effective_backend("aarch64-unknown-linux-gnu", RequestedBackend::Cpu, None)
            .err()
            .map(|error| error.code()),
        Some("unsupported_platform")
    );
}

#[cfg(unix)]
fn scripted_server_root(base: &Path, relative_path: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let root = base.join("scripted-extracted");
    let executable = root.join(relative_path);
    if let Some(parent) = executable.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create server fixture directory: {error}"));
    }
    fs::write(
        &executable,
        r#"#!/bin/sh
case "$1" in
  --version) echo 'llama.cpp b12757' ;;
  --help) echo 'llama-server --model --host' ;;
  --list-devices) echo 'Metal0: Apple GPU' ;;
  *) exit 2 ;;
esac
"#,
    )
    .unwrap_or_else(|error| panic!("write scripted server fixture: {error}"));
    let mut permissions = fs::metadata(&executable)
        .unwrap_or_else(|error| panic!("read server fixture permissions: {error}"))
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions)
        .unwrap_or_else(|error| panic!("set server fixture executable mode: {error}"));
    root
}

#[cfg(unix)]
fn hanging_server_path(base: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let executable = base.join("llama-server");
    fs::write(&executable, "#!/bin/sh\nwhile :; do :; done\n")
        .unwrap_or_else(|error| panic!("write hanging server fixture: {error}"));
    let mut permissions = fs::metadata(&executable)
        .unwrap_or_else(|error| panic!("read hanging fixture permissions: {error}"))
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions)
        .unwrap_or_else(|error| panic!("set hanging fixture executable mode: {error}"));
    executable
}
#[test]
fn metal_help_probe_requires_llama_server_identity() {
    let valid = ProbeOutput {
        text: "Usage: llama-server [OPTIONS]".to_string(),
        successful: true,
        truncated: false,
    };
    assert!(validate_help_probe(&valid).is_ok());
    let unrelated = ProbeOutput {
        text: "Usage: unrelated [OPTIONS]".to_string(),
        successful: true,
        truncated: false,
    };
    assert_eq!(
        validate_help_probe(&unrelated).err(),
        Some(RuntimeProbeError::Failed)
    );
}
#[test]
fn metal_device_parser_counts_only_indexed_devices() {
    let output = "Available devices:\nMetal0: Apple M3\nCPU: system\nMetal1: Apple M3\nMetal backend is enabled";
    assert_eq!(count_metal_devices(output), 2);
    assert_eq!(count_metal_devices("No devices found"), 0);
}

#[test]
fn server_locator_requires_manifest_path_to_resolve_to_a_regular_file() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let relative = format!("bin/{}", server_name());
    let root = server_root(base.path(), &relative);
    let entry = RuntimeProbeEntry {
        target: current_target().to_string(),
        server_relative_path: relative,
    };
    let executable = locate_server(&root, &entry)
        .unwrap_or_else(|error| panic!("locate valid server fixture: {error}"));
    assert!(executable.is_file());

    for hostile in [
        "../llama-server",
        "/tmp/llama-server",
        "C:\\Windows\\llama-server.exe",
        "bin//llama-server",
        "bin/./llama-server",
        "bin/other-server",
    ] {
        let entry = RuntimeProbeEntry {
            target: current_target().to_string(),
            server_relative_path: hostile.to_string(),
        };
        assert_eq!(
            locate_server(&root, &entry).err(),
            Some(RuntimeProbeError::InvalidArchive),
            "{hostile}"
        );
    }

    let directory_entry = RuntimeProbeEntry {
        target: current_target().to_string(),
        server_relative_path: format!("bin/{}", server_name()),
    };
    let directory_root = base.path().join("directory-root");
    fs::create_dir_all(directory_root.join(format!("bin/{}", server_name())))
        .unwrap_or_else(|error| panic!("create directory at server path: {error}"));
    assert_eq!(
        locate_server(&directory_root, &directory_entry).err(),
        Some(RuntimeProbeError::InvalidArchive)
    );
}

#[tokio::test]
async fn read_limited_caps_retained_bytes_while_draining_the_pipe() {
    use tokio::io::AsyncWriteExt;

    let (mut writer, reader) = tokio::io::duplex(8);
    let write = async move {
        writer
            .write_all(&[b'x'; 128])
            .await
            .unwrap_or_else(|error| panic!("write oversized probe fixture: {error}"));
    };
    let read = read_limited(reader, 16);
    let (_, output) = tokio::join!(write, read);
    let output = output.unwrap_or_else(|error| panic!("read bounded probe fixture: {error}"));
    assert_eq!(output.bytes.len(), 16);
    assert!(output.truncated);
}
#[test]
fn probe_output_must_be_successful_nonempty_and_bounded() {
    let output = ProbeOutput {
        text: format!("version {}\n", "x".repeat(MAX_SUMMARY_BYTES + 100)),
        successful: true,
        truncated: false,
    };
    let summary = validate_probe_output(&output)
        .unwrap_or_else(|error| panic!("validate bounded version summary: {error}"));
    assert!(summary.len() <= MAX_SUMMARY_BYTES);

    for output in [
        ProbeOutput {
            text: "version".to_string(),
            successful: false,
            truncated: false,
        },
        ProbeOutput {
            text: "version".to_string(),
            successful: true,
            truncated: true,
        },
        ProbeOutput {
            text: "  \n".to_string(),
            successful: true,
            truncated: false,
        },
    ] {
        assert_eq!(
            validate_probe_output(&output).err(),
            Some(RuntimeProbeError::Failed)
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_a_running_probe_terminates_and_waits_for_the_child() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let executable = hanging_server_path(base.path());
    let cancellation = CancellationToken::new();
    let child_cancellation = cancellation.clone();
    let probe = tokio::spawn(async move {
        run_probe_process(&executable, "--version", &child_cancellation).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    cancellation.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), probe)
        .await
        .unwrap_or_else(|error| panic!("cancelled probe did not finish: {error}"))
        .unwrap_or_else(|error| panic!("probe task failed: {error}"));
    assert_eq!(result.err(), Some(RuntimeProbeError::Cancelled));
}
#[cfg(unix)]
#[tokio::test]
async fn probe_runs_only_the_manifest_server_and_returns_a_controlled_summary() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let relative = server_name();
    let root = scripted_server_root(base.path(), relative);
    let entry = RuntimeProbeEntry {
        target: current_target().to_string(),
        server_relative_path: relative.to_string(),
    };
    let cancellation = CancellationToken::new();
    let candidate = Box::pin(probe_runtime(
        &root,
        &entry,
        RequestedBackend::Cpu,
        &cancellation,
    ))
    .await
    .unwrap_or_else(|error| panic!("probe executable fixture: {error}"));

    assert_eq!(candidate.summary.effective_backend, "cpu");
    assert!(candidate.summary.help_probe_passed);
    assert!(candidate.summary.dependencies_loaded);
    assert!(candidate.summary.metal_device_count.is_none());
    assert!(!candidate.summary.version_output.is_empty());
    assert!(chrono::DateTime::parse_from_rfc3339(&candidate.summary.probed_at).is_ok());
    assert_eq!(
        candidate.executable,
        root.join(relative)
            .canonicalize()
            .unwrap_or_else(|error| panic!("canonicalize executable: {error}"))
    );
}

#[tokio::test]
async fn probe_reports_cancellation_and_does_not_leak_paths_in_errors() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let entry = RuntimeProbeEntry {
        target: current_target().to_string(),
        server_relative_path: server_name().to_string(),
    };
    let error = Box::pin(probe_runtime(
        base.path(),
        &entry,
        RequestedBackend::Cpu,
        &cancellation,
    ))
    .await
    .expect_err("missing server must be rejected before spawning a child");
    assert_eq!(error.code(), "runtime_archive_invalid");
    assert!(!error
        .to_string()
        .contains(&base.path().to_string_lossy().to_string()));

    let root = server_root(base.path(), server_name());
    let error = Box::pin(probe_runtime(
        &root,
        &entry,
        RequestedBackend::Cpu,
        &cancellation,
    ))
    .await
    .expect_err("cancelled probe must not start a child");
    assert_eq!(error.code(), "cancelled");
}

#[cfg(windows)]
#[tokio::test]
async fn process_runner_captures_bounded_command_output() {
    let system_root = std::env::var_os("SystemRoot")
        .unwrap_or_else(|| panic!("Windows SystemRoot environment is not set"));
    let command = PathBuf::from(system_root).join("System32").join("cmd.exe");
    let output = Box::pin(run_probe_process(&command, "/?", &CancellationToken::new()))
        .await
        .unwrap_or_else(|error| panic!("run harmless command fixture: {error}"));
    assert!(!output.text.is_empty());
    assert!(!output.truncated);
}
#[cfg(unix)]
#[test]
fn server_locator_rejects_symbolic_link_components() {
    use std::os::unix::fs::symlink;

    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let root = base.path().join("extracted");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("create root: {error}"));
    let outside = base.path().join(server_name());
    fs::copy(
        std::env::current_exe()
            .unwrap_or_else(|error| panic!("resolve current executable: {error}")),
        &outside,
    )
    .unwrap_or_else(|error| panic!("copy external fixture: {error}"));
    symlink(&outside, root.join(server_name()))
        .unwrap_or_else(|error| panic!("create symbolic link fixture: {error}"));
    let entry = RuntimeProbeEntry {
        target: current_target().to_string(),
        server_relative_path: server_name().to_string(),
    };
    assert_eq!(
        locate_server(&root, &entry).err(),
        Some(RuntimeProbeError::InvalidArchive)
    );
}

#[test]
fn errors_implement_standard_error_without_sensitive_diagnostics() {
    let error = RuntimeProbeError::Failed;
    let display = error.to_string();
    assert_eq!(error.code(), "runtime_probe_failed");
    assert!(!display.contains("C:\\"));
    assert!((&error as &dyn Error).source().is_none());
}

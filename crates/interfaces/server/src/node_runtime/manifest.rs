//! 固定 Gitee 运行时清单的解析与目标资源选择。

use burncloud_download::RuntimeArchive;
use burncloud_node_runtime::RuntimeRequest;
use serde::Deserialize;
use std::collections::BTreeMap;

use super::probe::{RequestedBackend, RuntimeProbeEntry};

const MANIFEST_JSON: &str = include_str!("../../../../../deploy/runtime/manifest.json");
const MANIFEST_SCHEMA_VERSION: u32 = 2;
const PROVIDER: &str = "gitee";
const REPOSITORY: &str = "songyi777/llama-cpp";
const COMMIT: &str = "51d01f6e526bc6d45b9d252b1d2d487efb6625d8";
const VERSION: &str = "b12757";
const RELEASE_URL_TEMPLATE: &str =
    "https://gitee.com/songyi777/llama-cpp/releases/download/{version}/{filename}";
const MAX_PATH_BYTES: u64 = 4096;
const MAX_ENTRIES: u64 = 100_000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

const TARGETS: [(&str, &str, &str, &[&str]); 4] = [
    (
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        "cpu",
        &["cpu"],
    ),
    (
        "x86_64-apple-darwin",
        "llama-b12757-bin-macos-x64.tar.gz",
        "native",
        &["cpu", "metal"],
    ),
    (
        "aarch64-apple-darwin",
        "llama-b12757-bin-macos-arm64.tar.gz",
        "native",
        &["cpu", "metal"],
    ),
    (
        "x86_64-unknown-linux-gnu",
        "llama-b12757-bin-ubuntu-x64.tar.gz",
        "cpu",
        &["cpu"],
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedRuntimeEntry {
    pub(super) version: String,
    pub(super) target: String,
    pub(super) package_kind: String,
    pub(super) archive: RuntimeArchive,
    pub(super) archive_format: String,
    pub(super) server_relative_path: String,
    pub(super) dependencies: Vec<String>,
    pub(super) license_components: Vec<String>,
    pub(super) requested_backend: RequestedBackend,
    pub(super) max_path_bytes: u64,
    pub(super) max_entries: u64,
    pub(super) max_file_bytes: u64,
    pub(super) max_unpacked_bytes: u64,
}

impl ResolvedRuntimeEntry {
    pub(super) fn probe_entry(&self) -> RuntimeProbeEntry {
        RuntimeProbeEntry {
            target: self.target.clone(),
            server_relative_path: self.server_relative_path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RuntimeManifestError {
    Invalid,
    UnsupportedPlatform,
    UnsupportedBackend,
    ConfigurationInvalid,
}

impl RuntimeManifestError {
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "runtime_manifest_invalid",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::UnsupportedBackend => "unsupported_backend",
            Self::ConfigurationInvalid => "runtime_configuration_invalid",
        }
    }

    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::Invalid => "服务端运行时清单无效",
            Self::UnsupportedPlatform => "当前平台没有受支持的 llama.cpp 运行时",
            Self::UnsupportedBackend => "当前来源没有请求的 llama.cpp backend",
            Self::ConfigurationInvalid => "运行时请求与服务端固定版本不匹配",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeManifest {
    schema_version: u32,
    provider: String,
    repository: String,
    commit: String,
    version: String,
    release_url_template: String,
    archive_limits: ArchiveLimits,
    targets: BTreeMap<String, RuntimeTarget>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveLimits {
    max_path_bytes: u64,
    max_entries: u64,
    max_file_bytes: u64,
    max_unpacked_bytes: u64,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeTarget {
    supported_backends: Vec<String>,
    package: RuntimePackage,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePackage {
    package_kind: String,
    filename: String,
    archive_format: String,
    size_bytes: u64,
    sha256: String,
    server_relative_path: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    license_components: Vec<String>,
}

pub(super) fn resolve_runtime_entry(
    request: &RuntimeRequest,
    requested_backend: RequestedBackend,
) -> Result<ResolvedRuntimeEntry, RuntimeManifestError> {
    if request.runtime != "llama.cpp"
        || request
            .version
            .as_deref()
            .is_some_and(|version| version != VERSION)
    {
        return Err(RuntimeManifestError::ConfigurationInvalid);
    }

    let manifest = parse_manifest(MANIFEST_JSON)?;
    let target = current_target().ok_or(RuntimeManifestError::UnsupportedPlatform)?;
    let target_entry = manifest
        .targets
        .get(target)
        .ok_or(RuntimeManifestError::UnsupportedPlatform)?;
    validate_requested_backend(&target_entry.supported_backends, requested_backend)?;
    let package = &target_entry.package;
    let archive = RuntimeArchive::new(
        target,
        package.filename.clone(),
        package.size_bytes,
        package.sha256.clone(),
    )
    .map_err(|_| RuntimeManifestError::Invalid)?;

    Ok(ResolvedRuntimeEntry {
        version: manifest.version.clone(),
        target: target.to_string(),
        package_kind: package.package_kind.clone(),
        archive,
        archive_format: package.archive_format.clone(),
        server_relative_path: package.server_relative_path.clone(),
        dependencies: package.dependencies.clone(),
        license_components: package.license_components.clone(),
        requested_backend,
        max_path_bytes: manifest.archive_limits.max_path_bytes,
        max_entries: manifest.archive_limits.max_entries,
        max_file_bytes: manifest.archive_limits.max_file_bytes,
        max_unpacked_bytes: manifest.archive_limits.max_unpacked_bytes,
    })
}

fn parse_manifest(raw: &str) -> Result<RuntimeManifest, RuntimeManifestError> {
    let manifest: RuntimeManifest =
        serde_json::from_str(raw).map_err(|_| RuntimeManifestError::Invalid)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &RuntimeManifest) -> Result<(), RuntimeManifestError> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.provider != PROVIDER
        || manifest.repository != REPOSITORY
        || manifest.commit != COMMIT
        || manifest.version != VERSION
        || manifest.release_url_template != RELEASE_URL_TEMPLATE
        || manifest.targets.len() != TARGETS.len()
    {
        return Err(RuntimeManifestError::Invalid);
    }
    let limits = &manifest.archive_limits;
    if limits.max_path_bytes == 0
        || limits.max_path_bytes > MAX_PATH_BYTES
        || limits.max_entries == 0
        || limits.max_entries > MAX_ENTRIES
        || limits.max_file_bytes == 0
        || limits.max_file_bytes > MAX_FILE_BYTES
        || limits.max_unpacked_bytes == 0
        || limits.max_unpacked_bytes > MAX_UNPACKED_BYTES
    {
        return Err(RuntimeManifestError::Invalid);
    }

    for (target, filename, package_kind, supported_backends) in TARGETS {
        let target_entry = manifest
            .targets
            .get(target)
            .ok_or(RuntimeManifestError::Invalid)?;
        if target_entry.supported_backends
            != supported_backends
                .iter()
                .map(|backend| (*backend).to_string())
                .collect::<Vec<_>>()
        {
            return Err(RuntimeManifestError::Invalid);
        }
        let package = &target_entry.package;
        let expected_format = if filename.ends_with(".zip") {
            "zip"
        } else {
            "tar.gz"
        };
        let expected_server = if target == "x86_64-pc-windows-msvc" {
            "llama-server.exe"
        } else {
            "llama-b12757/llama-server"
        };
        if package.package_kind != package_kind
            || package.filename != filename
            || package.archive_format != expected_format
            || package.size_bytes == 0
            || !is_sha256(&package.sha256)
            || package.server_relative_path != expected_server
            || !is_safe_relative_path(&package.server_relative_path)
        {
            return Err(RuntimeManifestError::Invalid);
        }
        // 验证每项不可选的下载身份，防止错误平台、文件名或摘要配置进入传输层。
        RuntimeArchive::new(
            target,
            package.filename.clone(),
            package.size_bytes,
            package.sha256.clone(),
        )
        .map_err(|_| RuntimeManifestError::Invalid)?;
    }
    Ok(())
}

fn validate_requested_backend(
    supported_backends: &[String],
    requested_backend: RequestedBackend,
) -> Result<(), RuntimeManifestError> {
    let supported = match requested_backend {
        RequestedBackend::Auto | RequestedBackend::Cpu => {
            supported_backends.iter().any(|backend| backend == "cpu")
        }
        RequestedBackend::Metal => supported_backends.iter().any(|backend| backend == "metal"),
    };
    if supported {
        Ok(())
    } else {
        Err(RuntimeManifestError::UnsupportedBackend)
    }
}

fn current_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        _ => None,
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PATH_BYTES as usize
        && !value.contains('\0')
        && !value.starts_with('/')
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        && !value.contains('\\')
        && !value.contains(':')
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;

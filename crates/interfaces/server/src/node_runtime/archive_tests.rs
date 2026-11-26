use super::*;
use flate2::{write::GzEncoder, Compression};
use std::{
    fs,
    io::Write,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
use tar::{Builder, EntryType, Header};
use tempfile::tempdir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

static NEXT_TEST_STAGING: AtomicUsize = AtomicUsize::new(0);

fn test_limits() -> RuntimeArchiveLimits {
    RuntimeArchiveLimits {
        max_path_bytes: 64,
        max_entries: 10,
        max_file_bytes: 16,
        max_unpacked_bytes: 24,
    }
}

fn create_staging(base: &Path) -> PathBuf {
    let staging = base.join(format!(
        "job-{}",
        NEXT_TEST_STAGING.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&staging).unwrap_or_else(|error| panic!("create staging: {error}"));
    fs::create_dir(staging.join("extracted"))
        .unwrap_or_else(|error| panic!("create extracted root: {error}"));
    staging
}

type ZipTestEntry<'a> = (&'a str, &'a [u8], Option<u32>);

fn write_zip(path: &Path, entries: &[ZipTestEntry<'_>]) -> Result<(), Box<dyn Error>> {
    let file = fs::File::create(path)?;
    let mut writer = ZipWriter::new(file);
    for (name, contents, mode) in entries {
        let mut options =
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        if let Some(mode) = mode {
            options = options.unix_permissions(*mode);
        }
        writer.start_file(*name, options)?;
        writer.write_all(contents)?;
    }
    writer.finish()?;
    Ok(())
}

fn write_tar_gz_file(
    path: &Path,
    name: &str,
    contents: &[u8],
    mode: u32,
) -> Result<(), Box<dyn Error>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.set_entry_type(EntryType::Regular);
    header.set_size(u64::try_from(contents.len())?);
    header.set_mode(mode);
    header.set_cksum();
    builder.append_data(&mut header, name, contents)?;
    let encoder = builder.into_inner()?;
    fs::write(path, encoder.finish()?)?;
    Ok(())
}

fn write_tar_gz_link(
    path: &Path,
    name: &str,
    target: &str,
    kind: EntryType,
) -> Result<(), Box<dyn Error>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_link_name(target)?;
    header.set_cksum();
    builder.append_data(&mut header, name, std::io::empty())?;
    let encoder = builder.into_inner()?;
    fs::write(path, encoder.finish()?)?;
    Ok(())
}

fn extract_test_archive(
    archive_path: &Path,
    target: &str,
    server_relative_path: &str,
    staging: &Path,
    limits: &RuntimeArchiveLimits,
    cancellation: &watch::Receiver<bool>,
) -> Result<ExtractedRuntime, RuntimeArchiveError> {
    let expected_size = fs::metadata(archive_path)
        .map_err(|error| RuntimeArchiveError::Io(error.to_string()))?
        .len();
    extract_archive_file(
        archive_path,
        expected_size,
        target,
        server_relative_path,
        staging,
        limits,
        cancellation,
    )
}

#[test]
fn entry_path_validation_rejects_cross_platform_escape_forms() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let root = base.path().join("extracted");
    fs::create_dir(&root).unwrap_or_else(|error| panic!("create extracted: {error}"));
    let limits = test_limits();
    for hostile in [
        "../escape",
        "a/../../escape",
        "/etc/passwd",
        "\\\\server\\share\\file",
        "C:\\Windows\\system.ini",
        "folder\\..\\escape",
        "a//b",
        "a/./b",
    ] {
        assert!(
            validate_archive_entry_path(hostile, false, &root, &limits).is_err(),
            "{hostile}"
        );
    }
    assert!(validate_archive_entry_path("a\0b", false, &root, &limits).is_err());
    assert!(validate_archive_entry_path(&"x".repeat(65), false, &root, &limits).is_err());
    assert!(validate_archive_entry_path("bin/llama-server", false, &root, &limits).is_ok());
    assert!(validate_archive_entry_path("bin/", true, &root, &limits).is_ok());
}

#[test]
fn zip_extracts_regular_files_and_checks_the_server_path() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let staging = create_staging(base.path());
    let archive_path = base.path().join("runtime.zip");
    write_zip(
        &archive_path,
        &[
            ("bin/llama-server.exe", b"server", Some(0o100755)),
            ("lib/runtime.dll", b"dll", None),
        ],
    )
    .unwrap_or_else(|error| panic!("write test ZIP: {error}"));
    let cancellation = watch::channel(false).1;
    let extracted = extract_test_archive(
        &archive_path,
        "x86_64-pc-windows-msvc",
        "bin/llama-server.exe",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .unwrap_or_else(|error| panic!("extract test ZIP: {error}"));
    assert_eq!(extracted.entry_count, 2);
    assert_eq!(extracted.unpacked_bytes, 9);
    assert_eq!(
        fs::read(extracted.extracted_root.join("bin/llama-server.exe"))
            .unwrap_or_else(|error| panic!("read server file: {error}")),
        b"server"
    );
    assert_eq!(
        fs::read(extracted.extracted_root.join("lib/runtime.dll"))
            .unwrap_or_else(|error| panic!("read library: {error}")),
        b"dll"
    );
}
#[test]
fn zip_traversal_is_rejected_and_does_not_damage_an_old_cache() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let staging = create_staging(base.path());
    let sentinel = base.path().join("existing-runtime");
    fs::create_dir(&sentinel).unwrap_or_else(|error| panic!("create old runtime: {error}"));
    fs::write(sentinel.join("ready"), b"valid")
        .unwrap_or_else(|error| panic!("write ready: {error}"));
    let archive_path = base.path().join("runtime.zip");
    write_zip(&archive_path, &[("../escape.txt", b"bad", None)])
        .unwrap_or_else(|error| panic!("write hostile ZIP: {error}"));
    let cancellation = watch::channel(false).1;
    let error = extract_test_archive(
        &archive_path,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("路径穿越 ZIP 必须被拒绝");
    assert_eq!(error.code(), "runtime_archive_invalid");
    assert!(!base.path().join("escape.txt").exists());
    assert!(!staging.join("extracted").exists());
    assert_eq!(
        fs::read(sentinel.join("ready")).unwrap_or_default(),
        b"valid"
    );
}

#[test]
fn zip_symlink_entry_is_rejected_and_cleaned() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let staging = create_staging(base.path());
    let archive_path = base.path().join("runtime.zip");
    let mut writer = ZipWriter::new(
        fs::File::create(&archive_path)
            .unwrap_or_else(|error| panic!("create symlink ZIP: {error}")),
    );
    writer
        .add_symlink(
            "llama-server.exe",
            "../outside.txt",
            SimpleFileOptions::default(),
        )
        .unwrap_or_else(|error| panic!("write symlink ZIP: {error}"));
    writer
        .finish()
        .unwrap_or_else(|error| panic!("finish symlink ZIP: {error}"));
    let cancellation = watch::channel(false).1;
    let error = extract_test_archive(
        &archive_path,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("ZIP 符号链接必须被拒绝");
    assert_eq!(error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());
}

#[test]
fn tar_gz_extracts_a_regular_executable() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let staging = create_staging(base.path());
    let archive_path = base.path().join("runtime.tar.gz");
    write_tar_gz_file(&archive_path, "bin/llama-server", b"server", 0o755)
        .unwrap_or_else(|error| panic!("write test TAR.GZ: {error}"));
    let cancellation = watch::channel(false).1;
    let extracted = extract_test_archive(
        &archive_path,
        "aarch64-apple-darwin",
        "bin/llama-server",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .unwrap_or_else(|error| panic!("extract test TAR.GZ: {error}"));
    assert_eq!(
        fs::read(extracted.extracted_root.join("bin/llama-server")).unwrap_or_default(),
        b"server"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(extracted.extracted_root.join("bin/llama-server"))
            .unwrap_or_else(|error| panic!("read executable mode: {error}"));
        assert_ne!(metadata.permissions().mode() & 0o100, 0);
    }
}

#[test]
fn tar_gz_symlink_and_hardlink_entries_are_rejected() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let sentinel = base.path().join("outside.txt");
    fs::write(&sentinel, b"untouched").unwrap_or_else(|error| panic!("write sentinel: {error}"));
    let cancellation = watch::channel(false).1;
    for (kind, filename) in [
        (EntryType::Symlink, "symlink.tar.gz"),
        (EntryType::Link, "hardlink.tar.gz"),
    ] {
        let staging = create_staging(base.path());
        let archive_path = base.path().join(filename);
        write_tar_gz_link(&archive_path, "llama-server", "../outside.txt", kind)
            .unwrap_or_else(|error| panic!("write hostile TAR.GZ: {error}"));
        let error = extract_test_archive(
            &archive_path,
            "x86_64-unknown-linux-gnu",
            "llama-server",
            &staging,
            &test_limits(),
            &cancellation,
        )
        .expect_err("TAR.GZ 链接 entry 必须被拒绝");
        assert_eq!(error.code(), "runtime_archive_invalid");
        assert!(!staging.join("extracted").exists());
    }
    assert_eq!(fs::read(sentinel).unwrap_or_default(), b"untouched");
}
#[test]
fn archive_format_and_required_server_path_are_enforced() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let archive_path = base.path().join("runtime.zip");
    write_zip(&archive_path, &[("other-file", b"content", None)])
        .unwrap_or_else(|error| panic!("write ZIP: {error}"));
    let cancellation = watch::channel(false).1;

    let staging = create_staging(base.path());
    let format_error = extract_test_archive(
        &archive_path,
        "x86_64-apple-darwin",
        "llama-server",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("macOS target 不得使用 ZIP");
    assert_eq!(format_error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());

    let staging = create_staging(base.path());
    let missing_error = extract_test_archive(
        &archive_path,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("缺少 manifest 服务端文件必须失败");
    assert_eq!(missing_error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());
}

#[test]
fn file_entry_count_and_total_expansion_limits_are_enforced() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let cancellation = watch::channel(false).1;

    let large_file_zip = base.path().join("large-file.zip");
    write_zip(
        &large_file_zip,
        &[("llama-server.exe", b"12345678901234567", None)],
    )
    .unwrap_or_else(|error| panic!("write oversized ZIP: {error}"));
    let staging = create_staging(base.path());
    let file_error = extract_test_archive(
        &large_file_zip,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("超限单文件必须失败");
    assert_eq!(file_error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());

    let multiple_files_zip = base.path().join("multiple-files.zip");
    write_zip(
        &multiple_files_zip,
        &[
            ("llama-server.exe", b"1234567890123", None),
            ("second", b"1234567890123", None),
        ],
    )
    .unwrap_or_else(|error| panic!("write multi-file ZIP: {error}"));
    let staging = create_staging(base.path());
    let total_error = extract_test_archive(
        &multiple_files_zip,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("总展开大小超过限制必须失败");
    assert_eq!(total_error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());

    let entry_limit_zip = base.path().join("many.zip");
    write_zip(
        &entry_limit_zip,
        &[("llama-server.exe", b"1", None), ("second", b"2", None)],
    )
    .unwrap_or_else(|error| panic!("write multi-entry ZIP: {error}"));
    let staging = create_staging(base.path());
    let limits = RuntimeArchiveLimits {
        max_entries: 1,
        ..test_limits()
    };
    let count_error = extract_test_archive(
        &entry_limit_zip,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &limits,
        &cancellation,
    )
    .expect_err("超限 entry 数必须失败");
    assert_eq!(count_error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());
}

#[test]
fn cancellation_cleans_only_the_incomplete_extracted_tree() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let staging = create_staging(base.path());
    let archive_path = base.path().join("runtime.zip");
    write_zip(&archive_path, &[("llama-server.exe", b"server", None)])
        .unwrap_or_else(|error| panic!("write ZIP: {error}"));
    let old_cache = base.path().join("old-cache");
    fs::create_dir(&old_cache).unwrap_or_else(|error| panic!("create cache: {error}"));
    fs::write(old_cache.join("ready"), b"valid")
        .unwrap_or_else(|error| panic!("write ready: {error}"));
    let (sender, receiver) = watch::channel(true);

    let error = extract_test_archive(
        &archive_path,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &test_limits(),
        &receiver,
    )
    .expect_err("已取消的解压不得继续");
    assert_eq!(error.code(), "cancelled");
    assert!(!staging.join("extracted").exists());
    assert_eq!(
        fs::read(old_cache.join("ready")).unwrap_or_default(),
        b"valid"
    );
    sender.send_replace(false);
}
#[test]
fn manifest_server_path_must_resolve_to_a_regular_file() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let staging = create_staging(base.path());
    let archive_path = base.path().join("directory.zip");
    let mut writer = ZipWriter::new(
        fs::File::create(&archive_path).unwrap_or_else(|error| panic!("create ZIP: {error}")),
    );
    writer
        .add_directory("llama-server.exe/", SimpleFileOptions::default())
        .unwrap_or_else(|error| panic!("write directory entry: {error}"));
    writer
        .finish()
        .unwrap_or_else(|error| panic!("finish ZIP: {error}"));
    let cancellation = watch::channel(false).1;

    let error = extract_test_archive(
        &archive_path,
        "x86_64-pc-windows-msvc",
        "llama-server.exe",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("manifest 服务端路径指向目录时必须失败");
    assert_eq!(error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());
}

#[test]
fn tar_extension_headers_are_not_treated_as_filesystem_entries() {
    let base = tempdir().unwrap_or_else(|error| panic!("temporary directory: {error}"));
    let staging = create_staging(base.path());
    let archive_path = base.path().join("extension.tar.gz");
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.set_entry_type(EntryType::GNULongName);
    header.set_size(4);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "././@LongLink", &b"name"[..])
        .unwrap_or_else(|error| panic!("write extension entry: {error}"));
    let encoder = builder
        .into_inner()
        .unwrap_or_else(|error| panic!("finish TAR builder: {error}"));
    fs::write(
        &archive_path,
        encoder
            .finish()
            .unwrap_or_else(|error| panic!("finish gzip: {error}")),
    )
    .unwrap_or_else(|error| panic!("write TAR.GZ: {error}"));
    let cancellation = watch::channel(false).1;

    let error = extract_test_archive(
        &archive_path,
        "x86_64-unknown-linux-gnu",
        "llama-server",
        &staging,
        &test_limits(),
        &cancellation,
    )
    .expect_err("tar extension header must be rejected");
    assert_eq!(error.code(), "runtime_archive_invalid");
    assert!(!staging.join("extracted").exists());
}
#[test]
fn manifest_archive_limits_cannot_exceed_the_fixed_safety_caps() {
    let limits = RuntimeArchiveLimits::from_manifest(
        DEFAULT_MAX_PATH_BYTES,
        DEFAULT_MAX_ENTRIES,
        DEFAULT_MAX_FILE_BYTES,
        DEFAULT_MAX_UNPACKED_BYTES,
    )
    .unwrap_or_else(|error| panic!("valid manifest limits: {error}"));
    assert_eq!(limits, RuntimeArchiveLimits::default());
    assert!(RuntimeArchiveLimits::from_manifest(0, 1, 1, 1).is_err());
    assert!(RuntimeArchiveLimits::from_manifest(DEFAULT_MAX_PATH_BYTES + 1, 1, 1, 1).is_err());
    assert!(RuntimeArchiveLimits::from_manifest(1, DEFAULT_MAX_ENTRIES + 1, 1, 1).is_err());
}

use super::*;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn fixed_manifest_identity_accepts_four_targets_only() {
    for (target, filename) in [
        ("x86_64-pc-windows-msvc", "llama-b12757-bin-win-cpu-x64.zip"),
        (
            "aarch64-apple-darwin",
            "llama-b12757-bin-macos-arm64.tar.gz",
        ),
        ("x86_64-apple-darwin", "llama-b12757-bin-macos-x64.tar.gz"),
        (
            "x86_64-unknown-linux-gnu",
            "llama-b12757-bin-ubuntu-x64.tar.gz",
        ),
    ] {
        assert!(RuntimeArchive::new(target, filename, 1, "a".repeat(64)).is_ok());
    }
    let invalid = RuntimeArchive::new("x86_64-pc-windows-msvc", "other.zip", 1, "a".repeat(64));
    assert!(matches!(
        invalid,
        Err(RuntimeDownloadError::InvalidArchiveIdentity)
    ));
}

#[test]
fn release_download_url_uses_the_fixed_gitee_release_tag() {
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        1,
        "a".repeat(64),
    )
    .unwrap();

    assert_eq!(
        archive.url().unwrap().as_str(),
        "https://gitee.com/songyi777/llama-cpp/releases/download/b12757/llama-b12757-bin-win-cpu-x64.zip"
    );
}

#[test]
fn runtime_redirect_policy_stops_before_following_a_third_redirect() {
    assert!(!runtime_redirect_limit_reached(0));
    assert!(!runtime_redirect_limit_reached(1));
    assert!(!runtime_redirect_limit_reached(2));
    assert!(runtime_redirect_limit_reached(MAX_RUNTIME_REDIRECTS + 1));
}

#[test]
fn release_redirect_allowlist_accepts_only_secure_gitee_attachment_urls() {
    let filename = "llama-b12757-bin-win-cpu-x64.zip";
    let allowed = [
        "https://gitee.com/songyi777/llama-cpp/attach_files/3850728/download/llama-b12757-bin-win-cpu-x64.zip?signature=redacted",
        "https://foruda.gitee.com/attach_file/1795748766413673453/llama-b12757-bin-win-cpu-x64.zip?signature=redacted",
    ];
    for value in allowed {
        let url = Url::parse(value).unwrap();
        assert!(is_allowed_attachment_url(&url, filename), "{value}");
    }

    let rejected = [
        "http://gitee.com/songyi777/llama-cpp/attach_files/3850728/download/llama-b12757-bin-win-cpu-x64.zip",
        "https://gitee.com.attacker.invalid/songyi777/llama-cpp/attach_files/3850728/download/llama-b12757-bin-win-cpu-x64.zip",
        "https://foruda.gitee.com.attacker.invalid/attach_file/1795748766413673453/llama-b12757-bin-win-cpu-x64.zip",
        "https://gitee.com/songyi777/llama-cpp/attach_files/not-a-number/download/llama-b12757-bin-win-cpu-x64.zip",
        "https://foruda.gitee.com/attach_file/1795748766413673453/other.zip",
        "https://gitee.com:8443/songyi777/llama-cpp/attach_files/3850728/download/llama-b12757-bin-win-cpu-x64.zip",
    ];
    for value in rejected {
        let url = Url::parse(value).unwrap();
        assert!(!is_allowed_attachment_url(&url, filename), "{value}");
    }
}

#[test]
fn response_url_accepts_only_the_archive_release_attachment_redirect() {
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        1,
        "a".repeat(64),
    )
    .unwrap();
    let release_url = archive.url().unwrap();
    let final_url = Url::parse("https://foruda.gitee.com/attach_file/123/llama-b12757-bin-win-cpu-x64.zip?signature=redacted").unwrap();
    let wrong_file = Url::parse("https://foruda.gitee.com/attach_file/123/other.zip").unwrap();

    assert!(response_url_matches_archive(
        &archive,
        &release_url,
        &release_url
    ));
    assert!(response_url_matches_archive(
        &archive,
        &release_url,
        &final_url
    ));
    assert!(!response_url_matches_archive(
        &archive,
        &release_url,
        &wrong_file
    ));
    assert!(!response_url_matches_archive(
        &archive,
        &wrong_file,
        &final_url
    ));
}

#[tokio::test]
#[ignore = "需要真实访问 Gitee Release 网络资源"]
async fn gitee_release_artifacts_download_and_match_manifest_hashes(
) -> Result<(), Box<dyn std::error::Error>> {
    let targets = [
        (
            "x86_64-pc-windows-msvc",
            "llama-b12757-bin-win-cpu-x64.zip",
            25_262_139,
            "5482ac02398d674107b56b3b201485b15c9ee60c3b996f864c04f5367ece0c8d",
        ),
        (
            "aarch64-apple-darwin",
            "llama-b12757-bin-macos-arm64.tar.gz",
            16_862_081,
            "873b413c5d4a336ae623bedde8f755a3ed81b18bc6ae4b01f5a0f6fcea63754c",
        ),
        (
            "x86_64-apple-darwin",
            "llama-b12757-bin-macos-x64.tar.gz",
            12_591_660,
            "f330d7bf9baff5cc401c06524618f9eaf8962439dcb9e11fa638f8e3b0bee14b",
        ),
        (
            "x86_64-unknown-linux-gnu",
            "llama-b12757-bin-ubuntu-x64.tar.gz",
            23_425_964,
            "99e34cc6a39f3afda735d43d76d15d6ead2297406cc9c0cf421b771fd3a1a84a",
        ),
    ];
    let temp = tempfile::tempdir()?;
    let cancellation = CancellationToken::new();

    for (target, filename, size_bytes, sha256) in targets {
        let archive = RuntimeArchive::new(target, filename, size_bytes, sha256)?;
        let (progress, _) = watch::channel(RuntimeDownloadProgress::default());
        let verified =
            download_runtime_archive(&archive, temp.path(), &progress, &cancellation).await?;
        if verified.size_bytes() != size_bytes {
            return Err(format!(
                "{filename}: size mismatch: actual {}, expected {size_bytes}",
                verified.size_bytes()
            )
            .into());
        }
        if verified.sha256() != sha256 {
            return Err(format!(
                "{filename}: SHA-256 mismatch: actual {}, expected {sha256}",
                verified.sha256()
            )
            .into());
        }
    }
    Ok(())
}

#[test]
fn strong_etag_parser_rejects_weak_and_malformed_values() {
    assert!(is_strong_etag("\"stable\""));
    assert!(!is_strong_etag("W/\"weak\""));
    assert!(!is_strong_etag("\"\""));
}

#[test]
fn content_range_parser_rejects_wrong_units_and_overflow() {
    assert_eq!(
        parse_content_range("bytes 10-19/20"),
        Some(ParsedContentRange::Partial {
            start: 10,
            end: 19,
            total: 20
        })
    );
    assert!(parse_content_range("items 10-19/20").is_none());
    assert!(parse_content_range("bytes 10-19/18446744073709551616").is_none());
}

#[tokio::test]
async fn streams_and_verifies_full_archive_then_reuses_verified_file() {
    let body = b"small fixed runtime archive".to_vec();
    let sha256 = hex::encode(Sha256::digest(&body));
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        body.len() as u64,
        sha256.clone(),
    )
    .unwrap();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/runtime.zip")
        .with_status(200)
        .with_header("etag", "\"entity-1\"")
        .with_body(body.clone())
        .expect(1)
        .create_async()
        .await;
    let temp = tempfile::tempdir().unwrap();
    let (progress, receiver) = watch::channel(RuntimeDownloadProgress::default());
    let cancel = CancellationToken::new();
    let url = Url::parse(&format!("{}/runtime.zip", server.url())).unwrap();

    let result =
        download_runtime_archive_at(&archive, temp.path(), url.clone(), &progress, &cancel)
            .await
            .unwrap();
    assert_eq!(result.size_bytes(), body.len() as u64);
    assert_eq!(result.sha256(), sha256);
    assert_eq!(fs::read(result.path()).await.unwrap(), body);
    assert_eq!(receiver.borrow().completed_bytes, body.len() as u64);
    mock.assert_async().await;

    let cached = download_runtime_archive_at(&archive, temp.path(), url, &progress, &cancel)
        .await
        .unwrap();
    assert_eq!(cached.path(), result.path());
}

#[tokio::test]
async fn range_response_appends_only_matching_etag_and_content_range() {
    let body = b"abcdefghij";
    let sha256 = hex::encode(Sha256::digest(body));
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        body.len() as u64,
        sha256,
    )
    .unwrap();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/runtime.zip")
        .match_header("range", "bytes=6-")
        .match_header("if-range", "\"entity-1\"")
        .with_status(206)
        .with_header("etag", "\"entity-1\"")
        .with_header("content-range", "bytes 6-9/10")
        .with_body("ghij")
        .create_async()
        .await;
    let temp = tempfile::tempdir().unwrap();
    let part = temp.path().join("llama-b12757-bin-win-cpu-x64.zip.part");
    let sidecar = temp
        .path()
        .join("llama-b12757-bin-win-cpu-x64.zip.part.json");
    fs::write(&part, b"abcdef").await.unwrap();
    let url = Url::parse(&format!("{}/runtime.zip", server.url())).unwrap();
    let metadata = RuntimePartialMetadata {
        schema_version: 1,
        provider: PROVIDER.into(),
        repository: REPOSITORY.into(),
        commit: COMMIT.into(),
        target: archive.target.clone(),
        filename: archive.filename.clone(),
        url: url.to_string(),
        expected_size: archive.size_bytes,
        expected_sha256: archive.sha256.clone(),
        strong_etag: "\"entity-1\"".into(),
        total_bytes: archive.size_bytes,
        committed_bytes: 6,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    fs::write(&sidecar, serde_json::to_vec(&metadata).unwrap())
        .await
        .unwrap();
    let (progress, _) = watch::channel(RuntimeDownloadProgress::default());
    let result = download_runtime_archive_at(
        &archive,
        temp.path(),
        url,
        &progress,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(result.path()).await.unwrap(), body);
    mock.assert_async().await;
}

#[tokio::test]
async fn range_200_response_discards_old_prefix_before_writing_full_entity() {
    let body = b"new-complete-entity";
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        body.len() as u64,
        hex::encode(Sha256::digest(body)),
    )
    .unwrap();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/runtime.zip")
        .match_header("range", "bytes=4-")
        .with_status(200)
        .with_header("etag", "\"entity-2\"")
        .with_body(body)
        .create_async()
        .await;
    let temp = tempfile::tempdir().unwrap();
    let url = Url::parse(&format!("{}/runtime.zip", server.url())).unwrap();
    seed_partial(temp.path(), &archive, &url, b"old!", "\"entity-1\"").await;
    let (progress, _) = watch::channel(RuntimeDownloadProgress::default());
    let result = download_runtime_archive_at(
        &archive,
        temp.path(),
        url,
        &progress,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(result.path()).await.unwrap(), body);
    mock.assert_async().await;
}

#[tokio::test]
async fn forbidden_and_redirect_responses_do_not_fall_back_to_another_source() {
    for (path, status) in [
        ("/forbidden.zip", 403u16),
        ("/missing.zip", 404u16),
        ("/redirect.zip", 302u16),
    ] {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", path)
            .with_status(status as usize)
            .create_async()
            .await;
        let archive = RuntimeArchive::new(
            "x86_64-pc-windows-msvc",
            "llama-b12757-bin-win-cpu-x64.zip",
            1,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let url = Url::parse(&format!("{}{}", server.url(), path)).unwrap();
        let (progress, _) = watch::channel(RuntimeDownloadProgress::default());
        let result = download_runtime_archive_at(
            &archive,
            temp.path(),
            url,
            &progress,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(result, Err(RuntimeDownloadError::SourceUnavailable(code)) if code == status)
        );
        mock.assert_async().await;
    }
}

#[tokio::test]
async fn checksum_mismatch_is_retried_without_promoting_the_bad_archive() {
    let body = b"wrong-content";
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        body.len() as u64,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/runtime.zip")
        .with_status(200)
        .with_body(body)
        .expect(3)
        .create_async()
        .await;
    let temp = tempfile::tempdir().unwrap();
    let url = Url::parse(&format!("{}/runtime.zip", server.url())).unwrap();
    let (progress, _) = watch::channel(RuntimeDownloadProgress::default());
    let result = download_runtime_archive_at(
        &archive,
        temp.path(),
        url,
        &progress,
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(
        result,
        Err(RuntimeDownloadError::ChecksumMismatch)
    ));
    assert!(!temp.path().join(archive.filename()).exists());
    mock.assert_async().await;
}

#[test]
fn runtime_download_errors_map_to_stable_codes() {
    let cases = [
        (
            RuntimeDownloadError::InvalidArchiveIdentity,
            "runtime_manifest_invalid",
        ),
        (
            RuntimeDownloadError::InvalidDownloadDirectory,
            "runtime_manifest_invalid",
        ),
        (
            RuntimeDownloadError::SourceUnavailable(404),
            "runtime_source_unavailable",
        ),
        (
            RuntimeDownloadError::ServerStatus(503),
            "runtime_download_failed",
        ),
        (
            RuntimeDownloadError::NoProgressTimeout,
            "runtime_download_failed",
        ),
        (
            RuntimeDownloadError::InvalidResponse,
            "runtime_download_failed",
        ),
        (RuntimeDownloadError::SizeMismatch, "runtime_size_mismatch"),
        (
            RuntimeDownloadError::ChecksumMismatch,
            "runtime_checksum_mismatch",
        ),
        (RuntimeDownloadError::Cancelled, "cancelled"),
        (
            RuntimeDownloadError::RetriesExhausted,
            "runtime_download_failed",
        ),
        (
            RuntimeDownloadError::Io(std::io::Error::other("测试 I/O 错误")),
            "runtime_download_failed",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.code(), expected);
    }
}

#[tokio::test]
async fn public_download_validates_directory_and_observes_pre_cancel() {
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        1,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let (progress, _) = watch::channel(RuntimeDownloadProgress::default());

    let invalid = download_runtime_archive(
        &archive,
        Path::new("relative-runtime-cache"),
        &progress,
        &cancellation,
    )
    .await;
    assert!(matches!(
        invalid,
        Err(RuntimeDownloadError::InvalidDownloadDirectory)
    ));

    let temp = tempfile::tempdir().unwrap();
    let cancelled = download_runtime_archive(&archive, temp.path(), &progress, &cancellation).await;
    assert!(matches!(cancelled, Err(RuntimeDownloadError::Cancelled)));
}

#[test]
fn invalid_content_length_is_not_treated_as_an_absent_header() {
    let mut headers = HeaderMap::new();
    assert!(matches!(content_length_matches(&headers, 8), Ok(true)));

    headers.insert(
        CONTENT_LENGTH,
        reqwest::header::HeaderValue::from_static("invalid"),
    );
    assert!(matches!(
        content_length_matches(&headers, 8),
        Err(RuntimeDownloadError::InvalidResponse)
    ));

    headers.insert(
        CONTENT_LENGTH,
        reqwest::header::HeaderValue::from_static("7"),
    );
    assert!(matches!(content_length_matches(&headers, 8), Ok(false)));
}

#[tokio::test]
async fn interrupted_stream_retries_from_the_last_synced_range() {
    let prefix = vec![b'a'; CHECKPOINT_INTERVAL_BYTES as usize];
    let suffix = b"tail";
    let mut body = prefix.clone();
    body.extend_from_slice(suffix);
    let body_len = body.len();
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        body_len as u64,
        hex::encode(Sha256::digest(&body)),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let _first_request = read_request_headers(&mut first).await;
        first
            .write_all(
                b"HTTP/1.1 200 OK\r\nETag: \"entity-1\"\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        first
            .write_all(format!("{:X}\r\n", prefix.len()).as_bytes())
            .await
            .unwrap();
        first.write_all(&prefix).await.unwrap();
        first.write_all(b"\r\n").await.unwrap();
        first.flush().await.unwrap();
        drop(first);

        let (mut second, _) = listener.accept().await.unwrap();
        let request = read_request_headers(&mut second).await;
        let range_matches = request
            .to_ascii_lowercase()
            .contains(&format!("range: bytes={}-", CHECKPOINT_INTERVAL_BYTES))
            && request
                .to_ascii_lowercase()
                .contains("if-range: \"entity-1\"");
        let response = format!(
            "HTTP/1.1 206 Partial Content\r\nETag: \"entity-1\"\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            prefix.len(),
            body_len - 1,
            body_len,
            suffix.len()
        );
        second.write_all(response.as_bytes()).await.unwrap();
        second.write_all(suffix).await.unwrap();
        second.flush().await.unwrap();
        range_matches
    });

    let temp = tempfile::tempdir().unwrap();
    let url = Url::parse(&format!("http://{address}/runtime.zip")).unwrap();
    let (progress, _) = watch::channel(RuntimeDownloadProgress::default());
    let result = download_runtime_archive_at(
        &archive,
        temp.path(),
        url,
        &progress,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(result.path()).await.unwrap(), body);
    assert!(server.await.unwrap(), "重试请求必须从已同步长度续传");
}

#[tokio::test]
async fn cancelling_an_active_stream_keeps_only_the_synced_contiguous_prefix() {
    let body = vec![b'x'; CHECKPOINT_INTERVAL_BYTES as usize];
    let body_len = body.len();
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        body_len as u64 + 64,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _request = read_request_headers(&mut socket).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nETag: \"entity-1\"\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n",
            )
            .await
            .unwrap();
        socket
            .write_all(format!("{:X}\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        socket.write_all(&body).await.unwrap();
        socket.write_all(b"\r\n").await.unwrap();
        socket.flush().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let temp = tempfile::tempdir().unwrap();
    let url = Url::parse(&format!("http://{address}/runtime.zip")).unwrap();
    let (progress, mut receiver) = watch::channel(RuntimeDownloadProgress::default());
    let cancellation = CancellationToken::new();
    let download = tokio::spawn({
        let archive = archive.clone();
        let directory = temp.path().to_path_buf();
        let url = url.clone();
        let progress = progress.clone();
        let cancellation = cancellation.clone();
        async move {
            download_runtime_archive_at(&archive, &directory, url, &progress, &cancellation).await
        }
    });

    tokio::time::timeout(Duration::from_secs(10), async {
        while receiver.borrow().completed_bytes < body_len as u64 {
            receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(10), download)
            .await
            .unwrap()
            .unwrap(),
        Err(RuntimeDownloadError::Cancelled)
    ));
    server.abort();

    let part_path = temp.path().join(format!("{}.part", archive.filename()));
    let sidecar_path = temp
        .path()
        .join(format!("{}.part.json", archive.filename()));
    assert_eq!(fs::read(part_path).await.unwrap(), vec![b'x'; body_len]);
    let metadata: RuntimePartialMetadata =
        serde_json::from_slice(&fs::read(sidecar_path).await.unwrap()).unwrap();
    assert_eq!(metadata.committed_bytes, CHECKPOINT_INTERVAL_BYTES);
    assert_eq!(metadata.strong_etag, "\"entity-1\"");
}

async fn read_request_headers(socket: &mut TcpStream) -> String {
    use tokio::io::AsyncReadExt as _;

    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "HTTP 请求应包含 header terminator");
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(request).unwrap();
        }
    }
}

#[tokio::test]
async fn cancellation_is_reported_before_network_access() {
    let archive = RuntimeArchive::new(
        "x86_64-pc-windows-msvc",
        "llama-b12757-bin-win-cpu-x64.zip",
        1,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let url = Url::parse("http://127.0.0.1:9/runtime.zip").unwrap();
    let (progress, _) = watch::channel(RuntimeDownloadProgress::default());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result =
        download_runtime_archive_at(&archive, temp.path(), url, &progress, &cancellation).await;
    assert!(matches!(result, Err(RuntimeDownloadError::Cancelled)));
}

async fn seed_partial(
    directory: &Path,
    archive: &RuntimeArchive,
    url: &Url,
    prefix: &[u8],
    etag: &str,
) {
    let part_path = directory.join(format!("{}.part", archive.filename));
    let sidecar_path = directory.join(format!("{}.part.json", archive.filename));
    fs::write(part_path, prefix).await.unwrap();
    let metadata = RuntimePartialMetadata {
        schema_version: 1,
        provider: PROVIDER.into(),
        repository: REPOSITORY.into(),
        commit: COMMIT.into(),
        target: archive.target.clone(),
        filename: archive.filename.clone(),
        url: url.to_string(),
        expected_size: archive.size_bytes,
        expected_sha256: archive.sha256.clone(),
        strong_etag: etag.into(),
        total_bytes: archive.size_bytes,
        committed_bytes: prefix.len() as u64,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    fs::write(sidecar_path, serde_json::to_vec(&metadata).unwrap())
        .await
        .unwrap();
}

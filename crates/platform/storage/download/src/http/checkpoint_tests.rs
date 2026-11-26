#![allow(
    clippy::panic_in_result_fn,
    reason = "测试用 Result 传播文件夹具错误，断言用于报告行为不符合预期"
)]

use super::*;
use std::error::Error;
use tokio::fs;

const ORIGINAL_URL: &str = "https://source.example/model.gguf";
const ENTITY_URL: &str = "https://cdn.example/model.gguf";

fn metadata(committed: u64, total: u64) -> ResumeMetadata {
    ResumeMetadata {
        task_id: 41,
        url: ORIGINAL_URL.into(),
        entity_url: ENTITY_URL.into(),
        strong_etag: Some("\"revision-1\"".into()),
        total_bytes: Some(total),
        committed_bytes: committed,
    }
}

fn response<'a>(url: &'a Url, entity: &'a Url, status: u16) -> ResponseHeaders<'a> {
    ResponseHeaders {
        status,
        url,
        entity_url: entity,
        etag: Some("\"revision-1\""),
        content_range: None,
        content_length: None,
    }
}

#[test]
fn only_matching_strong_entity_and_continuous_suffix_can_append() -> Result<(), Box<dyn Error>> {
    let url = Url::parse(ORIGINAL_URL)?;
    let entity = Url::parse(ENTITY_URL)?;
    let changed = Url::parse("https://other.example/model.gguf")?;
    let checkpoint = metadata(10, 15);
    let mut headers = response(&url, &entity, 206);
    headers.content_range = Some("bytes 10-14/15");
    headers.content_length = Some(5);
    assert_eq!(
        resume_decision(Some(&checkpoint), &headers, 10)?,
        TransferMode::Append {
            expected_body_bytes: 5
        },
    );
    assert_eq!(
        resume_decision(None, &headers, 10)?,
        TransferMode::RestartRequest
    );
    assert_eq!(
        resume_decision(Some(&checkpoint), &headers, 9)?,
        TransferMode::RestartRequest
    );

    for etag in [
        None,
        Some("W/\"revision-1\""),
        Some("\"revision-2\""),
        Some("revision-1"),
    ] {
        headers.etag = etag;
        assert_eq!(
            resume_decision(Some(&checkpoint), &headers, 10)?,
            TransferMode::RestartRequest
        );
    }
    headers.etag = checkpoint.strong_etag.as_deref();
    for content_range in [
        None,
        Some("items 10-14/15"),
        Some("bytes 9-14/15"),
        Some("bytes 10-13/15"),
        Some("bytes 10-15/16"),
        Some("bytes 10-14/*"),
        Some("bytes 10-14/+15"),
        Some("bytes 10-18446744073709551615/15"),
    ] {
        headers.content_range = content_range;
        assert_eq!(
            resume_decision(Some(&checkpoint), &headers, 10)?,
            TransferMode::RestartRequest
        );
    }
    headers.content_range = Some("bytes 10-14/15");
    headers.content_length = Some(4);
    assert!(matches!(
        resume_decision(Some(&checkpoint), &headers, 10),
        Err(DownloadError::Failed(_))
    ));
    headers.content_length = Some(5);
    headers.url = &changed;
    assert_eq!(
        resume_decision(Some(&checkpoint), &headers, 10)?,
        TransferMode::RestartRequest
    );
    headers.url = &url;
    headers.entity_url = &changed;
    assert_eq!(
        resume_decision(Some(&checkpoint), &headers, 10)?,
        TransferMode::RestartRequest
    );
    Ok(())
}

#[test]
fn full_response_restarts_and_only_proven_complete_416_enters_verification(
) -> Result<(), Box<dyn Error>> {
    let url = Url::parse(ORIGINAL_URL)?;
    let entity = Url::parse(ENTITY_URL)?;
    let checkpoint = metadata(10, 10);
    let mut headers = response(&url, &entity, 200);
    headers.content_length = Some(10);
    assert_eq!(
        resume_decision(Some(&checkpoint), &headers, 10)?,
        TransferMode::FromZero {
            expected_body_bytes: Some(10)
        }
    );
    headers.status = 416;
    headers.content_range = Some("bytes */10");
    assert_eq!(
        resume_decision(Some(&checkpoint), &headers, 10)?,
        TransferMode::VerifyComplete
    );
    assert_eq!(
        resume_decision(Some(&metadata(10, 11)), &headers, 10)?,
        TransferMode::RestartRequest
    );
    for content_range in [None, Some("bytes */11"), Some("bytes 0-9/10")] {
        headers.content_range = content_range;
        assert_eq!(
            resume_decision(Some(&checkpoint), &headers, 10)?,
            TransferMode::RestartRequest
        );
    }
    headers.content_range = Some("bytes */10");
    headers.etag = None;
    assert_eq!(
        resume_decision(Some(&checkpoint), &headers, 10)?,
        TransferMode::RestartRequest
    );
    headers.status = 503;
    assert!(matches!(
        resume_decision(Some(&checkpoint), &headers, 10),
        Err(DownloadError::Failed(_))
    ));
    Ok(())
}

#[test]
fn short_or_long_response_is_not_complete() -> Result<(), Box<dyn Error>> {
    validate_body_length(Some(5), 5)?;
    validate_body_length(None, 5)?;
    for actual in [0, 4, 6] {
        assert!(matches!(
            validate_body_length(Some(5), actual),
            Err(DownloadError::Failed(_))
        ));
    }
    Ok(())
}

#[tokio::test]
async fn recovery_discards_trailing_uncommitted_bytes_and_restarts_after_truncation(
) -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("model.gguf.part");
    let sidecar = directory.path().join("model.gguf.part.json");
    let mut file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .await?;
    file.write_all(b"abc").await?;
    persist_checkpoint(&mut file, &metadata(3, 7), &sidecar).await?;
    file.write_all(b"de").await?;
    persist_checkpoint(&mut file, &metadata(5, 7), &sidecar).await?;
    file.write_all(b"xx").await?;
    file.flush().await?;
    drop(file);

    let url = Url::parse(ORIGINAL_URL)?;
    let mut file = File::options().read(true).write(true).open(&path).await?;
    assert_eq!(
        restore_checkpoint(&mut file, &sidecar, 41, &url).await?,
        Some(metadata(5, 7))
    );
    assert_eq!(file.stream_position().await?, 5);
    assert_eq!(fs::read(&path).await?, b"abcde");
    file.set_len(4).await?;
    assert_eq!(
        restore_checkpoint(&mut file, &sidecar, 41, &url).await?,
        None
    );
    assert_eq!(file.metadata().await?.len(), 0);
    assert_eq!(file.stream_position().await?, 0);
    Ok(())
}

#[tokio::test]
async fn missing_damaged_or_foreign_evidence_never_reuses_file_length() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("model.gguf.part");
    let sidecar = directory.path().join("model.gguf.part.json");
    let mut file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .await?;
    let mut wrong_task = metadata(5, 5);
    wrong_task.task_id += 1;
    let url = Url::parse(ORIGINAL_URL)?;
    let mut weak = metadata(5, 5);
    weak.strong_etag = Some("W/\"revision-1\"".into());
    let cases = [
        None,
        Some(b"not-json".to_vec()),
        Some(serde_json::to_vec(&wrong_task)?),
        Some(serde_json::to_vec(&weak)?),
    ];
    for payload in cases {
        // 旧分片的逻辑长度可以大于落盘的连续内容。
        file.set_len(5).await?;
        if let Some(payload) = payload {
            fs::write(&sidecar, payload).await?;
        }
        assert_eq!(
            restore_checkpoint(&mut file, &sidecar, 41, &url).await?,
            None
        );
        assert_eq!(file.metadata().await?.len(), 0);
    }
    Ok(())
}

#[tokio::test]
async fn checkpoint_io_failure_does_not_replace_committed_evidence() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("model.gguf.part");
    let sidecar = directory.path().join("model.gguf.part.json");
    let mut file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .await?;
    file.write_all(b"abc").await?;
    persist_checkpoint(&mut file, &metadata(3, 5), &sidecar).await?;
    file.write_all(b"de").await?;
    let mut oversized = metadata(5, 5);
    oversized.entity_url.push_str(&"a".repeat(64 * 1024));
    assert!(persist_checkpoint(&mut file, &oversized, &sidecar)
        .await
        .is_err());
    assert!(persist_checkpoint(
        &mut file,
        &metadata(5, 5),
        &directory.path().join("missing/part.json")
    )
    .await
    .is_err());
    let non_file = directory.path().join("directory.json");
    fs::create_dir(&non_file).await?;
    assert!(read_checkpoint(&non_file).await.is_err());
    assert_eq!(read_checkpoint(&sidecar).await?, Some(metadata(3, 5)));
    Ok(())
}

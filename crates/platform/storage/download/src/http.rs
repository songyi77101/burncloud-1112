use crate::error::DownloadError;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};
use tempfile::NamedTempFile;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};
use url::Url;

// 侧车只记录已同步的连续字节，不能作为完整工件的摘要证明。
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeMetadata {
    task_id: i64,
    url: String,
    entity_url: String,
    strong_etag: Option<String>,
    total_bytes: Option<u64>,
    committed_bytes: u64,
}

struct ResponseHeaders<'a> {
    status: u16,
    url: &'a Url,
    entity_url: &'a Url,
    etag: Option<&'a str>,
    content_range: Option<&'a str>,
    content_length: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
enum TransferMode {
    FromZero { expected_body_bytes: Option<u64> },
    Append { expected_body_bytes: u64 },
    RestartRequest,
    VerifyComplete,
}

#[derive(Debug, PartialEq, Eq)]
enum ContentRange {
    Partial { start: u64, end: u64, total: u64 },
    Unsatisfied { total: u64 },
}

const MAX_CHECKPOINT_BYTES: u64 = 64 * 1024;

fn resume_decision(
    metadata: Option<&ResumeMetadata>,
    response: &ResponseHeaders<'_>,
    file_length: u64,
) -> Result<TransferMode, DownloadError> {
    match response.status {
        // 收到 200 时即使实体相同也只能从零写入，不能消费旧断点。
        200 if response.content_range.is_none() => Ok(TransferMode::FromZero {
            expected_body_bytes: response.content_length,
        }),
        200 => Err(DownloadError::Failed(
            "HTTP 200 完整响应不应包含 Content-Range".into(),
        )),
        206 | 416 => resume_range(metadata, response, file_length),
        status => Err(DownloadError::Failed(format!(
            "文件传输返回非预期的 HTTP 状态：{status}"
        ))),
    }
}

fn resume_range(
    metadata: Option<&ResumeMetadata>,
    response: &ResponseHeaders<'_>,
    file_length: u64,
) -> Result<TransferMode, DownloadError> {
    let Some(checkpoint) = metadata.filter(|checkpoint| {
        file_length > 0
            && checkpoint.committed_bytes == file_length
            && checkpoint.url == response.url.as_str()
            && checkpoint.entity_url == response.entity_url.as_str()
            && response.etag.is_some_and(|etag| {
                is_strong_etag(etag) && checkpoint.strong_etag.as_deref() == Some(etag)
            })
    }) else {
        return Ok(TransferMode::RestartRequest);
    };

    let mode = match (
        response.status,
        response.content_range.and_then(parse_content_range),
    ) {
        (206, Some(ContentRange::Partial { start, end, total }))
            if start == file_length
                && start <= end
                && end.checked_add(1) == Some(total)
                && checkpoint.total_bytes.is_none_or(|known| known == total) =>
        {
            let expected_body_bytes = total - start;
            if let Some(length) = response.content_length {
                validate_body_length(Some(expected_body_bytes), length)?;
            }
            TransferMode::Append {
                expected_body_bytes,
            }
        }
        (416, Some(ContentRange::Unsatisfied { total }))
            if total == file_length
                && checkpoint.total_bytes.is_none_or(|known| known == total) =>
        {
            TransferMode::VerifyComplete
        }
        // 206/416 的响应体不是完整文件，必须另发不带 Range 的请求。
        _ => TransferMode::RestartRequest,
    };
    Ok(mode)
}

fn is_strong_etag(value: &str) -> bool {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .is_some_and(|tag| {
            tag.bytes()
                .all(|byte| matches!(byte, 0x21 | 0x23..=0x7e | 0x80..=0xff))
        })
}

fn parse_content_range(value: &str) -> Option<ContentRange> {
    let (unit, spec) = value.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (positions, total) = spec.split_once('/')?;
    let total = parse_byte_position(total)?;
    if positions == "*" {
        return Some(ContentRange::Unsatisfied { total });
    }
    let (start, end) = positions.split_once('-')?;
    Some(ContentRange::Partial {
        start: parse_byte_position(start)?,
        end: parse_byte_position(end)?,
        total,
    })
}

fn parse_byte_position(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn validate_body_length(expected: Option<u64>, received: u64) -> Result<(), DownloadError> {
    if let Some(expected) = expected.filter(|expected| *expected != received) {
        return Err(DownloadError::Failed(format!(
            "下载响应体长度不一致：预期 {expected} 字节，实际收到 {received} 字节"
        )));
    }
    Ok(())
}

async fn restore_checkpoint(
    file: &mut File,
    sidecar: &Path,
    task_id: i64,
    url: &Url,
) -> Result<Option<ResumeMetadata>, DownloadError> {
    let actual_length = file.metadata().await?.len();
    let checkpoint = read_checkpoint(sidecar).await?.filter(|checkpoint| {
        checkpoint.task_id == task_id
            && checkpoint.url == url.as_str()
            && has_entity_url(&checkpoint.entity_url)
            && checkpoint
                .strong_etag
                .as_deref()
                .is_some_and(is_strong_etag)
            && checkpoint.committed_bytes > 0
            && checkpoint.committed_bytes <= actual_length
            && checkpoint
                .total_bytes
                .is_none_or(|total| checkpoint.committed_bytes <= total)
    });
    let committed = checkpoint
        .as_ref()
        .map_or(0, |checkpoint| checkpoint.committed_bytes);
    // 文件可能在侧车提交前已写入额外字节；没有有效侧车的部分文件不能提供连续性证据。
    if actual_length != committed {
        file.set_len(committed).await?;
    }
    file.seek(io::SeekFrom::Start(committed)).await?;
    Ok(checkpoint)
}

fn has_entity_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}

async fn read_checkpoint(sidecar: &Path) -> io::Result<Option<ResumeMetadata>> {
    let file = match File::open(sidecar).await {
        Ok(file) => file,
        // 不存在侧车是首次下载和旧 aria2 任务的正常入口。
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut payload = Vec::new();
    file.take(MAX_CHECKPOINT_BYTES + 1)
        .read_to_end(&mut payload)
        .await?;
    if payload.len() as u64 > MAX_CHECKPOINT_BYTES {
        tracing::warn!("下载断点侧车超过大小上限，从零开始");
        return Ok(None);
    }
    match serde_json::from_slice(&payload) {
        Ok(checkpoint) => Ok(Some(checkpoint)),
        Err(error) => {
            // 损坏或旧格式不能当作连续性证据，也不能将原始侧车内容写入日志。
            tracing::warn!(
                line = error.line(),
                column = error.column(),
                "下载断点侧车格式无效，从零开始"
            );
            Ok(None)
        }
    }
}

async fn persist_checkpoint(
    file: &mut File,
    metadata: &ResumeMetadata,
    sidecar: &Path,
) -> Result<(), DownloadError> {
    file.flush().await?;
    file.sync_data().await?;
    let payload = serde_json::to_vec(metadata).map_err(io::Error::other)?;
    if payload.len() as u64 > MAX_CHECKPOINT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "下载断点侧车超过大小上限").into());
    }
    let sidecar = sidecar.to_path_buf();
    tokio::task::spawn_blocking(move || replace_checkpoint(&sidecar, &payload))
        .await
        .map_err(io::Error::other)??;
    Ok(())
}

fn replace_checkpoint(sidecar: &Path, payload: &[u8]) -> io::Result<()> {
    use std::io::Write as _;

    let parent = sidecar
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "下载断点侧车缺少父目录"))?;
    // 同目录暂存可避免跨盘重命名；若写入或同步失败，旧检查点仍然可用。
    let mut staging = NamedTempFile::new_in(parent)?;
    staging.write_all(payload)?;
    staging.as_file().sync_all()?;
    drop(staging.persist(sidecar).map_err(|failure| failure.error)?);
    Ok(())
}

#[cfg(test)]
mod checkpoint_tests;

//! BurnCloud 数据面真实推理与 OpenAI SSE 解码。

use super::{
    backend::{BackendFuture, Callback},
    catalog::CatalogClient,
    model::{
        ChatError, ErrorKind, GenerationEvent, GenerationRequest, HistoryMessage, InferenceMetrics,
        MessageRole, Operation,
    },
};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::time::Instant;

const MAX_SSE_BUFFER_BYTES: usize = 1024 * 1024;
const MAX_SSE_CHUNK_BYTES: usize = 256 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize)]
struct RouterRequest {
    model: String,
    messages: Vec<RouterMessage>,
    stream: bool,
}

#[derive(Clone, Debug, Serialize)]
struct RouterMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    _prompt_tokens: Option<u64>,
    _completion_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SseEnvelope {
    #[serde(default)]
    choices: Vec<SseChoice>,
    usage: Option<Usage>,
    error: Option<SseError>,
}

#[derive(Debug, Deserialize)]
struct SseChoice {
    delta: Option<SseDelta>,
    message: Option<SseDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SseDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    reasoning: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SseError {
    message: Option<String>,
}
#[derive(Debug, Default)]
struct ParsedSse {
    events: Vec<GenerationEvent>,
    done: bool,
    usage: Option<Usage>,
}

/// 通过 BurnCloud 数据面生成目录模型回复。
///
/// # Examples
///
/// 该函数只接受已完成部署的请求和本次调用内存中的数据面令牌，令牌不会写入会话或日志。
///
/// # Notes
///
/// 请求始终发送到 `/v1/chat/completions`；本地 `llama-server` 地址不会暴露给客户端，也不会
/// 在真实请求失败时回退到模拟文本。
pub(super) fn generate_via_router(
    client: &CatalogClient,
    request: GenerationRequest,
    api_token: &str,
    callback: Callback<GenerationEvent>,
) -> BackendFuture<Option<InferenceMetrics>> {
    let http = client.http_client().clone();
    let url = client.router_url();
    let token = api_token.to_string();
    Box::pin(async move {
        let url = url?;
        generate_with_client(&http, &url, request, &token, callback).await
    })
}

async fn generate_with_client(
    client: &Client,
    base: &Url,
    request: GenerationRequest,
    api_token: &str,
    mut callback: Callback<GenerationEvent>,
) -> Result<Option<InferenceMetrics>, ChatError> {
    if api_token.trim().is_empty()
        || !request.prepared.is_valid()
        || request.messages.is_empty()
        || request.messages.iter().any(|message| {
            message.content.trim().is_empty() || matches!(message.role, MessageRole::System)
        })
    {
        return Err(failure(
            ErrorKind::InvalidParameter,
            "真实推理请求缺少有效模型、消息或数据面令牌",
        ));
    }

    let model = request
        .prepared
        .route_model_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::path::Path::new(&request.prepared.local_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| {
            failure(
                ErrorKind::InvalidParameter,
                "部署结果没有有效的路由模型标识",
            )
        })?;
    let messages = request
        .messages
        .into_iter()
        .map(router_message)
        .collect::<Result<Vec<_>, _>>()?;
    let payload = RouterRequest {
        model: model.to_string(),
        messages,
        stream: true,
    };
    let url = base.join("/v1/chat/completions").map_err(|error| {
        failure(
            ErrorKind::InvalidParameter,
            format!("数据面地址无效：{error}"),
        )
    })?;
    let started = Instant::now();
    let response = client
        .post(url)
        .bearer_auth(api_token)
        .header("Accept", "text/event-stream")
        .json(&payload)
        .send()
        .await
        .map_err(|error| transport(error.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = read_bounded_body(response).await?;
        return Err(failure(
            if status == reqwest::StatusCode::UNAUTHORIZED {
                ErrorKind::NotConnected
            } else {
                ErrorKind::ExecutionFailed
            },
            format!(
                "数据面请求失败（HTTP {status}）：{}",
                summarize_error(&body)
            ),
        ));
    }

    let mut decoder = SseDecoder::default();
    let mut first_event_at = None;
    let mut usage = None;
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| transport(error.to_string()))?
    {
        if chunk.len() > MAX_SSE_CHUNK_BYTES {
            return Err(failure(
                ErrorKind::InvalidResult,
                "数据面 SSE 数据块超过大小限制",
            ));
        }
        let parsed = decoder.feed(&chunk)?;
        if first_event_at.is_none() && !parsed.events.is_empty() {
            first_event_at = Some(Instant::now());
        }
        for event in parsed.events {
            callback(event);
        }
        if parsed.usage.is_some() {
            usage = parsed.usage;
        }
        if parsed.done {
            break;
        }
    }
    let parsed = decoder.finish()?;
    for event in parsed.events {
        callback(event);
    }
    if parsed.usage.is_some() {
        usage = parsed.usage;
    }
    if !parsed.done && !decoder.done {
        return Err(failure(
            ErrorKind::InvalidResult,
            "数据面 SSE 在完成标记前结束",
        ));
    }

    // 数据面当前未提供可验证的成本字段；宁可不显示指标，也不伪造成本或速度。
    let _elapsed = started.elapsed();
    let _first_event_at = first_event_at;
    let _usage = usage;
    Ok(None)
}

fn router_message(message: HistoryMessage) -> Result<RouterMessage, ChatError> {
    let role = match message.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => {
            return Err(failure(
                ErrorKind::InvalidParameter,
                "数据面请求不接受系统消息",
            ))
        }
    };
    Ok(RouterMessage {
        role,
        content: message.content,
    })
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
    done: bool,
}

impl SseDecoder {
    fn feed(&mut self, chunk: &[u8]) -> Result<ParsedSse, ChatError> {
        if chunk.len() > MAX_SSE_CHUNK_BYTES
            || self.buffer.len().saturating_add(chunk.len()) > MAX_SSE_BUFFER_BYTES
        {
            return Err(failure(
                ErrorKind::InvalidResult,
                "数据面 SSE 缓冲区超过大小限制",
            ));
        }
        self.buffer.extend_from_slice(chunk);
        self.parse_complete_frames()
    }

    fn finish(&mut self) -> Result<ParsedSse, ChatError> {
        if self.buffer.iter().any(|byte| !byte.is_ascii_whitespace()) {
            return Err(failure(ErrorKind::InvalidResult, "数据面 SSE 存在未完成帧"));
        }
        Ok(ParsedSse {
            done: self.done,
            ..ParsedSse::default()
        })
    }

    fn parse_complete_frames(&mut self) -> Result<ParsedSse, ChatError> {
        let mut result = ParsedSse {
            done: self.done,
            ..ParsedSse::default()
        };
        while let Some(end) = find_frame_end(&self.buffer) {
            let frame: Vec<u8> = self.buffer.drain(..end).collect();
            let parsed = parse_frame(&frame)?;
            result.events.extend(parsed.events);
            result.usage = parsed.usage.or(result.usage);
            if parsed.done {
                self.done = true;
                result.done = true;
            }
        }
        Ok(result)
    }
}

/// 解析跨网络分块的 OpenAI 兼容 SSE。
///
/// # Examples
///
/// ```text
/// data: {"choices":[{"delta":{"content":"hi"}}]}
///
/// data: [DONE]
/// ```
///
/// # Notes
///
/// 该函数只暴露给聊天模块测试支持；生产推理使用同一解析器的有状态包装，以便验证 `[DONE]`。
#[allow(
    dead_code,
    reason = "生产路径使用有状态 SSE 解码器，保留此无状态入口供模块回归测试使用"
)]
pub(super) fn decode_sse(
    buffer: &mut Vec<u8>,
    chunk: &[u8],
) -> Result<Vec<GenerationEvent>, ChatError> {
    let mut decoder = SseDecoder {
        buffer: std::mem::take(buffer),
        done: false,
    };
    let parsed = decoder.feed(chunk)?;
    *buffer = decoder.buffer;
    Ok(parsed.events)
}

fn parse_frame(frame: &[u8]) -> Result<ParsedSse, ChatError> {
    let text = std::str::from_utf8(frame)
        .map_err(|_| failure(ErrorKind::InvalidResult, "数据面 SSE 不是有效 UTF-8"))?;
    let mut data = String::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(value) = line.strip_prefix("data:") else {
            return Err(failure(ErrorKind::InvalidResult, "数据面 SSE 包含未知字段"));
        };
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(value.trim_start());
    }
    if data.is_empty() {
        return Ok(ParsedSse::default());
    }
    if data == "[DONE]" {
        return Ok(ParsedSse {
            done: true,
            ..ParsedSse::default()
        });
    }
    let envelope: SseEnvelope = serde_json::from_str(&data).map_err(|error| {
        failure(
            ErrorKind::InvalidResult,
            format!("数据面 SSE JSON 无效：{error}"),
        )
    })?;
    if let Some(error) = envelope.error {
        return Err(failure(
            ErrorKind::ExecutionFailed,
            error.message.unwrap_or_else(|| "数据面返回错误".into()),
        ));
    }
    let mut result = ParsedSse {
        usage: envelope.usage,
        ..ParsedSse::default()
    };
    for choice in envelope.choices {
        let delta = choice.delta.or(choice.message);
        if let Some(delta) = delta {
            if let Some(content) = delta.content.filter(|text| !text.is_empty()) {
                result.events.push(GenerationEvent::Content(content));
            }
            if let Some(thinking) = delta
                .reasoning_content
                .or(delta.reasoning)
                .filter(|text| !text.is_empty())
            {
                result.events.push(GenerationEvent::Thinking(thinking));
            }
        }
        let _finish_reason = choice.finish_reason;
    }
    Ok(result)
}

fn find_frame_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| position + 2)
        .or_else(|| {
            buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
        })
}

async fn read_bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, ChatError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| transport(error.to_string()))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_ERROR_BODY_BYTES {
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn summarize_error(body: &[u8]) -> String {
    std::str::from_utf8(body)
        .ok()
        .and_then(|text| {
            serde_json::from_str::<SseEnvelope>(text)
                .ok()
                .and_then(|envelope| envelope.error.and_then(|error| error.message))
                .or_else(|| (!text.trim().is_empty()).then(|| text.trim().to_owned()))
        })
        .unwrap_or_else(|| "服务端未提供错误详情".into())
}
fn transport(reason: String) -> ChatError {
    failure(ErrorKind::NotConnected, format!("数据面请求失败：{reason}"))
}

fn failure(kind: ErrorKind, reason: impl Into<String>) -> ChatError {
    ChatError {
        kind,
        operation: Operation::Generate,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_content_and_done_frames() {
        let mut decoder = SseDecoder::default();
        let first = decoder
            .feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"he")
            .expect("半帧应保留");
        assert!(first.events.is_empty());
        let second = decoder
            .feed(b"llo\"}}]}\n\ndata: [DONE]\n\n")
            .expect("完整帧应解析");
        assert_eq!(
            second.events,
            vec![GenerationEvent::Content("hello".into())]
        );
        assert!(second.done);
        assert!(decoder.finish().expect("完成后 EOF 应通过").done);
    }

    #[tokio::test]
    async fn sends_router_request_and_emits_only_real_sse_content() {
        use std::{cell::RefCell, rc::Rc};
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("监听");
        let address = listener.local_addr().expect("地址");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("请求");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let count = stream.read(&mut chunk).await.expect("读取请求");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).expect("请求 UTF-8");
            assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer sk-test"));
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"真实\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"答案\"}}]}\n\n",
                "data: [DONE]\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            stream.write_all(response.as_bytes()).await.expect("响应");
        });
        let variant = super::super::model::ModelVariant {
            id: "route.gguf".into(),
            quantization: "Q4_K_M".into(),
            runtime: "llama.cpp".into(),
            artifact_source: "https://example.invalid/route.gguf".into(),
            expected_sha256: "a".repeat(64),
            ram_bytes: 1,
            vram_bytes: 0,
            disk_bytes: 1,
            file_size_bytes: None,
        };
        let model = super::super::model::ChatModel {
            id: "route-model".into(),
            name: "Route model".into(),
            description: "真实数据面测试模型".into(),
            updated_at: None,
            variants: vec![variant.clone()],
            catalog_selection: None,
        };
        let request = GenerationRequest {
            prepared: super::super::model::PreparedModel {
                route_model_id: None,
                model,
                variant,
                actual_sha256: "a".repeat(64),
                local_path: r"C:\models\route-model.gguf".into(),
            },
            messages: vec![HistoryMessage {
                role: MessageRole::User,
                content: "你好".into(),
            }],
            reasoning_effort: super::super::state::ReasoningEffort::High,
        };
        let client = Client::new();
        let base = Url::parse(&format!("http://{address}")).expect("地址");
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&events);
        let result = generate_with_client(
            &client,
            &base,
            request,
            "sk-test",
            Box::new(move |event| sink.borrow_mut().push(event)),
        )
        .await
        .expect("真实 SSE 应成功");
        assert!(result.is_none());
        assert_eq!(
            *events.borrow(),
            vec![
                GenerationEvent::Content("真实".into()),
                GenerationEvent::Content("答案".into())
            ]
        );
        server.await.expect("服务端任务");
    }

    #[test]
    fn rejects_unknown_sse_fields_and_oversized_frames() {
        let mut decoder = SseDecoder::default();
        let error = decoder
            .feed(b"event: message\ndata: {}\n\n")
            .expect_err("未知字段必须拒绝");
        assert_eq!(error.kind, ErrorKind::InvalidResult);
        let mut decoder = SseDecoder::default();
        let error = decoder
            .feed(&vec![b'x'; MAX_SSE_CHUNK_BYTES + 1])
            .expect_err("超大块必须拒绝");
        assert_eq!(error.kind, ErrorKind::InvalidResult);
    }
}

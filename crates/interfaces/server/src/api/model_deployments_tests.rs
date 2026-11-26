#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "测试只使用固定的本地序列化输入"
)]

use super::*;
use serde_json::json;

#[test]
fn submit_response_keeps_reused_at_the_top_level() {
    let value = serde_json::to_value(SubmitEnvelope {
        success: true,
        data: json!({"task_id": "task-1"}),
        reused: true,
    })
    .expect("固定响应应可序列化");

    assert_eq!(value["success"], true);
    assert_eq!(value["reused"], true);
    assert_eq!(value["data"]["task_id"], "task-1");
}

#[test]
fn missing_backend_defaults_to_auto() {
    let request: SubmitRequest = serde_json::from_value(json!({
        "model": "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
        "source": "hf-mirror",
        "revision": "main",
        "variant_id": "qwen2.5-0.5b-instruct-q4_k_m.gguf",
        "checksum": "sha256:74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db"
    }))
    .expect("backend 缺省时应使用 auto");

    let command = validate_request(request).expect("固定凭据应通过校验");
    assert_eq!(command.backend, RequestedBackend::Auto);
}

//! Integration test: mock OpenAI-compatible upstreams + full voicebot behavior.
//! Single sequential test: global config LazyLock + env-var setup means
//! parallel #[tokio::test]s would race; run everything in defined order here.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use hyper_util::rt::TokioExecutor;
use serde_json::{json, Value};

use voicebot_rs::{build_app, test_router, App};

static SEEN_LLM: std::sync::Mutex<Vec<Value>> = std::sync::Mutex::new(Vec::new());

struct Chunks(tokio::sync::mpsc::Receiver<Result<bytes::Bytes, std::io::Error>>);
impl futures_core::Stream for Chunks {
    type Item = Result<bytes::Bytes, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.0).poll_recv(cx)
    }
}

fn json_ok(v: Value) -> Response {
    let raw = serde_json::to_vec(&v).unwrap_or_default();
    Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Body::from(raw))
        .unwrap()
}

async fn mock_upstream() -> u16 {
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(|req: Request<Body>| async move {
                let raw = req.into_body().collect().await.unwrap().to_bytes();
                let v: Value = serde_json::from_slice(&raw).unwrap_or(json!({}));
                SEEN_LLM.lock().unwrap().push(v.clone());
                if v.get("stream") == Some(&json!(true)) {
                    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(8);
                    tokio::spawn(async move {
                        for piece in ["你好", "，", "这里是", "测试。"] {
                            let ev = json!({"choices":[{"delta":{"content":piece}}]});
                            let _ = tx.send(Ok(bytes::Bytes::from(format!("data: {ev}\n\n")))).await;
                            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        }
                        let _ = tx.send(Ok(bytes::Bytes::from("data: [DONE]\n\n"))).await;
                    });
                    Response::builder()
                        .status(200)
                        .header("content-type", "text/event-stream")
                        .body(Body::from_stream(Chunks(rx)))
                        .unwrap()
                } else {
                    json_ok(json!({"choices":[{"message":{"content":"你好，这里是测试。"}}]}))
                }
            }),
        )
        .route(
            "/v1/audio/transcriptions",
            post(|req: Request<Body>| async move {
                let _ = req.into_body().collect().await;
                json_ok(json!({"text": "语音输入的内容"}))
            }),
        )
        .route(
            "/v1/audio/speech",
            post(|req: Request<Body>| async move {
                let _ = req.into_body().collect().await;
                Response::builder()
                    .status(200)
                    .header("content-type", "audio/mpeg")
                    .body(Body::from(b"FAKE_MP3_BYTES".to_vec()))
                    .unwrap()
            }),
        )
        .route("/healthz", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

fn multipart_body(fields: &[(&str, &str, Option<Vec<u8>>)]) -> (String, Vec<u8>) {
    let boundary = "----vbtestboundary42";
    let mut out = Vec::new();
    for (name, value, data) in fields {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        if let Some(data) = data {
            out.extend_from_slice(
                format!("content-disposition: form-data; name=\"{name}\"; filename=\"blob\"\r\ncontent-type: application/octet-stream\r\n\r\n").as_bytes(),
            );
            out.extend_from_slice(data);
            out.extend_from_slice(b"\r\n");
        } else {
            out.extend_from_slice(format!("content-disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
        }
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), out)
}

type HClient = hyper_util::client::legacy::Client<hyper_util::client::legacy::connect::HttpConnector, http_body_util::Full<bytes::Bytes>>;

async fn req(client: &HClient, port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: Vec<u8>) -> (u16, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(format!("http://127.0.0.1:{port}{path}"));
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let resp = client.request(builder.body(http_body_util::Full::new(bytes::Bytes::from(body))).unwrap()).await.unwrap();
    let status = resp.status().as_u16();
    let hdrs = resp.headers().clone();
    let body = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, hdrs, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn full_parity_suite() {
    let llm_port = mock_upstream().await;

    // 隔离的 base_dir（测试专用），env 在 LazyLock 首次访问前设置
    let base = std::env::temp_dir().join(format!("vb_test_{llm_port}"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("static")).unwrap();
    std::fs::write(base.join("static/index.html"), b"<html>vb-index</html>").unwrap();
    std::env::set_var("VB_BASE_DIR", &base);
    std::env::set_var("QWEN_BASE_URL", format!("http://127.0.0.1:{llm_port}/v1"));
    std::env::set_var("QWEN_API_KEY", "test-qwen-key");
    std::env::set_var("QWEN_MODEL", "qwen");
    std::env::set_var("TTS_BASE", format!("http://127.0.0.1:{llm_port}/v1"));
    std::env::set_var("TTS_API_KEY", "test-tts-key");
    std::env::set_var("TTS_MODEL", "test-tts");
    std::env::set_var("TTS_CACHE_ENABLED", "0");
    std::env::set_var("ASR_BASE", format!("http://127.0.0.1:{llm_port}/v1"));
    std::env::set_var("ASR_API_KEY", "test-asr-key");
    std::env::set_var("ASR_MODEL", "test-asr");
    std::env::set_var("BOT_TOKEN", "sekrit");

    let app: Arc<App> = build_app().await;
    let router = test_router(app.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client: HClient = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build_http();
    let auth = [("x-bot-token", "sekrit")];

    // 1. health：无 token 401；有 token 200 带模型信息
    let (st, _, _) = req(&client, port, "GET", "/api/health", &[], vec![]).await;
    assert_eq!(st, 401);
    let (st, _, body) = req(&client, port, "GET", "/api/health", &auth, vec![]).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["asr_model"], json!("test-asr"));

    // 2. 静态 index
    let (st, _, body) = req(&client, port, "GET", "/", &[], vec![]).await;
    assert_eq!(st, 200);
    assert_eq!(body, b"<html>vb-index</html>");
    let (st, _, _) = req(&client, port, "GET", "/static/../Cargo.toml", &[], vec![]).await;
    assert_eq!(st, 404, "path traversal 必须被拒绝");
    // 3. 空输入 400（带鉴权；401 先行已在 health 断言）
    let (ct, mp) = multipart_body(&[]);
    let (st, _, _) = req(&client, port, "POST", "/api/chat", &[("x-bot-token", "sekrit"), ("content-type", &ct)], mp).await;
    assert_eq!(st, 400);

    // 4. 非流式 chat（文本输入）→ reply + b64 音频
    let (ct, mp) = multipart_body(&[("text", "问个问题", None), ("session_id", "s1", None)]);
    let (st, _, body) = req(&client, port, "POST", "/api/chat", &[("x-bot-token", "sekrit"), ("content-type", &ct)], mp).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["reply_text"], json!("你好，这里是测试。"));
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(v["audio"].as_str().unwrap())
        .unwrap();
    assert_eq!(decoded, b"FAKE_MP3_BYTES");

    // 历史落盘（user + assistant）
    let (st, _, body) = req(&client, port, "GET", "/api/history?session_id=s1", &auth, vec![]).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    let msgs = v["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["content"], json!("问个问题"));
    assert_eq!(msgs[1]["role"], json!("assistant"));
    // sessions.json 持久化文件存在
    assert!(base.join("sessions.json").exists());

    // 5. 流式 chat：SSE 事件顺序 sentence* → done → audio → audio_done
    let (ct, mp) = multipart_body(&[("text", "再问一个", None), ("session_id", "s2", None)]);
    let (st, hdr, body) = req(&client, port, "POST", "/api/chat/stream", &[("x-bot-token", "sekrit"), ("content-type", &ct)], mp).await;
    assert_eq!(st, 200);
    assert!(hdr.get("content-type").unwrap().to_str().unwrap().contains("text/event-stream"));
    let text = String::from_utf8(body).unwrap();
    let events: Vec<Value> = text
        .split("\n\n")
        .filter(|f| f.starts_with("data: "))
        .map(|f| serde_json::from_str(&f["data: ".len()..]).unwrap())
        .collect();
    let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types.first(), Some(&"sentence"));
    assert_eq!(types.last(), Some(&"audio_done"));
    let done_idx = types.iter().position(|t| *t == "done").unwrap();
    let audio_idx = types.iter().position(|t| *t == "audio").unwrap();
    assert!(done_idx < audio_idx, "audio 必须在 done 之后");
    let done = &events[done_idx];
    assert_eq!(done["phase"], json!("text_done"));
    assert_eq!(done["reply"], json!("你好，这里是测试。"));
    let audio_done = events.last().unwrap();
    assert_eq!(audio_done["audio_total"], json!(1));
    assert_eq!(audio_done["audio_failed"], json!(0));

    // 6. tts job 流式端点（能力令牌）
    let url = app.jobs.create("你好世界。", "voice-x");
    let (st, hdr, body) = req(&client, port, "GET", &url, &[], vec![]).await;
    assert_eq!(st, 200);
    assert_eq!(hdr.get("content-type").unwrap(), "audio/wav");
    assert_eq!(body, b"FAKE_MP3_BYTES");
    let (st, _, _) = req(&client, port, "GET", "/api/tts/stream/deadbeef", &[], vec![]).await;
    assert_eq!(st, 404);

    // 7. personas：默认列表 + 切换激活 + 整体替换
    let (st, _, body) = req(&client, port, "GET", "/api/personas", &auth, vec![]).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert!(v["personas"].as_array().unwrap().len() >= 5);
    let first_id = v["personas"][0]["id"].as_str().unwrap().to_string();
    let body = json!({"id": first_id}).to_string();
    let (st, _, _) = req(&client, port, "POST", "/api/personas/active", &auth, body.into_bytes()).await;
    assert_eq!(st, 200);
    let new_personas = json!([
        {"id": "", "name": "新角色", "temperature": "0.9", "prompt": "测试提示词", "voice": "v1"}
    ]);
    let body = json!({"personas": new_personas}).to_string();
    let (st, _, body) = req(&client, port, "POST", "/api/personas", &auth, body.into_bytes()).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    let saved_id = v["personas"][0]["id"].as_str().unwrap().to_string();
    assert!(saved_id.starts_with('p'), "自动补 id 前缀 p");

    // 8. model status / stack 校验 / provider
    let (st, _, body) = req(&client, port, "GET", "/api/model/status", &auth, vec![]).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["provider"], json!("local"));
    for (cat, opts) in v["catalog"].as_object().unwrap() {
        for o in opts.as_array().unwrap() {
            assert!(o.get("api_key").is_none(), "{cat} 公共目录不得泄露 api_key");
            assert!(o.get("base_url").is_none(), "{cat} 公共目录不得泄露 base_url");
        }
    }
    let (st, _, _) = req(&client, port, "POST", "/api/model/stack", &auth, br#"{"text":"no-such-model"}"#.to_vec()).await;
    assert_eq!(st, 400);
    let (st, _, body) = req(&client, port, "POST", "/api/model/stack", &auth, br#"{"tts":"step-stepaudio-2.5-tts"}"#.to_vec()).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["stack"]["tts"], json!("step-stepaudio-2.5-tts"));
    let (st, _, body) = req(&client, port, "POST", "/api/model/provider", &auth, br#"{"provider":"cloud"}"#.to_vec()).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["provider"], json!("cloud"));
    // 还原 local，便于后续断言
    let _ = req(&client, port, "POST", "/api/model/provider", &auth, br#"{"provider":"local"}"#.to_vec()).await;

    // 9. history clear / truncate
    let _ = req(&client, port, "POST", "/api/history/clear?session_id=s1", &auth, vec![]).await;
    let (st, _, body) = req(&client, port, "GET", "/api/history?session_id=s1", &auth, vec![]).await;
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["messages"].as_array().unwrap().len(), 0);
    let (st, _, body) = req(&client, port, "POST", "/api/history/truncate", &auth, br#"{"session_id":"s2","keep_count":1}"#.to_vec()).await;
    assert_eq!(st, 200);
    // 10. 用新角色再跑一次非流式 chat，验证 system prompt 热生效
    let (ct, mp) = multipart_body(&[("text", "新角色", None), ("session_id", "s3", None)]);
    let (st, _, _) = req(&client, port, "POST", "/api/chat", &[("x-bot-token", "sekrit"), ("content-type", &ct)], mp).await;
    assert_eq!(st, 200);
    // 10. LLM 请求结构：首条 system 来自 persona prompt；模型来自 stack
    let seen = SEEN_LLM.lock().unwrap();
    let last = seen.last().unwrap();
    assert_eq!(last["messages"][0]["role"], json!("system"));
    assert!(last["messages"][0]["content"].as_str().unwrap().contains("测试提示词"));
    assert_eq!(last["model"], json!("qwen"));
    drop(seen);

    let _ = std::fs::remove_dir_all(&base);
}

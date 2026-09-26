//! voicebot-rs — rsaga-trpg-voicebot 的 Rust(axum) 改写。
//! 端点与行为对齐 aws 部署版 app.py/config.py/step_clients.py（2026-09 快照）。

pub mod clients;
pub mod config;
pub mod state;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde_json::{json, Value};

use clients::{clean_text, split_sentences};
use config::{cfg, get_active_persona, log_line, persona_hash};
use axum::extract::FromRequest;
use state::{Sessions, TtsJobs};

pub struct App {
    pub http: reqwest::Client,
    pub sessions: Sessions,
    pub jobs: TtsJobs,
    pub tts_stream_lock: tokio::sync::Mutex<()>,
}

type SharedApp = Arc<App>;

fn sse_body(obj: &Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(obj).unwrap_or_default())
}

fn http_err(status: StatusCode, detail: &str) -> Response {
    (status, Json(json!({"detail": detail}))).into_response()
}

fn token_ok(headers: &HeaderMap) -> Result<(), Response> {
    let token = &cfg().bot_token;
    if token.is_empty() {
        return Ok(());
    }
    let got = headers.get("x-bot-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if got == token.as_str() {
        Ok(())
    } else {
        Err(http_err(StatusCode::UNAUTHORIZED, "token 无效"))
    }
}

fn json_response(status: StatusCode, v: Value) -> Response {
    let raw = serde_json::to_vec(&v).unwrap_or_default();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, raw.len())
        .body(Body::from(raw))
        .unwrap()
}

fn tts_audio_mime() -> &'static str {
    let fmt = config::get_tts_config()
        .get("response_format")
        .and_then(Value::as_str)
        .unwrap_or("mp3")
        .trim()
        .to_lowercase();
    if fmt.is_empty() || fmt == "mp3" || fmt == "mpeg" || fmt == "mpga" {
        "audio/mpeg"
    } else if fmt == "wav" || fmt == "wave" {
        "audio/wav"
    } else {
        "audio/mpeg"
    }
}

fn persona_voice() -> String {
    get_active_persona().get("voice").and_then(Value::as_str).unwrap_or("").to_string()
}

fn safe_bg_scene(_scene: &str, persona: &str) -> String {
    let who = ["成年人", "成年", "成人"]
        .iter()
        .fold(if persona.is_empty() { "角色".to_string() } else { persona.to_string() }, |s, x| s.replace(x, "角色"));
    format!(
        "{who}置身雨夜室内场景，窗外雨幕与昏黄灯光交织，女性角色拥有精致秀美且结构对称的五官，眉眼清晰有神、鼻唇比例协调，神态、姿态、手部动作和情绪细节清晰，男性角色仅以黑色剪影、背影或暗影轮廓出现，不描绘男性五官和皮肤细节，用构图、道具、表情、距离和空间关系表现剧情张力，水墨写意与工笔重彩融合的国风插画风，宣纸肌理，东方美学，面部完整自然，无五官错位、重复眼睛或畸形脸，电影感光影，无文字、无logo、无水印。"
    )
}

fn chunk_list_from_reply(text: &str) -> Vec<String> {
    let tts_cfg = config::get_tts_config();
    let clean = clean_text(text);
    if clean.is_empty() {
        return vec![];
    }
    let c = cfg();
    if clients::prefers_whole_reply_tts(&tts_cfg) {
        let limit = c.grok_tts_chunk_chars.max(120);
        if clean.chars().count() <= limit {
            return vec![clean];
        }
        let mut chunks: Vec<String> = Vec::new();
        let mut cur = String::new();
        for piece in split_sentences_keep(&clean) {
            let plen = piece.chars().count();
            if plen > limit {
                if !cur.is_empty() {
                    chunks.push(std::mem::take(&mut cur));
                }
                let chars: Vec<char> = piece.chars().collect();
                for i in (0..chars.len()).step_by(limit) {
                    chunks.push(chars[i..(i + limit).min(chars.len())].iter().collect());
                }
            } else if !cur.is_empty() && cur.chars().count() + plen > limit {
                chunks.push(std::mem::take(&mut cur));
                cur = piece;
            } else {
                cur.push_str(&piece);
            }
        }
        if !cur.is_empty() {
            chunks.push(cur);
        }
        return chunks;
    }
    let target = c.tts_merge_target_chars.max(1);
    let limit = c.tts_merge_max_chars.max(target);
    let mut pieces: Vec<String> = split_sentences_keep(&clean)
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if pieces.is_empty() {
        pieces = vec![clean];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for piece in pieces {
        let plen = piece.chars().count();
        if plen > limit {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            let chars: Vec<char> = piece.chars().collect();
            for i in (0..chars.len()).step_by(limit) {
                chunks.push(chars[i..(i + limit).min(chars.len())].iter().collect());
            }
            continue;
        }
        if !current.is_empty() && current.chars().count() + plen > limit {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(&piece);
        if current.chars().count() >= target {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// 与 python re.split(r"(?<=[。！？!?…])") 等价：保留标点，产出全部片段。
fn split_sentences_keep(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if matches!(ch, '。' | '！' | '？' | '!' | '?' | '…') {
            parts.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

// ── 路由 ────────────────────────────────────────────────────────────────────

async fn health(State(_app): State<SharedApp>, headers: HeaderMap) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let asr = config::get_asr_config();
    let tts = config::get_tts_config();
    json_response(StatusCode::OK, json!({
        "ok": true,
        "asr_model": asr.get("model").cloned().unwrap_or(Value::Null),
        "asr_base": asr.get("base_url").cloned().unwrap_or(Value::Null),
        "tts_model": tts.get("model").cloned().unwrap_or(Value::Null),
        "tts_base": tts.get("base_url").cloned().unwrap_or(Value::Null),
    }))
}

async fn history(
    State(app): State<SharedApp>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let sid = params.get("session_id").cloned().unwrap_or_else(|| "default".into());
    json_response(StatusCode::OK, json!({"messages": app.sessions.history(&sid)}))
}

async fn history_clear(
    State(app): State<SharedApp>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let sid = params.get("session_id").cloned().unwrap_or_else(|| "default".into());
    app.sessions.remove(&sid);
    app.sessions.save();
    log_line(&format!("已清空会话 {sid}"));
    json_response(StatusCode::OK, json!({"ok": true}))
}


/// python `await request.json()` 对齐：不校验 content-type，直接解析 body。
async fn parse_json_body(body: axum::extract::Request) -> Result<Value, Response> {
    let bytes = axum::body::to_bytes(body.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|_| http_err(StatusCode::BAD_REQUEST, "invalid body"))?;
    serde_json::from_slice(&bytes).map_err(|_| http_err(StatusCode::BAD_REQUEST, "invalid json"))
}

async fn history_truncate(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    body: axum::extract::Request,
) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let v: Value = match parse_json_body(body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let sid = v.get("session_id").and_then(Value::as_str).unwrap_or("default").to_string();
    let keep = v.get("keep_count").and_then(Value::as_i64).unwrap_or(0);
    if keep >= 0 {
        app.sessions.history_mut(&sid, |h| {
            let keep = keep as usize;
            if h.len() > keep {
                h.truncate(keep);
            }
        });
        app.sessions.save();
    }
    let kept = app.sessions.history(&sid).len();
    json_response(StatusCode::OK, json!({"ok": true, "kept": kept}))
}

async fn tts_stream(
    State(app): State<SharedApp>,
    Path(job_id): Path<String>,
) -> Response {
    let Some((text, voice)) = app.jobs.get(&job_id) else {
        return http_err(StatusCode::NOT_FOUND, "音频任务不存在或已过期");
    };
    let audio = {
        let _guard = app.tts_stream_lock.lock().await;
        clients::tts(&app.http, &text, &voice).await
    };
    match audio {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, tts_audio_mime())
            .header(header::CACHE_CONTROL, "private, max-age=1800")
            .header(header::CONTENT_LENGTH, bytes.len())
            .header("accept-ranges", "none")
            .body(Body::from(bytes))
            .unwrap(),
        Err(e) => json_response(
            StatusCode::BAD_GATEWAY,
            json!({"detail": e.to_string()}),
        ),
    }
}

async fn bg_gen(State(app): State<SharedApp>, headers: HeaderMap, body: axum::extract::Request) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let v: Value = match parse_json_body(body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let text = v.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let persona = v.get("persona").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return http_err(StatusCode::BAD_REQUEST, "无 text");
    }
    let scene = match clients::summarize_for_image(&app.http, &text, &persona).await {
        Ok(s) => s,
        Err(e) => {
            log_line(&format!("背景图总结失败，使用安全 fallback: {e}"));
            safe_bg_scene(&text.chars().take(60).collect::<String>(), &persona)
        }
    };
    match clients::gen_bg_image(&app.http, &scene).await {
        Ok(b64) => json_response(StatusCode::OK, json!({"image": b64, "scene": scene})),
        Err(e) => {
            if e.is_image_moderation() {
                let safe = safe_bg_scene(&scene, &persona);
                log_line("绘图被审核拒绝，改用安全场景重试");
                return match clients::gen_bg_image(&app.http, &safe).await {
                    Ok(b64) => json_response(StatusCode::OK, json!({"image": b64, "scene": safe, "fallback": true})),
                    Err(retry) => json_response(StatusCode::BAD_GATEWAY, json!({"error": "bg", "detail": retry.detail()})),
                };
            }
            json_response(StatusCode::BAD_GATEWAY, json!({"error": "bg", "detail": e.detail()}))
        }
    }
}

async fn personas_list(State(_app): State<SharedApp>, headers: HeaderMap) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let personas = config::get_personas();
    let active = get_active_persona();
    json_response(StatusCode::OK, json!({
        "personas": personas,
        "active_id": active.get("id").cloned().unwrap_or(Value::Null),
        "voice_options": cfg().voice_options.iter()
            .map(|(n, v)| json!([n, v])).collect::<Vec<_>>(),
    }))
}

async fn personas_save(State(_app): State<SharedApp>, headers: HeaderMap, body: axum::extract::Request) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let v: Value = match parse_json_body(body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let mut personas: Vec<Value> = v.get("personas").and_then(Value::as_array).cloned().unwrap_or_default();
    for p in personas.iter_mut() {
        let obj = p.as_object_mut().unwrap();
        let has_id = obj.get("id").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false);
        if !has_id {
            let name = obj.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            obj.insert("id".into(), json!(format!("p{}", persona_hash(&name))));
        }
        let temp = obj.get("temperature").and_then(Value::as_f64).unwrap_or(1.0);
        obj.insert("temperature".into(), json!(temp));
        let prompt = obj.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_string();
        obj.insert("prompt".into(), json!(prompt));
        let name = obj.get("name").and_then(Value::as_str).unwrap_or("");
        let name = if name.trim().is_empty() { "未命名".to_string() } else { name.trim().to_string() };
        obj.insert("name".into(), json!(name));
        let voice = obj.get("voice").and_then(Value::as_str).unwrap_or("").trim().to_string();
        obj.insert("voice".into(), json!(voice));
    }
    config::set_personas(&personas);
    log_line(&format!("角色库已更新，共 {} 个", personas.len()));
    json_response(StatusCode::OK, json!({"ok": true, "personas": personas}))
}

async fn personas_active(State(_app): State<SharedApp>, headers: HeaderMap, body: axum::extract::Request) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let v: Value = match parse_json_body(body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let pid = v.get("id").and_then(Value::as_str).unwrap_or("");
    match config::set_active_persona(pid) {
        Some(p) => {
            log_line(&format!("切换激活角色 → {} ({})", p.get("name").and_then(Value::as_str).unwrap_or(""), pid));
            json_response(StatusCode::OK, json!({"ok": true, "active": p}))
        }
        None => http_err(StatusCode::NOT_FOUND, "角色不存在"),
    }
}

async fn model_status(State(_app): State<SharedApp>, headers: HeaderMap) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let provider = config::get_model_provider();
    let llm = config::get_llm_config();
    let img = config::get_img_config();
    let asr = config::get_asr_config();
    let tts = config::get_tts_config();
    json_response(StatusCode::OK, json!({
        "provider": provider,
        "stack": config::get_model_stack(),
        "asr_enabled": cfg().asr_enabled,
        "catalog": config::public_catalog(),
        "llm_model": llm.get("model").cloned().unwrap_or(Value::Null),
        "vision_model": llm.get("vision_model").cloned().unwrap_or(Value::Null),
        "img_model": img.get("model").cloned().unwrap_or(Value::Null),
        "options": [
            {"id": "local", "label": "本地 Qwen", "desc": "Qwen + Step 绘图 + 当前语音链路"},
            {"id": "cloud", "label": "云端 DeepSeek", "desc": "DeepSeek v4 Flash + OpenRouter 生图 + Grok 看图兜底 + 当前语音链路"},
        ],
        "asr_model": asr.get("model").cloned().unwrap_or(Value::Null),
        "asr_base": asr.get("base_url").cloned().unwrap_or(Value::Null),
        "tts_model": tts.get("model").cloned().unwrap_or(Value::Null),
        "tts_base": tts.get("base_url").cloned().unwrap_or(Value::Null),
    }))
}

async fn model_stack(State(_app): State<SharedApp>, headers: HeaderMap, body: axum::extract::Request) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let v: Value = match parse_json_body(body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match config::set_model_stack(&v) {
        Ok(stack) => {
            log_line(&format!("模型组合已更新 → {stack}"));
            json_response(StatusCode::OK, json!({"ok": true, "stack": stack}))
        }
        Err(e) => http_err(StatusCode::BAD_REQUEST, &e),
    }
}

async fn model_provider(State(_app): State<SharedApp>, headers: HeaderMap, body: axum::extract::Request) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let v: Value = match parse_json_body(body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let provider = v.get("provider").and_then(Value::as_str).unwrap_or("");
    if provider != "local" && provider != "cloud" {
        return http_err(StatusCode::BAD_REQUEST, "provider 只支持 local 或 cloud");
    }
    if let Err(e) = config::set_model_provider(provider) {
        return http_err(StatusCode::BAD_REQUEST, &e);
    }
    log_line(&format!("模型提供商已切换 → {provider}"));
    let llm = config::get_llm_config();
    json_response(StatusCode::OK, json!({
        "ok": true, "provider": provider,
        "llm_model": llm.get("model").cloned().unwrap_or(Value::Null),
    }))
}

struct ChatForm {
    text: String,
    audio: Option<Vec<u8>>,
    image: Option<String>,
    session_id: String,
}

async fn parse_chat_form(mut mp: Multipart) -> Result<ChatForm, Response> {
    let mut form = ChatForm { text: String::new(), audio: None, image: None, session_id: "default".into() };
    while let Some(field) = mp.next_field().await.map_err(|_| http_err(StatusCode::BAD_REQUEST, "invalid multipart"))? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "text" => form.text = field.text().await.unwrap_or_default(),
            "session_id" => {
                let v = field.text().await.unwrap_or_default();
                if !v.is_empty() { form.session_id = v; }
            }
            "image" => form.image = Some(field.text().await.unwrap_or_default()),
            "audio" => {
                let bytes = field.bytes().await.map_err(|_| http_err(StatusCode::BAD_REQUEST, "audio read failed"))?;
                if !bytes.is_empty() {
                    form.audio = Some(bytes.to_vec());
                }
            }
            _ => {}
        }
    }
    Ok(form)
}

async fn chat(State(app): State<SharedApp>, headers: HeaderMap, mp: Multipart) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let form = match parse_chat_form(mp).await {
        Ok(f) => f,
        Err(r) => return r,
    };
    let mut user_text = form.text.trim().to_string();
    if let Some(audio) = &form.audio {
        if user_text.is_empty() {
            if !cfg().asr_enabled {
                return json_response(StatusCode::SERVICE_UNAVAILABLE, json!({
                    "error": "asr_disabled", "detail": "语音识别未配置，请先配置 ASR 模型"}));
            }
            log_line(&format!("ASR 输入 audio bytes={}", audio.len()));
            match clients::asr(&app.http, audio, "audio/webm").await {
                Ok(t) => {
                    log_line(&format!("ASR → {t:?}"));
                    user_text = t;
                }
                Err(e) => {
                    log_line(&format!("ASR 失败: {e}"));
                    return json_response(StatusCode::BAD_GATEWAY, json!({"error": "asr", "detail": e.detail()}));
                }
            }
        }
    }
    if user_text.is_empty() {
        return http_err(StatusCode::BAD_REQUEST, "无输入（text/audio 至少一个）");
    }

    let sid = form.session_id.clone();
    app.sessions.history_mut(&sid, |h| h.push(json!({"role": "user", "content": user_text})));
    let hist = app.sessions.history(&sid);
    let reply = match clients::llm(&app.http, &hist).await {
        Ok(r) => r,
        Err(e) => {
            log_line(&format!("LLM 失败: {e}"));
            app.sessions.history_mut(&sid, |h| { h.pop(); });
            app.sessions.save();
            return json_response(StatusCode::BAD_GATEWAY, json!({"error": "llm", "detail": e.detail()}));
        }
    };
    let reply_clean = clean_text(&reply);
    app.sessions.history_mut(&sid, |h| h.push(json!({"role": "assistant", "content": reply_clean})));
    app.sessions.trim(&sid);
    app.sessions.save();

    let voice = persona_voice();
    match clients::tts(&app.http, &reply_clean, &voice).await {
        Ok(mp3) => {
            let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&mp3);
            json_response(StatusCode::OK, json!({"user_text": user_text, "reply_text": reply_clean, "audio": audio_b64}))
        }
        Err(e) => {
            log_line(&format!("TTS 失败: {e}"));
            json_response(StatusCode::OK, json!({
                "user_text": user_text, "reply_text": reply_clean,
                "error": "tts", "detail": e.detail()}))
        }
    }
}

async fn chat_stream(State(app): State<SharedApp>, headers: HeaderMap, mp: Multipart) -> Response {
    if let Err(r) = token_ok(&headers) { return r; }
    let form = match parse_chat_form(mp).await {
        Ok(f) => f,
        Err(r) => return r,
    };
    let mut user_text = form.text.trim().to_string();
    let had_audio = form.audio.is_some();
    if let Some(audio) = &form.audio {
        if user_text.is_empty() {
            if !cfg().asr_enabled {
                return json_response(StatusCode::SERVICE_UNAVAILABLE, json!({
                    "error": "asr_disabled", "detail": "语音识别未配置，请先配置 ASR 模型"}));
            }
            log_line(&format!("ASR 输入 audio bytes={}", audio.len()));
            match clients::asr(&app.http, audio, "audio/webm").await {
                Ok(t) => {
                    log_line(&format!("ASR → {t:?}"));
                    user_text = t;
                }
                Err(e) => {
                    log_line(&format!("ASR 失败: {e}"));
                    return json_response(StatusCode::BAD_GATEWAY, json!({"error": "asr", "detail": e.detail()}));
                }
            }
        }
    }
    if user_text.is_empty() && form.image.is_none() {
        return http_err(StatusCode::BAD_REQUEST, "无输入（text/audio/image 至少一个）");
    }
    let image = form.image.clone();
    if image.is_some() && user_text.is_empty() {
        user_text = "请简短描述这张图片里的内容，两三句话。".to_string();
    }

    let sid = form.session_id.clone();
    app.sessions.history_mut(&sid, |h| h.push(json!({"role": "user", "content": user_text})));

    // SSE producer（对应 python event_gen）
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(64);
    let app2 = app.clone();
    let sid2 = sid.clone();
    let user_text2 = user_text.clone();
    let had_audio2 = had_audio;
    let image2 = image.clone();
    tokio::spawn(async move {
        let mut tx = tx;
        chat_stream_producer(app2, sid2, user_text2, had_audio2, image2, &mut tx).await;
    });

    let stream = tokio_stream_bridge(rx);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .unwrap()
}


fn tokio_stream_bridge(
    rx: tokio::sync::mpsc::Receiver<Result<bytes::Bytes, std::io::Error>>,
) -> impl futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> {
    struct S(tokio::sync::mpsc::Receiver<Result<bytes::Bytes, std::io::Error>>);
    impl futures_core::Stream for S {
        type Item = Result<bytes::Bytes, std::io::Error>;
        fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
            std::pin::Pin::new(&mut self.0).poll_recv(cx)
        }
    }
    S(rx)
}

async fn chat_stream_producer(
    app: SharedApp,
    sid: String,
    user_text: String,
    had_audio: bool,
    image: Option<String>,
    tx: &mut tokio::sync::mpsc::Sender<Result<bytes::Bytes, std::io::Error>>,
) {
    async fn push(tx: &mut tokio::sync::mpsc::Sender<Result<bytes::Bytes, std::io::Error>>, obj: Value) {
        let _ = tx.send(Ok(bytes::Bytes::from(sse_body(&obj)))).await;
    }

    if had_audio {
        push(tx, json!({"type": "user", "text": user_text})).await;
    }

    let tts_cfg = config::get_tts_config();
    let whole_reply_tts = clients::prefers_whole_reply_tts(&tts_cfg);
    let voice = persona_voice();

    // 1. LLM 流式 → sentence 事件
    let hist = app.sessions.history(&sid);
    let mut full_reply = String::new();
    let stream_result = clients::llm_stream(&app.http, &hist, image.as_deref()).await;
    let pieces = match stream_result {
        Ok(p) => p,
        Err(e) => {
            log_line(&format!("LLM 流式失败: {e}"));
            app.sessions.history_mut(&sid, |h| { h.pop(); });
            app.sessions.save();
            push(tx, json!({"type": "error", "stage": "llm", "detail": e.detail()})).await;
            return;
        }
    };
    let mut seq = 0u64;
    let mut buf = String::new();
    for piece in pieces {
        full_reply.push_str(&piece);
        let piece_clean = clean_text(&piece);
        if !piece_clean.is_empty() {
            seq += 1;
            push(tx, json!({"type": "sentence", "seq": seq, "text": piece_clean, "audio": ""})).await;
        }
        let (_complete, rest) = split_sentences(&buf);
        buf = rest;
        buf.push_str(&piece);
        let _ = &mut buf;
    }

    // 2. 收尾历史
    let reply_clean = clean_text(&full_reply);
    app.sessions.history_mut(&sid, |h| h.push(json!({"role": "assistant", "content": reply_clean})));
    app.sessions.trim(&sid);
    app.sessions.save();

    // 3. TTS 并发生成、按序投递
    let chunks = chunk_list_from_reply(&reply_clean);
    let total = chunks.len();
    push(tx, json!({"type": "done", "phase": "text_done", "reply": reply_clean, "audio_total": total})).await;

    struct AudioOutcome { seq: u64, event: Value }
    let mut join = tokio::task::JoinSet::new();
    for (idx, chunk) in chunks.into_iter().enumerate() {
        let app = app.clone();
        let voice = voice.clone();
        let whole = whole_reply_tts;
        join.spawn(async move {
            let result = if whole {
                clients::tts(&app.http, &chunk, &voice).await
            } else {
                let _guard = app.tts_stream_lock.lock().await;
                clients::tts(&app.http, &chunk, &voice).await
            };
            let event = match result {
                Ok(mp3) => json!({
                    "type": "audio", "seq": idx as u64 + 1,
                    "audio": base64::engine::general_purpose::STANDARD.encode(&mp3),
                    "audio_mime": tts_audio_mime(),
                }),
                Err(e) => {
                    log_line(&format!("TTS 失败 seq={}: {e}", idx + 1));
                    json!({"type": "audio", "seq": idx as u64 + 1, "audio": "", "tts_error": e.detail()})
                }
            };
            AudioOutcome { seq: idx as u64 + 1, event }
        });
    }

    let mut ready: std::collections::HashMap<u64, Value> = std::collections::HashMap::new();
    let mut next_seq: u64 = 1;
    let mut audio_failed = 0u64;
    while !join.is_empty() {
        match tokio::time::timeout(std::time::Duration::from_secs(10), join.join_next()).await {
            Err(_) => {
                push(tx, json!({"type": "heartbeat", "phase": "audio"})).await;
                continue;
            }
            Ok(None) => break,
            Ok(Some(Ok(outcome))) => {
                ready.insert(outcome.seq, outcome.event);
            }
            Ok(Some(Err(_))) => continue,
        }
        while let Some(event) = ready.remove(&next_seq) {
            if event.get("tts_error").is_some() {
                audio_failed += 1;
            }
            push(tx, event).await;
            next_seq += 1;
        }
    }
    push(tx, json!({"type": "audio_done", "audio_total": total, "audio_ok": total as u64 - audio_failed, "audio_failed": audio_failed})).await;
}

// ── static ─────────────────────────────────────────────────────────────────

fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

async fn index() -> Response {
    serve_static_file("index.html").await
}

async fn static_files(Path(rest): Path<String>) -> Response {
    serve_static_file(&rest).await
}

async fn serve_static_file(rel: &str) -> Response {
    if rel.contains("..") || rel.starts_with('/') {
        return http_err(StatusCode::NOT_FOUND, "Not Found");
    }
    let path = cfg().base_dir.join("static").join(rel);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut builder = Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, mime_for(rel));
            if let Ok(v) = HeaderValue::from_str(&bytes.len().to_string()) {
                builder = builder.header(header::CONTENT_LENGTH, v);
            }
            builder.body(Body::from(bytes)).unwrap()
        }
        Err(_) => http_err(StatusCode::NOT_FOUND, "Not Found"),
    }
}

// ── main ───────────────────────────────────────────────────────────────────

pub async fn build_app() -> SharedApp {
    Arc::new(App {
        http: reqwest::Client::new(),
        sessions: Sessions::load(),
        jobs: TtsJobs::new(),
        tts_stream_lock: tokio::sync::Mutex::new(()),
    })
}

pub fn test_router(app: SharedApp) -> Router {
    make_router(app)
}

pub fn make_router(app: SharedApp) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/history", get(history))
        .route("/api/history/clear", post(history_clear))
        .route("/api/history/truncate", post(history_truncate))
        .route("/api/tts/stream/{job_id}", get(tts_stream))
        .route("/api/bg/gen", post(bg_gen))
        .route("/api/personas", get(personas_list).post(personas_save))
        .route("/api/personas/active", post(personas_active))
        .route("/api/chat", post(chat))
        .route("/api/chat/stream", post(chat_stream))
        .route("/api/model/status", get(model_status))
        .route("/api/model/stack", post(model_stack))
        .route("/api/model/provider", post(model_provider))
        .route("/static/{*rest}", get(static_files))
        .route("/", get(index))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(app)
}

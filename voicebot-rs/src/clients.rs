//! clients.rs — 对齐 voicebot step_clients.py：ASR / LLM / TTS / 绘图客户端。

use base64::Engine;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::config::{self, cfg, DEFAULT_USER_AGENT};

#[derive(Debug, Clone)]
pub struct StepError {
    pub what: String,
    pub status: u16,
    pub body: String,
}

impl StepError {
    pub fn new(what: &str, status: u16, body: &str) -> Self {
        Self { what: what.to_string(), status, body: body.to_string() }
    }
    pub fn msg(what: &str) -> Self {
        Self::new(what, 0, "")
    }
    /// python: e.body[:300] or e.what
    pub fn detail(&self) -> String {
        let head: String = self.body.chars().take(300).collect();
        if !head.is_empty() { head } else { self.what.clone() }
    }
    pub fn is_image_moderation(&self) -> bool {
        let hay = format!("{} {}", self.body, self.what).to_lowercase();
        hay.contains("content moderation") || hay.contains("rejected") || hay.contains("invalid argument")
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let head: String = self.body.chars().take(500).collect();
        write!(f, "{} [status={}] body={}", self.what, self.status, head)
    }
}

pub type StepResult<T> = Result<T, StepError>;

// ── text helpers ───────────────────────────────────────────────────────────

/// python _clean: 去 ```代码块```、去 [#*`_>] 字符、trim。
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    // remove ```...``` blocks (DOTALL)
    while let Some(start) = rest.find("```") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        match after.find("```") {
            Some(end) => rest = &after[end + 3..],
            None => { rest = after; break; }
        }
    }
    out.push_str(rest);
    out.chars()
        .filter(|c| !matches!(c, '#' | '*' | '`' | '_' | '>' ))
        .collect::<String>()
        .trim()
        .to_string()
}

/// 句末标点切分（python: re.split(r"(?<=[。！？!?…])")）。
pub fn split_at_sentence_end(text: &str) -> Vec<String> {
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

/// python split_sentences: 返回 (完整句, 剩余)。
pub fn split_sentences(text: &str) -> (Vec<String>, String) {
    let parts = split_at_sentence_end(text);
    if parts.len() <= 1 {
        return (vec![], text.to_string());
    }
    let remainder = parts.last().cloned().unwrap_or_default();
    let complete = parts[..parts.len() - 1]
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.to_string())
        .collect();
    (complete, remainder)
}

const GROK_VOICE_MODEL: &str = "x-ai/grok-voice-tts-1.0";
const GROK_PREFIX: &str = "<slow><soft>[breath] ";
const GROK_SUFFIX: &str = " [sigh]</soft></slow>";

pub fn is_grok_voice_tts(tts_cfg: &Value) -> bool {
    tts_cfg.get("model").and_then(Value::as_str).unwrap_or("").to_lowercase() == GROK_VOICE_MODEL
}

pub fn prefers_whole_reply_tts(tts_cfg: &Value) -> bool {
    is_grok_voice_tts(tts_cfg)
}

fn is_step_audio_chat(tts_cfg: &Value) -> bool {
    tts_cfg.get("provider").and_then(Value::as_str) == Some("step_audio_chat")
}

fn is_bfl_image_model(img_cfg: &Value) -> bool {
    let model = img_cfg.get("model").and_then(Value::as_str).unwrap_or("").to_lowercase();
    model.contains("black-forest-labs/") || model.contains("flux")
}

fn add_grok_feminine_speech_tags(text: &str) -> String {
    let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return clean;
    }
    if clean.contains("<soft>") || clean.contains("<whisper>") || clean.contains("[breath]") {
        return clean;
    }
    let mut tagged = String::with_capacity(clean.len() + 32);
    for ch in clean.chars() {
        tagged.push(ch);
        if matches!(ch, '。' | '！' | '？' | '!' | '?' | '…') {
            tagged.push_str(" [pause] ");
        } else if matches!(ch, '，' | '、' | '；' | ';') {
            tagged.push_str(" [breath] ");
        }
    }
    let collapsed: String = tagged.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{GROK_PREFIX}{collapsed}{GROK_SUFFIX}")
}

pub fn bfl_safe_scene_prompt(prompt: &str) -> String {
    const REPL: &[(&str, &str)] = &[
        ("虚构成年人", "原创国风角色"),
        ("虚构成年", "原创国风人物"),
        ("成年人", "角色"),
        ("成年", "角色"),
        ("成人", "角色"),
        ("自愿互动", "戏剧互动"),
        ("成熟亲密", "细腻情绪"),
        ("成熟", "沉静"),
        ("亲密", "微妙"),
        ("暧昧", "悬念"),
        ("性感", "优雅"),
        ("情欲", "情绪"),
        ("裸露", ""),
        ("紧身", "合身"),
        ("曲线", "轮廓"),
        ("白皙脚踝", "衣摆细节"),
        ("脚踝", "衣摆"),
        ("唇角", "神情"),
    ];
    let mut text = prompt.to_string();
    for (src, dst) in REPL {
        text = text.replace(src, dst);
    }
    // 指尖轻触…唇… → 指尖轻触桌面道具；半尺…距离 → 近景对峙
    let re_touch = regex::Regex::new(r"指尖轻触[^，。；;]*唇[^，。；;]*").unwrap();
    text = re_touch.replace_all(&text, "指尖轻触桌面道具").into_owned();
    let re_half = regex::Regex::new(r"半尺[^，。；;]*距离").unwrap();
    text = re_half.replace_all(&text, "近景对峙").into_owned();
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_matches(|c: char| c == ' ' || c == '，' || c == ',' || c == '；' || c == ';').to_string()
}

// ── headers ────────────────────────────────────────────────────────────────

fn auth_headers(cfgv: &Value) -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert("content-type", "application/json".parse().unwrap());
    h.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
    if let Some(key) = cfgv.get("api_key").and_then(Value::as_str) {
        if !key.is_empty() {
            if let Ok(v) = format!("Bearer {key}").parse() {
                h.insert("authorization", v);
            }
        }
    }
    let provider = cfgv.get("provider").and_then(Value::as_str).unwrap_or("");
    if provider.starts_with("openrouter") {
        h.insert("http-referer", "https://voice.ajw.cn".parse().unwrap());
        h.insert("x-title", "Rsaga TRPG Bot".parse().unwrap());
    }
    h
}

fn opt_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn is_local_qwen(cfgv: &Value) -> bool {
    let base = opt_str(cfgv, "base_url").trim_end_matches('/').to_string();
    let model = opt_str(cfgv, "model");
    base.contains("qwen.ajw.cn") || model.starts_with("qwen3-")
}

// ── token estimation / local qwen trimming (python parity) ────────────────

pub fn estimate_text_tokens(text: &str) -> i64 {
    if text.is_empty() { return 0; }
    let mut wide = 0i64;
    let mut narrow = 0i64;
    for ch in text.chars() {
        if ch as u32 > 127 { wide += 1; } else { narrow += 1; }
    }
    wide + (narrow + 3) / 4
}

fn estimate_content_tokens(content: &Value) -> i64 {
    match content {
        Value::String(s) => estimate_text_tokens(s),
        Value::Array(items) => items.iter().map(|item| {
            match item {
                Value::Object(o) => {
                    let t = o.get("type").and_then(Value::as_str).unwrap_or("");
                    if t == "text" {
                        estimate_text_tokens(o.get("text").and_then(Value::as_str).unwrap_or(""))
                    } else if t == "image_url" {
                        512
                    } else {
                        0
                    }
                }
                other => estimate_text_tokens(&other.to_string()),
            }
        }).sum(),
        other => estimate_text_tokens(&other.to_string()),
    }
}

fn estimate_message_tokens(msg: &Value) -> i64 {
    8 + estimate_content_tokens(msg.get("content").unwrap_or(&Value::Null))
}

fn estimate_messages_tokens(msgs: &[Value]) -> i64 {
    msgs.iter().map(estimate_message_tokens).sum()
}

fn local_qwen_prompt_budget(max_tokens: i64) -> i64 {
    let c = cfg();
    if c.local_qwen_prompt_budget_tokens > 0 {
        return c.local_qwen_prompt_budget_tokens;
    }
    let mt = if max_tokens > 0 { max_tokens } else { 2400 };
    (c.local_qwen_context_tokens - mt - c.local_qwen_context_margin_tokens).max(512)
}

fn truncate_text_to_budget(text: &str, budget: i64) -> String {
    if estimate_text_tokens(text) <= budget {
        return text.to_string();
    }
    if budget <= 0 {
        return String::new();
    }
    let mut trimmed = text.to_string();
    while !trimmed.is_empty() && estimate_text_tokens(&trimmed) > budget {
        let estimate = estimate_text_tokens(&trimmed).max(1);
        let mut keep = ((trimmed.chars().count() as i64) * budget / estimate - 1).max(1) as usize;
        if keep >= trimmed.chars().count() {
            keep = trimmed.chars().count().saturating_sub(1);
        }
        trimmed = trimmed.chars().skip(trimmed.chars().count() - keep).collect();
    }
    if trimmed.is_empty() { String::new() } else { format!("…{trimmed}") }
}

fn truncate_message_to_budget(msg: &Value, budget: i64) -> Value {
    let mut trimmed = msg.clone();
    let content_budget = (budget - 8).max(0);
    let content = msg.get("content").cloned().unwrap_or(Value::String(String::new()));
    match content {
        Value::Array(items) => {
            let mut remaining = content_budget;
            let mut new_content = Vec::new();
            for item in items {
                if let Value::Object(o) = &item {
                    let t = o.get("type").and_then(Value::as_str).unwrap_or("");
                    if t == "image_url" {
                        if remaining >= 512 {
                            new_content.push(item.clone());
                            remaining -= 512;
                        }
                        continue;
                    }
                    if t == "text" {
                        let text = o.get("text").and_then(Value::as_str).unwrap_or("");
                        let cut = truncate_text_to_budget(text, remaining);
                        if !cut.is_empty() {
                            let mut copied = o.clone();
                            copied.insert("text".into(), json!(cut));
                            let cost = estimate_text_tokens(&cut);
                            new_content.push(Value::Object(copied));
                            remaining -= cost;
                        }
                        continue;
                    }
                    let cost = estimate_content_tokens(&item);
                    if cost <= remaining {
                        new_content.push(item.clone());
                        remaining -= cost;
                    }
                } else {
                    let text = truncate_text_to_budget(&item.to_string(), remaining);
                    if !text.is_empty() {
                        new_content.push(json!(text));
                        remaining -= estimate_text_tokens(&text);
                    }
                }
            }
            if let Value::Object(t) = &mut trimmed {
                t.insert("content".into(), json!(new_content));
            }
        }
        Value::String(s) => {
            let cut = truncate_text_to_budget(&s, content_budget);
            if let Value::Object(t) = &mut trimmed {
                t.insert("content".into(), json!(cut));
            }
        }
        _ => {}
    }
    trimmed
}

fn trim_messages_for_local_qwen(messages: &[Value], max_tokens: i64) -> Vec<Value> {
    let budget = local_qwen_prompt_budget(max_tokens);
    if estimate_messages_tokens(messages) <= budget || messages.is_empty() {
        return messages.to_vec();
    }
    let start = if messages[0].get("role").and_then(Value::as_str) == Some("system") { 1 } else { 0 };
    let system: Vec<Value> = if start == 1 { vec![messages[0].clone()] } else { vec![] };
    if start >= messages.len() {
        return vec![truncate_message_to_budget(&messages[0], budget)];
    }
    let tail = messages.last().unwrap().clone();
    let middle = &messages[start..messages.len() - 1];
    let mut base = system.clone();
    base.push(tail.clone());
    let base_tokens = estimate_messages_tokens(&base);
    if base_tokens > budget {
        let available = (budget - estimate_messages_tokens(&system)).max(64);
        let trimmed_tail = truncate_message_to_budget(&tail, available);
        let mut trimmed = system;
        trimmed.push(trimmed_tail);
        return trimmed;
    }
    let mut used = base_tokens;
    let mut kept_rev = Vec::new();
    for msg in middle.iter().rev() {
        let cost = estimate_message_tokens(msg);
        if used + cost <= budget {
            kept_rev.push(msg.clone());
            used += cost;
        }
    }
    kept_rev.reverse();
    let mut trimmed = system;
    trimmed.extend(kept_rev);
    trimmed.push(tail);
    trimmed
}

// ── LLM ────────────────────────────────────────────────────────────────────

pub struct PreparedLlm {
    pub model: String,
    pub messages: Vec<Value>,
    pub sampling: Value,
}

pub fn prepare_llm(history: &[Value], image_data_url: Option<&str>) -> PreparedLlm {
    let llm_cfg = config::get_llm_config();
    let persona = config::get_active_persona();
    let mut messages = vec![json!({"role": "system", "content": persona.get("prompt").and_then(Value::as_str).unwrap_or("")})];
    messages.extend(history.iter().cloned());

    let mut model = opt_str(&llm_cfg, "model");
    if let Some(img) = image_data_url {
        if let Some(last) = messages.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some("user") {
                let vision = opt_str(&llm_cfg, "vision_model");
                if !vision.is_empty() {
                    model = vision;
                }
                let text_part = last.get("content").and_then(Value::as_str).unwrap_or("").to_string();
                let text_part = if text_part.is_empty() { "看图回答".to_string() } else { text_part };
                *last = json!({"role": "user", "content": [
                    {"type": "image_url", "image_url": {"url": img}},
                    {"type": "text", "text": text_part},
                ]});
            }
        }
    }
    if is_local_qwen(&llm_cfg) {
        let sampling = llm_sampling_params(&llm_cfg, &persona);
        let mt = sampling.get("max_tokens").and_then(Value::as_i64).unwrap_or(0);
        messages = trim_messages_for_local_qwen(&messages, mt);
    }
    PreparedLlm { model, messages, sampling: llm_sampling_params(&llm_cfg, &persona) }
}

fn llm_sampling_params(cfgv: &Value, persona: &Value) -> Value {
    let c = cfg();
    if is_local_qwen(cfgv) {
        let mut p = json!({
            "temperature": c.llm_temperature,
            "top_p": c.llm_top_p,
            "max_tokens": c.llm_max_tokens,
        });
        if c.llm_min_tokens > 0 {
            p["min_tokens"] = json!(c.llm_min_tokens);
        }
        if c.llm_repeat_penalty > 0.0 {
            p["repeat_penalty"] = json!(c.llm_repeat_penalty);
        }
        return p;
    }
    json!({
        "temperature": persona.get("temperature").and_then(Value::as_f64).unwrap_or(1.0),
        "top_p": 0.9,
        "max_tokens": c.llm_max_tokens,
    })
}

pub async fn llm(client: &reqwest::Client, history: &[Value]) -> StepResult<String> {
    let llm_cfg = config::get_llm_config();
    let prep = prepare_llm(history, None);
    let mut payload = json!({"model": prep.model, "messages": prep.messages, "stream": false});
    if let (Value::Object(p), Value::Object(s)) = (&mut payload, &prep.sampling) {
        for (k, v) in s {
            p.insert(k.clone(), v.clone());
        }
    }
    let url = format!("{}/chat/completions", opt_str(&llm_cfg, "base_url"));
    let resp = client.post(&url).headers(auth_headers(&llm_cfg)).json(&payload).timeout(std::time::Duration::from_secs(180)).send().await
        .map_err(|e| StepError::new(&format!("LLM 请求异常: {e}"), 0, ""))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(StepError::new("LLM HTTP 失败", status, &text));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| StepError::new("LLM 返回结构异常", status, &text))?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| StepError::new("LLM 返回结构异常", status, &text))
}

/// Streaming LLM: returns pieces of delta.content. Python llm_stream parity.
pub async fn llm_stream(client: &reqwest::Client, history: &[Value], image_data_url: Option<&str>)
    -> StepResult<Vec<String>>
{
    let llm_cfg = config::get_llm_config();
    let prep = prepare_llm(history, image_data_url);
    let mut payload = json!({"model": prep.model, "messages": prep.messages, "stream": true});
    if let (Value::Object(p), Value::Object(s)) = (&mut payload, &prep.sampling) {
        for (k, v) in s {
            p.insert(k.clone(), v.clone());
        }
    }
    let url = format!("{}/chat/completions", opt_str(&llm_cfg, "base_url"));
    let resp = client.post(&url).headers(auth_headers(&llm_cfg)).json(&payload).timeout(std::time::Duration::from_secs(300)).send().await
        .map_err(|e| StepError::new(&format!("LLM 流式异常: {e}"), 0, ""))?;
    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(StepError::new("LLM HTTP 失败", status, &body));
    }
    let mut pieces = Vec::new();
    let mut buf = String::new();
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = std::pin::Pin::new(&mut stream).next().await {
        let chunk = chunk.map_err(|e| StepError::new(&format!("LLM 流式异常: {e}"), 0, ""))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            let line = line.trim();
            if line.is_empty() { continue; }
            let data = line.strip_prefix("data:").map(|s| s.trim()).unwrap_or(line);
            if data == "[DONE]" { return Ok(pieces); }
            let Ok(v) = serde_json::from_str::<Value>(data) else { continue; };
            let piece = v["choices"][0]["delta"]["content"].as_str();
            if let Some(p) = piece {
                if !p.is_empty() {
                    pieces.push(p.to_string());
                }
            }
        }
    }
    Ok(pieces)
}

// ── ASR ────────────────────────────────────────────────────────────────────

fn ffmpeg_bin() -> String {
    let c = cfg();
    let configured = c.ffmpeg_bin.trim();
    if !configured.is_empty() {
        if let Some(found) = which(configured) { return found; }
        if std::path::Path::new(configured).exists() { return configured.to_string(); }
        if configured != "ffmpeg" { return configured.to_string(); }
    }
    if let Some(found) = which("ffmpeg") { return found; }
    configured.to_string()
}


fn which(bin: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        let p = std::path::Path::new(dir).join(bin);
        if p.is_file() {
            if let Ok(md) = std::fs::metadata(&p) {
                use std::os::unix::fs::PermissionsExt;
                if md.permissions().mode() & 0o111 != 0 {
                    return p.to_str().map(|s| s.to_string());
                }
            }
        }
    }
    None
}

fn ext_for_mime(mime: &str) -> &'static str {
    let m = mime.to_lowercase();
    if m.contains("webm") { ".webm" }
    else if m.contains("ogg") { ".ogg" }
    else if m.contains("mp4") || m.contains("m4a") { ".m4a" }
    else if m.contains("mp3") { ".mp3" }
    else if m.contains("wav") { ".wav" }
    else { ".bin" }
}

pub async fn to_wav(audio: &[u8], mime: &str) -> StepResult<Vec<u8>> {
    let src = std::env::temp_dir().join(format!("vb_asr_{}{}", std::process::id(), ext_for_mime(mime)));
    let dst = std::env::temp_dir().join(format!("vb_asr_{}.wav", std::process::id()));
    std::fs::write(&src, audio).map_err(|e| StepError::new("ffmpeg 不可用", 0, &e.to_string()))?;
    let out = tokio::process::Command::new(ffmpeg_bin())
        .args(["-y", "-i"])
        .arg(&src)
        .args(["-ar", "16000", "-ac", "1"])
        .arg(&dst)
        .output()
        .await
        .map_err(|e| StepError::new("ffmpeg 不可用", 0, &e.to_string()))?;
    let _ = std::fs::remove_file(&src);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        let _ = std::fs::remove_file(&dst);
        return Err(StepError::new("ffmpeg 转码失败", 0, &err));
    }
    let bytes = std::fs::read(&dst).unwrap_or_default();
    let _ = std::fs::remove_file(&dst);
    Ok(bytes)
}

pub async fn asr(client: &reqwest::Client, audio: &[u8], mime: &str) -> StepResult<String> {
    let wav = to_wav(audio, mime).await?;
    let asr_cfg = config::get_asr_config();
    let url = format!("{}/audio/transcriptions", opt_str(&asr_cfg, "base_url"));
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
    let key = opt_str(&asr_cfg, "api_key");
    if !key.is_empty() {
        if let Ok(v) = format!("Bearer {key}").parse() {
            headers.insert("authorization", v);
        }
    }
    let resp = if opt_str(&asr_cfg, "transport") == "openrouter_json" {
        let payload = json!({
            "model": opt_str(&asr_cfg, "model"),
            "input_audio": {"data": base64::engine::general_purpose::STANDARD.encode(&wav), "format": "wav"},
            "language": "zh",
        });
        client.post(&url).headers(headers).json(&payload).timeout(std::time::Duration::from_secs(120)).send().await
    } else {
        let form = reqwest::multipart::Form::new()
            .part("file", reqwest::multipart::Part::bytes(wav)
                .file_name("audio.wav")
                .mime_str("audio/wav").unwrap())
            .text("model", opt_str(&asr_cfg, "model"));
        client.post(&url).headers(headers).multipart(form).timeout(std::time::Duration::from_secs(120)).send().await
    }
    .map_err(|e| StepError::new(&format!("ASR 请求异常: {e}"), 0, ""))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(StepError::new("ASR HTTP 失败", status, &text));
    }
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Ok(text.trim().to_string()),
    };
    let out = v.get("text").and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| v.get("result").and_then(|r| r.get("text")).and_then(Value::as_str).map(|s| s.to_string()));
    match out {
        Some(t) if !t.is_empty() => Ok(t.trim().to_string()),
        _ => Err(StepError::new("ASR 返回无 text 字段", status, &text)),
    }
}

// ── TTS ────────────────────────────────────────────────────────────────────

fn build_tts_payload(text: &str, voice: &str, tts_cfg: &Value) -> Value {
    let input_text = if is_grok_voice_tts(tts_cfg) {
        add_grok_feminine_speech_tags(text)
    } else {
        text.to_string()
    };
    let c = cfg();
    let mut payload = json!({
        "model": opt_str(tts_cfg, "model"),
        "input": input_text,
        "voice": if voice.is_empty() {
            let v = opt_str(tts_cfg, "voice");
            if v.is_empty() { c.tts_voice.clone() } else { v }
        } else { voice.to_string() },
    });
    if let Value::Object(p) = &mut payload {
        let instr = opt_str(tts_cfg, "instruction");
        if !instr.is_empty() { p.insert("instruction".into(), json!(instr)); }
        let fmt = opt_str(tts_cfg, "response_format");
        if !fmt.is_empty() { p.insert("response_format".into(), json!(fmt)); }
        let speed = opt_str(tts_cfg, "speed");
        if !speed.is_empty() {
            if let Ok(f) = speed.parse::<f64>() {
                p.insert("speed".into(), json!(f));
            }
        }
    }
    payload
}

fn tts_cache_path(payload: &Value, tts_cfg: &Value) -> Option<std::path::PathBuf> {
    if !cfg().tts_cache_enabled { return None; }
    let text = payload.get("input").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if text.is_empty() { return None; }
    let mut key = Map::new();
    key.insert("base".into(), json!(opt_str(tts_cfg, "base_url")));
    key.insert("model".into(), payload.get("model").cloned().unwrap_or(Value::Null));
    key.insert("voice".into(), payload.get("voice").cloned().unwrap_or(Value::Null));
    key.insert("instruction".into(), payload.get("instruction").cloned().unwrap_or(Value::Null));
    key.insert("response_format".into(), payload.get("response_format").cloned().unwrap_or(Value::Null));
    key.insert("speed".into(), payload.get("speed").cloned().unwrap_or(Value::Null));
    key.insert("input".into(), json!(text));
    let serialized = serde_json::to_string(&Value::Object(key)).unwrap_or_default();
    let digest = Sha256::digest(serialized.as_bytes());
    Some(cfg().tts_cache_dir.join(format!("{:x}.audio", digest)))
}

fn write_tts_cache(path: &std::path::PathBuf, content: &[u8]) {
    if content.is_empty() { return; }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension(format!("audio.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, path)).is_err() {
        config::log_warn(&format!("写入 TTS 缓存失败 {}", path.display()));
    }
}

pub async fn tts(client: &reqwest::Client, text: &str, voice: &str) -> StepResult<Vec<u8>> {
    let tts_cfg = config::get_tts_config();
    let payload = build_tts_payload(text, voice, &tts_cfg);
    if let Some(cache) = tts_cache_path(&payload, &tts_cfg) {
        if let Ok(bytes) = std::fs::read(&cache) {
            return Ok(bytes);
        }
    }
    let bytes = if is_step_audio_chat(&tts_cfg) {
        step_audio_chat_tts(client, text, voice, &tts_cfg).await?
    } else {
        speech_tts(client, &payload, &tts_cfg).await?
    };
    if let Some(cache) = tts_cache_path(&payload, &tts_cfg) {
        write_tts_cache(&cache, &bytes);
    }
    Ok(bytes)
}

async fn speech_tts(client: &reqwest::Client, payload: &Value, tts_cfg: &Value) -> StepResult<Vec<u8>> {
    let url = format!("{}/audio/speech", opt_str(tts_cfg, "base_url"));
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
    let key = opt_str(tts_cfg, "api_key");
    if !key.is_empty() {
        if let Ok(v) = format!("Bearer {key}").parse() {
            headers.insert("authorization", v);
        }
    }
    let openrouter = opt_str(tts_cfg, "base_url").contains("openrouter.ai");
    for attempt in 0..5u32 {
        let resp = client.post(&url).headers(headers.clone()).json(payload).timeout(std::time::Duration::from_secs(120)).send().await
            .map_err(|e| StepError::new(&format!("TTS 请求异常: {e}"), 0, ""))?;
        let status = resp.status().as_u16();
        if status == 429 {
            tokio::time::sleep(std::time::Duration::from_secs((6 * (attempt as u64 + 1)).min(30))).await;
            continue;
        }
        if openrouter && (500..=504).contains(&status) && attempt < 4 {
            tokio::time::sleep(std::time::Duration::from_secs_f64((1.5 * (attempt as f64 + 1.0)).min(6.0))).await;
            continue;
        }
        let bytes = resp.bytes().await.unwrap_or_default();
        if status != 200 {
            return Err(StepError::new("TTS HTTP 失败", status, &String::from_utf8_lossy(&bytes)));
        }
        return Ok(bytes.to_vec());
    }
    Err(StepError::new("TTS 多次限流重试仍失败", 429, "rate limited"))
}

async fn step_audio_chat_tts(client: &reqwest::Client, text: &str, voice: &str, tts_cfg: &Value) -> StepResult<Vec<u8>> {
    let url = format!("{}/chat/completions", opt_str(tts_cfg, "base_url"));
    let c = cfg();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
    let key = opt_str(tts_cfg, "api_key");
    if !key.is_empty() {
        if let Ok(v) = format!("Bearer {key}").parse() {
            headers.insert("authorization", v);
        }
    }
    let voice_value = {
        let v = if voice.is_empty() { opt_str(tts_cfg, "voice") } else { voice.to_string() };
        if v.is_empty() { "qingchunshaonv".to_string() } else { v }
    };
    let format_value = {
        let f = opt_str(tts_cfg, "response_format");
        if f.is_empty() { "wav".to_string() } else { f }
    };
    let payload = json!({
        "model": opt_str(tts_cfg, "model"),
        "modalities": ["text", "audio"],
        "messages": [{"role": "user", "content": text}],
        "audio": {"voice": voice_value, "format": format_value},
        "stream": false,
    });
    let _ = c;
    for attempt in 0..5u32 {
        let resp = client.post(&url).headers(headers.clone()).json(&payload).timeout(std::time::Duration::from_secs(120)).send().await
            .map_err(|e| StepError::new(&format!("TTS 请求异常: {e}"), 0, ""))?;
        let status = resp.status().as_u16();
        if status == 429 {
            tokio::time::sleep(std::time::Duration::from_secs((6 * (attempt as u64 + 1)).min(30))).await;
            continue;
        }
        let text = resp.text().await.unwrap_or_default();
        if status != 200 {
            return Err(StepError::new("TTS HTTP 失败", status, &text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| StepError::new("TTS 返回结构异常", status, &text))?;
        let b64 = v["choices"][0]["message"]["audio"]["data"].as_str().unwrap_or("");
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64)
            .map_err(|_| StepError::new("TTS 音频解码失败", status, &text))?;
        return Ok(bytes);
    }
    Err(StepError::new("TTS 多次限流重试仍失败", 429, "rate limited"))
}

// ── 背景图 ─────────────────────────────────────────────────────────────────

pub async fn summarize_for_image(client: &reqwest::Client, story_text: &str, persona: &str) -> StepResult<String> {
    let cfgv = config::get_summary_config();
    let sys = "你是中国传统国风插画分镜画师，擅长水墨写意与工笔重彩。根据给定文本提炼一段详细、可直接用于文生图的当前剧情画面描述。完整阅读文本，不要只抓第一句；优先保留当前剧情最关键的动作和情绪转折。具体描绘时间、地点、空间布局、前景与背景、人物站位、神态、眼神、手部动作、身体倾向、中式服饰与发饰、发丝、关键道具、镜头距离、视线方向、墨色晕染、设色层次、留白和画面构图。女性角色必须是精致美型，优先描述对称自然的面部结构、协调的五官比例、清澈有神的双眼、自然眉形、挺秀鼻梁和清晰优美的唇形；明确避免多眼、多眉、歪嘴、五官错位、重复面部、模糊脸和畸形脸。若画面中出现男性，男性一律只作为黑色剪影、背影或暗影轮廓出镜，不描绘男性五官、皮肤、衣饰细节。画面应保留剧情需要的张力，用光影、构图、道具、距离和空间关系表达人物关系。不要使用现实公众人物形象；不要生成文字、标题、对白或字幕。输出约180到300个中文字符的单段画面描述，只描写可见元素，不要解释、前缀或引号。";
    let user = format!("角色：{persona}\n内容：{story_text}\n画面描述：");
    let payload = json!({
        "model": opt_str(&cfgv, "model"),
        "messages": [{"role": "system", "content": sys}, {"role": "user", "content": user}],
        "stream": false, "temperature": 0.4, "max_tokens": 512,
    });
    let url = format!("{}/chat/completions", opt_str(&cfgv, "base_url"));
    let resp = client.post(&url).headers(auth_headers(&cfgv)).json(&payload).timeout(std::time::Duration::from_secs(30)).send().await
        .map_err(|e| StepError::new(&format!("总结请求异常: {e}"), 0, ""))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(StepError::new("总结 HTTP 失败", status, &text));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| StepError::new("总结返回异常", status, &text))?;
    v["choices"][0]["message"]["content"].as_str().map(|s| s.trim().to_string())
        .ok_or_else(|| StepError::new("总结返回异常", status, &text))
}

pub async fn gen_bg_image(client: &reqwest::Client, prompt: &str) -> StepResult<String> {
    let img_cfg = config::get_img_config();
    let (clean_prompt, story_boundaries) = if is_bfl_image_model(&img_cfg) {
        (bfl_safe_scene_prompt(prompt),
        "原创国风角色，女性角色五官精致、面部结构对称、双眼清澈有神，神态、姿态、行为、眼神和情绪细节清晰，男性角色仅以黑色剪影、背影或暗影轮廓出现，不描绘男性面部细节，电影感构图，光影和环境氛围突出，用道具、表情和空间关系表现剧情张力，无文字、无logo、无水印")
    } else {
        (prompt.to_string(),
        "女性角色必须拥有精致秀美、结构对称、比例协调的五官，双眼清澈有神，面部完整自然，不出现多眼、歪嘴、五官错位或畸形脸；女性角色的神态、姿态、行为、眼神和情绪细节要细腻清晰；男性角色仅以黑色剪影、背影或暗影轮廓出现，不描绘男性五官和皮肤细节；服装、姿态、距离、表情与情绪张力按剧情需要呈现；不要使用现实公众人物形象；不要呈现胁迫、暴力血腥、自残或违法内容；无文字、无logo、无水印")
    };
    let full_prompt = format!("{}，{clean_prompt}，{story_boundaries}", cfg().img_style_prefix);
    let provider = opt_str(&img_cfg, "provider");
    if provider.starts_with("openrouter") {
        return gen_bg_openrouter(client, &img_cfg, &full_prompt).await;
    }
    if provider == "local_boogu" {
        return gen_bg_local_boogu(client, &img_cfg, &full_prompt).await;
    }
    gen_bg_step(client, &img_cfg, &full_prompt).await
}

async fn gen_bg_local_boogu(client: &reqwest::Client, img_cfg: &Value, full_prompt: &str) -> StepResult<String> {
    let payload = json!({"model": opt_str(img_cfg, "model"), "prompt": full_prompt, "size": cfg().img_size, "n": 1});
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-api-key", opt_str(img_cfg, "api_key").parse().unwrap());
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
    let url = format!("{}/images/generations", opt_str(img_cfg, "base_url"));
    let resp = client.post(&url).headers(headers).json(&payload).timeout(std::time::Duration::from_secs(300)).send().await
        .map_err(|e| StepError::new(&format!("Boogu 绘图请求异常: {e}"), 0, ""))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(StepError::new("Boogu 绘图 HTTP 失败", status, &text));
    }
    extract_generation_image(&text, "Boogu 绘图")
}

async fn gen_bg_step(client: &reqwest::Client, img_cfg: &Value, full_prompt: &str) -> StepResult<String> {
    let payload = json!({"model": opt_str(img_cfg, "model"), "prompt": full_prompt, "size": cfg().img_size, "n": 1});
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {}", opt_str(img_cfg, "api_key")).parse().unwrap());
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
    let url = format!("{}/images/generations", opt_str(img_cfg, "base_url"));
    let resp = client.post(&url).headers(headers).json(&payload).timeout(std::time::Duration::from_secs(180)).send().await
        .map_err(|e| StepError::new(&format!("绘图请求异常: {e}"), 0, ""))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(StepError::new("绘图 HTTP 失败", status, &text));
    }
    extract_generation_image(&text, "绘图")
}

fn extract_generation_image(text: &str, what: &str) -> StepResult<String> {
    let v: Value = serde_json::from_str(text).map_err(|_| StepError::new(&format!("{what}返回无图"), 0, text))?;
    let item = v.get("data").and_then(Value::as_array).and_then(|a| a.first()).cloned()
        .ok_or_else(|| StepError::new(&format!("{what}返回无图"), 0, text))?;
    if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
        return Ok(b64.to_string());
    }
    let url = item.get("url").and_then(Value::as_str).unwrap_or("");
    if url.is_empty() {
        return Err(StepError::new(&format!("{what}返回无 url/b64"), 0, text));
    }
    Ok(String::new()) // resolved by caller via download below
}

async fn gen_bg_openrouter(client: &reqwest::Client, img_cfg: &Value, full_prompt: &str) -> StepResult<String> {
    let model = opt_str(img_cfg, "model");
    let provider = opt_str(img_cfg, "provider");
    if provider == "openrouter_images" || !model.starts_with("x-ai/grok-imagine") {
        // OpenRouter Images API
        let payload = json!({"model": model, "prompt": full_prompt, "n": 1, "output_format": "png"});
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("authorization", format!("Bearer {}", opt_str(img_cfg, "api_key")).parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("http-referer", "https://voice.ajw.cn".parse().unwrap());
        headers.insert("x-title", "Rsaga TRPG Bot".parse().unwrap());
        headers.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
        let url = format!("{}/images", opt_str(img_cfg, "base_url"));
        let resp = client.post(&url).headers(headers).json(&payload).timeout(std::time::Duration::from_secs(240)).send().await
            .map_err(|e| StepError::new(&format!("OpenRouter Images 绘图请求异常: {e}"), 0, ""))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        if status != 200 {
            return Err(StepError::new("OpenRouter Images 绘图 HTTP 失败", status, &text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| StepError::new("OpenRouter Images 绘图返回结构异常", status, &text))?;
        let item = v.get("data").and_then(Value::as_array).and_then(|a| a.first()).cloned()
            .ok_or_else(|| StepError::new("OpenRouter Images 绘图返回结构异常", status, &text))?;
        if let Some(b64) = item.get("b64_json").or_else(|| item.get("base64")).and_then(Value::as_str) {
            return resolve_image_ref(client, b64, "OpenRouter Images").await;
        }
        if let Some(r) = item.get("url").or_else(|| item.get("image_url")) {
            return resolve_image_ref(client, &r.to_string(), "OpenRouter Images").await;
        }
        return Err(StepError::new("OpenRouter Images 绘图返回无图", status, &text));
    }

    // Grok Imagine via chat/completions
    let payload = json!({"model": model, "messages": [{"role": "user", "content": full_prompt}], "stream": false, "max_tokens": 4096});
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {}", opt_str(img_cfg, "api_key")).parse().unwrap());
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("http-referer", "https://voice.ajw.cn".parse().unwrap());
    headers.insert("x-title", "Rsaga TRPG Bot".parse().unwrap());
    headers.insert("user-agent", DEFAULT_USER_AGENT.parse().unwrap());
    let url = format!("{}/chat/completions", opt_str(img_cfg, "base_url"));
    let resp = client.post(&url).headers(headers).json(&payload).timeout(std::time::Duration::from_secs(180)).send().await
        .map_err(|e| StepError::new(&format!("OpenRouter 绘图请求异常: {e}"), 0, ""))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(StepError::new("OpenRouter 绘图 HTTP 失败", status, &text));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| StepError::new("OpenRouter 绘图返回结构异常", status, &text))?;
    let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();

    // message.images 字段
    if let Some(images) = v["choices"][0]["message"]["images"].as_array() {
        if let Some(first) = images.first() {
            let r = first.get("image_url").or_else(|| first.get("url")).and_then(Value::as_str);
            if let Some(r) = r {
                return resolve_image_ref(client, r, "OpenRouter").await;
            }
        }
    }
    let re_md = regex::Regex::new(r"!\[.*?\]\((https?://[^\s)]+)\)").unwrap();
    if let Some(caps) = re_md.captures(&content) {
        return resolve_image_ref(client, &caps[1].to_string(), "OpenRouter").await;
    }
    let re_url = regex::Regex::new(r"https?://[^\s]+\.[a-zA-Z]{2,}[^\s]*").unwrap();
    if let Some(caps) = re_url.captures(&content) {
        return resolve_image_ref(client, &caps[0].to_string(), "OpenRouter").await;
    }
    if content.len() > 100 && !content.starts_with("http") && content.starts_with("data:image") {
        if let Some((_, b64)) = content.split_once(',') {
            return Ok(b64.to_string());
        }
    }
    let head: String = content.chars().take(500).collect();
    Err(StepError::new("OpenRouter 绘图返回无图片 URL", status, &head))
}

pub async fn resolve_image_ref(client: &reqwest::Client, r: &str, provider_name: &str) -> StepResult<String> {
    let r = r.trim().trim_matches('"');
    if r.starts_with("data:image") {
        if let Some((_, b64)) = r.split_once(',') {
            return Ok(b64.to_string());
        }
        return Err(StepError::new(&format!("{provider_name} data URL 缺少 base64 数据"), 0, &r.chars().take(120).collect::<String>()));
    }
    let re_b64 = regex::Regex::new(r"^[A-Za-z0-9+/=\s]+$").unwrap();
    if re_b64.is_match(r) && r.len() > 100 {
        return Ok(r.split_whitespace().collect::<String>());
    }
    if !r.starts_with("http://") && !r.starts_with("https://") {
        return Err(StepError::new(&format!("{provider_name} 图片引用不是 URL/data URL"), 0, &r.chars().take(500).collect::<String>()));
    }
    let bytes = client.get(r).timeout(std::time::Duration::from_secs(60)).send().await
        .map_err(|e| StepError::new(&format!("下载 {provider_name} 图片异常: {e}"), 0, ""))?
        .bytes().await.map_err(|e| StepError::new(&format!("下载 {provider_name} 图片异常: {e}"), 0, ""))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

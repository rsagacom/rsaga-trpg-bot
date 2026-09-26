//! config.rs — 对齐 voicebot config.py：env + .env、模型目录、状态文件。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

pub const DEFAULT_USER_AGENT: &str = "RsagaTRPGBot/1.0 (+https://voice.ajw.cn)";

pub struct Cfg {
    pub base_dir: PathBuf,
    // StepAudio / ASR / TTS
    pub step_api_key: String,
    pub step_base: String,
    pub asr_model: String,
    pub asr_enabled: bool,
    pub tts_model: String,
    pub asr_base: String,
    pub asr_api_key: String,
    pub tts_base: String,
    pub tts_api_key: String,
    pub tts_response_format: String,
    pub tts_speed: String,
    pub tts_cache_enabled: bool,
    pub tts_cache_dir: PathBuf,
    pub tts_merge_target_chars: usize,
    pub tts_merge_max_chars: usize,
    pub tts_voice: String,
    pub tts_instruction: String,
    pub voice_options: Vec<(String, String)>,
    // LLM
    pub qwen_base_url: String,
    pub qwen_api_key: String,
    pub qwen_model: String,
    pub openrouter_base_url: String,
    pub openrouter_api_key: String,
    pub openrouter_model: String,
    pub openrouter_vision_model: String,
    pub openrouter_img_model: String,
    pub agnes_base_url: String,
    pub agnes_api_key: String,
    // image
    pub img_model: String,
    pub img_size: String,
    pub img_style_prefix: String,
    // summary
    pub summary_base_url: String,
    pub summary_model: String,
    pub summary_api_key: String,
    // local draw
    pub local_draw_base_url: String,
    pub local_draw_api_key: String,
    pub local_draw_model: String,
    // auth / sessions / limits
    pub bot_token: String,
    pub max_history: usize,
    pub llm_max_tokens: i64,
    pub llm_temperature: f64,
    pub llm_top_p: f64,
    pub llm_repeat_penalty: f64,
    pub llm_min_tokens: i64,
    pub ffmpeg_bin: String,
    pub grok_tts_chunk_chars: usize,
    pub local_qwen_context_tokens: i64,
    pub local_qwen_context_margin_tokens: i64,
    pub local_qwen_prompt_budget_tokens: i64,
    pub model_provider_env: String,
    pub vb_host: String,
    pub vb_port: u16,
}

fn envs(map: &BTreeMap<String, String>, name: &str, default: &str) -> String {
    if let Some(v) = map.get(name) {
        if !v.trim().is_empty() {
            return v.trim().to_string();
        }
    }
    default.to_string()
}

fn truthy(v: &str) -> bool {
    !matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "no" | "off" | "")
}

/// Parse a .env file (KEY=VALUE, strip quotes); existing process env wins over file (python-dotenv parity).
fn parse_env_file(path: &Path, into: &mut BTreeMap<String, String>) {
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let k = k.trim().to_string();
            let mut v = v.trim().to_string();
            if v.len() >= 2
                && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
            {
                v = v[1..v.len() - 1].to_string();
            }
            into.entry(k).or_insert(v);
        }
    }
}

impl Cfg {
    pub fn load() -> Cfg {
        let mut map: BTreeMap<String, String> = std::env::vars().collect();
        let base_dir = std::env::var("VB_BASE_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(Path::to_path_buf))
                    .unwrap_or_else(|| PathBuf::from("."))
            });
        parse_env_file(&base_dir.join(".env"), &mut map);

        let voice_options = envs(&map, "VOICE_OPTIONS", "默认女声=,默认男声=")
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                let (n, v) = s.split_once('=').unwrap_or((s, ""));
                (n.trim().to_string(), v.trim().to_string())
            })
            .collect();

        let step_api_key = envs(&map, "STEP_API_KEY", "");
        Cfg {
            step_base: envs(&map, "STEP_BASE", "https://api.stepfun.com/v1"),
            asr_model: envs(&map, "ASR_MODEL", "stepaudio-2.5-asr"),
            asr_enabled: truthy(&envs(&map, "ASR_ENABLED", "1")),
            tts_model: envs(&map, "TTS_MODEL", "stepaudio-2.5-tts"),
            asr_base: envs(&map, "ASR_BASE", &envs(&map, "STEP_BASE", "https://api.stepfun.com/v1")),
            asr_api_key: envs(&map, "ASR_API_KEY", &step_api_key),
            tts_base: envs(&map, "TTS_BASE", &envs(&map, "STEP_BASE", "https://api.stepfun.com/v1")),
            tts_api_key: envs(&map, "TTS_API_KEY", &step_api_key),
            tts_response_format: envs(&map, "TTS_RESPONSE_FORMAT", ""),
            tts_speed: envs(&map, "TTS_SPEED", ""),
            tts_cache_enabled: truthy(&envs(&map, "TTS_CACHE_ENABLED", "1")),
            tts_cache_dir: PathBuf::from(envs(&map, "TTS_CACHE_DIR", &base_dir.join("tts_cache").to_string_lossy())),
            tts_merge_target_chars: envs(&map, "TTS_MERGE_TARGET_CHARS", "48").parse().unwrap_or(48),
            tts_merge_max_chars: envs(&map, "TTS_MERGE_MAX_CHARS", "90").parse().unwrap_or(90),
            tts_voice: envs(&map, "TTS_VOICE", "your-voice-id"),
            tts_instruction: envs(&map, "TTS_INSTRUCTION", "自然语速，温柔亲昵"),
            step_api_key,
            voice_options,
            qwen_base_url: envs(&map, "QWEN_BASE_URL", "https://qwen.ajw.cn/v1"),
            qwen_api_key: envs(&map, "QWEN_API_KEY", ""),
            qwen_model: envs(&map, "QWEN_MODEL", "qwen"),
            openrouter_base_url: envs(&map, "OPENROUTER_BASE_URL", "https://openrouter.ai/api/v1"),
            openrouter_api_key: envs(&map, "OPENROUTER_API_KEY", ""),
            openrouter_model: envs(&map, "OPENROUTER_MODEL", "x-ai/grok-4.3"),
            openrouter_vision_model: envs(&map, "OPENROUTER_VISION_MODEL", "x-ai/grok-4.3"),
            openrouter_img_model: envs(&map, "OPENROUTER_IMG_MODEL", "black-forest-labs/flux.2-pro"),
            agnes_base_url: envs(&map, "AGNES_BASE_URL", "https://apihub.agnes-ai.com/v1"),
            agnes_api_key: envs(&map, "AGNES_API_KEY", ""),
            img_model: envs(&map, "IMG_MODEL", "step-2x-large"),
            img_size: envs(&map, "IMG_SIZE", "256x256"),
            img_style_prefix: envs(
                &map,
                "IMG_STYLE_PREFIX",
                "国风水墨插画风格，宣纸肌理，东方美学",
            ),
            summary_base_url: envs(&map, "SUMMARY_BASE_URL", "https://api.stepfun.com/v1"),
            summary_model: envs(&map, "SUMMARY_MODEL", "step-1-8k"),
            summary_api_key: envs(&map, "SUMMARY_API_KEY", &envs(&map, "STEP_API_KEY", "")),
            local_draw_base_url: envs(&map, "LOCAL_DRAW_BASE_URL", ""),
            local_draw_api_key: envs(&map, "LOCAL_DRAW_API_KEY", ""),
            local_draw_model: envs(&map, "LOCAL_DRAW_MODEL", "boogu"),
            bot_token: envs(&map, "BOT_TOKEN", ""),
            max_history: envs(&map, "MAX_HISTORY", "40").parse().unwrap_or(40),
            llm_max_tokens: envs(&map, "LLM_MAX_TOKENS", "1000").parse().unwrap_or(1000),
            llm_temperature: envs(&map, "LLM_TEMPERATURE", "0.95").parse().unwrap_or(0.95),
            llm_top_p: envs(&map, "LLM_TOP_P", "0.93").parse().unwrap_or(0.93),
            llm_repeat_penalty: envs(&map, "LLM_REPEAT_PENALTY", "1.08").parse().unwrap_or(1.08),
            llm_min_tokens: envs(&map, "LLM_MIN_TOKENS", "0").parse().unwrap_or(0),
            ffmpeg_bin: envs(&map, "FFMPEG_BIN", "ffmpeg"),
            grok_tts_chunk_chars: envs(&map, "GROK_TTS_CHUNK_CHARS", "420").parse().unwrap_or(420).max(120),
            local_qwen_context_tokens: envs(&map, "LOCAL_QWEN_CONTEXT_TOKENS", "8192").parse().unwrap_or(8192),
            local_qwen_context_margin_tokens: envs(&map, "LOCAL_QWEN_CONTEXT_MARGIN_TOKENS", "384").parse().unwrap_or(384),
            local_qwen_prompt_budget_tokens: envs(&map, "LOCAL_QWEN_PROMPT_BUDGET_TOKENS", "0").parse().unwrap_or(0),
            model_provider_env: envs(&map, "MODEL_PROVIDER", "local"),
            vb_host: envs(&map, "VB_HOST", "127.0.0.1"),
            vb_port: envs(&map, "VB_PORT", "8788").parse().unwrap_or(8788),
            base_dir,
        }
    }
}


// ── state files ────────────────────────────────────────────────────────────

fn read_json(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok())
}

fn write_json(path: &Path, v: &Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(v) {
        if let Err(e) = std::fs::write(path, text) {
            log_line(&format!("保存 {} 失败: {e}", path.display()));
        }
    }
}

pub fn log_line(msg: &str) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, mo, d, h, mi, s) = civil_from_unix(secs as i64);
    println!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} INFO {msg}");
}

pub fn log_warn(msg: &str) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, mo, d, h, mi, s) = civil_from_unix(secs as i64);
    println!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} WARNING {msg}");
}

fn civil_from_unix(t: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = t.div_euclid(86400);
    let sod = t.rem_euclid(86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, (sod / 3600) as u32, ((sod % 3600) / 60) as u32, (sod % 60) as u32)
}

pub static CFG: std::sync::LazyLock<Cfg> = std::sync::LazyLock::new(Cfg::load);
pub fn cfg() -> &'static Cfg {
    &CFG
}

fn option(
    id: &str, label: &str, desc: &str,
    fields: &[(&str, Value)],
) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(id));
    m.insert("label".into(), json!(label));
    m.insert("desc".into(), json!(desc));
    for (k, v) in fields {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

pub fn runtime_catalog() -> serde_json::Map<String, Value> {
    let c = cfg();
    let mut cat = serde_json::Map::new();

    cat.insert("text".into(), json!([
        option("local-qwen", "本地 Qwen", "qwen.ajw.cn 本地大脑", &[
            ("base_url", json!(c.qwen_base_url)), ("api_key", json!(c.qwen_api_key)),
            ("model", json!(c.qwen_model)), ("provider", json!("openai_chat")),
        ]),
        option("openrouter-grok-4.3", "Grok 4.3", "OpenRouter，支持看图/文件", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("x-ai/grok-4.3")), ("provider", json!("openrouter_chat")),
            ("vision_model", json!("x-ai/grok-4.3")),
        ]),
    ]));

    cat.insert("tts".into(), json!([
        option("local-cv3", "本地 CV3", "Win11 CosyVoice3 语音代理", &[
            ("base_url", json!(c.tts_base)), ("api_key", json!(c.tts_api_key)),
            ("model", json!(c.tts_model)), ("provider", json!("openai_speech")),
            ("voice", json!(c.tts_voice)), ("instruction", json!(c.tts_instruction)),
            ("response_format", json!("wav")), ("speed", json!(c.tts_speed)),
        ]),
        option("openrouter-grok-voice-tts-1.0", "Grok Voice TTS", "真人语气更好，OpenRouter", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("x-ai/grok-voice-tts-1.0")), ("provider", json!("openai_speech")),
            ("voice", json!("eve")),
            ("instruction", json!("女性独白和对白默认采用低柔、妩媚、妖娆、娇柔的真人语气；语速略慢，带轻微气息感和笑意，尾音柔软，停顿自然，情绪随文本起伏，避免机械平铺。")),
            ("response_format", json!("mp3")),
        ]),
        option("openrouter-kokoro-82m", "Kokoro 82M", "便宜快速，中文可用", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("hexgrad/kokoro-82m")), ("provider", json!("openai_speech")),
            ("voice", json!("zf_xiaoxiao")), ("response_format", json!("mp3")),
        ]),
        option("step-stepaudio-2.5-tts", "StepAudio 2.5 TTS", "阶跃官方 TTS 旧入口", &[
            ("base_url", json!(c.step_base)), ("api_key", json!(c.step_api_key)),
            ("model", json!("stepaudio-2.5-tts")), ("provider", json!("openai_speech")),
            ("voice", json!(c.tts_voice)), ("instruction", json!(c.tts_instruction)),
            ("response_format", json!(c.tts_response_format)), ("speed", json!(c.tts_speed)),
        ]),
    ]));

    cat.insert("asr".into(), json!([
        option("disabled", "未配置 ASR", "语音输入暂不可用，需要重新配置 ASR 模型", &[
            ("provider", json!("disabled")),
        ]),
        option("local-sensevoice", "本地 SenseVoice", "Win11/qwen.ajw.cn 本地语音识别", &[
            ("base_url", json!(c.asr_base)), ("api_key", json!(c.asr_api_key)),
            ("model", json!(c.asr_model)), ("transport", json!("multipart")),
        ]),
        option("openrouter-qwen3-asr-flash", "Qwen3 ASR Flash", "中文转写推荐，OpenRouter", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("qwen/qwen3-asr-flash-2026-02-10")), ("transport", json!("openrouter_json")),
        ]),
        option("openrouter-voxtral-mini-transcribe", "Voxtral Mini Transcribe", "Mistral 转写，中文可用", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("mistralai/voxtral-mini-transcribe")), ("transport", json!("openrouter_json")),
        ]),
    ]));

    cat.insert("image".into(), json!([
        option("local-step-image", "本地 Step 生图", "阶跃 Step /images/generations", &[
            ("base_url", json!(c.step_base)), ("api_key", json!(c.step_api_key)),
            ("model", json!(c.img_model)), ("provider", json!("images_generations")),
        ]),
        option("local-boogu", "本地 Boogu (CachyOS)", "CachyOS RTX3060 Boogu FLOW 模型，~40s/张", &[
            ("base_url", json!(c.local_draw_base_url)), ("api_key", json!(c.local_draw_api_key)),
            ("model", json!(c.local_draw_model)), ("provider", json!("local_boogu")),
        ]),
        option("openrouter-flux-2-pro", "FLUX.2 Pro", "当前 OpenRouter 生图质量优先", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("black-forest-labs/flux.2-pro")), ("provider", json!("openrouter_images")),
        ]),
        option("openrouter-flux-2-flex", "FLUX.2 Flex", "OpenRouter 生图备选", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("black-forest-labs/flux.2-flex")), ("provider", json!("openrouter_images")),
        ]),
        option("openrouter-grok-imagine", "Grok Imagine", "OpenRouter 旧生图入口", &[
            ("base_url", json!(c.openrouter_base_url)), ("api_key", json!(c.openrouter_api_key)),
            ("model", json!("x-ai/grok-imagine-image-quality")), ("provider", json!("openrouter_chat_image")),
        ]),
        option("agnes-image-2.1-flash", "Agnes Image 2.1 Flash", "Agnes 文生图", &[
            ("base_url", json!(c.agnes_base_url)), ("api_key", json!(c.agnes_api_key)),
            ("model", json!("agnes-image-2.1-flash")), ("provider", json!("images_generations")),
        ]),
    ]));

    cat
}

pub fn public_catalog() -> Value {
    let cat = runtime_catalog();
    let mut public = Map::new();
    for (category, options) in cat.iter() {
        let list = options.as_array().cloned().unwrap_or_default();
        let cleaned: Vec<Value> = list
            .iter()
            .map(|opt| {
                let mut o = opt.as_object().cloned().unwrap_or_default();
                o.remove("api_key");
                o.remove("base_url");
                Value::Object(
                    o.into_iter().filter(|(_, v)| !v.is_null() && v.as_str() != Some("")).collect(),
                )
            })
            .collect();
        public.insert(category.clone(), json!(cleaned));
    }
    Value::Object(public)
}

pub fn find_option(category: &str, id: &str) -> Option<Value> {
    let cat = runtime_catalog();
    cat.get(category)?
        .as_array()?
        .iter()
        .find(|o| o.get("id").and_then(Value::as_str) == Some(id))
        .cloned()
}

fn stack_file() -> PathBuf { cfg().base_dir.join("model_stack_state.json") }
fn provider_file() -> PathBuf { cfg().base_dir.join("provider_state.json") }
fn personas_file() -> PathBuf { cfg().base_dir.join("personas.json") }
fn bot_state_file() -> PathBuf { cfg().base_dir.join("bot_state.json") }

fn default_stack_for(provider: &str) -> Value {
    if provider == "cloud" {
        json!({"text": "openrouter-grok-4.3", "tts": "local-cv3", "asr": "local-sensevoice", "image": "openrouter-flux-2-pro"})
    } else {
        json!({"text": "local-qwen", "tts": "local-cv3", "asr": "local-sensevoice", "image": "local-boogu"})
    }
}

pub fn get_model_provider() -> String {
    let state = read_json(&provider_file()).unwrap_or(Value::Null);
    state
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or(&cfg().model_provider_env)
        .to_string()
}

fn set_model_provider_state(provider: &str) {
    write_json(&provider_file(), &json!({"provider": provider}));
}

pub fn get_model_stack() -> Value {
    let mut stack = default_stack_for(&get_model_provider());
    if let Some(Value::Object(raw)) = read_json(&stack_file()) {
        if let Value::Object(s) = &mut stack {
            for (k, v) in raw {
                if s.contains_key(&k) {
                    s.insert(k, v);
                }
            }
        }
    }
    let defaults = default_stack_for(&get_model_provider());
    if let (Value::Object(s), Value::Object(d)) = (&mut stack, &defaults) {
        for (cat, fallback) in d {
            let cur = s.get(cat).and_then(Value::as_str).unwrap_or("");
            if find_option(cat, cur).is_none() {
                s.insert(cat.clone(), fallback.clone());
            }
        }
    }
    stack
}

/// Err(msg) → HTTP 400 detail (python ValueError parity)
pub fn set_model_stack(stack: &Value) -> Result<Value, String> {
    let mut current = get_model_stack();
    if let (Value::Object(incoming), Value::Object(cur)) = (stack, &mut current) {
        for cat in ["text", "tts", "asr", "image"] {
            if let Some(v) = incoming.get(cat) {
                let value = match v.as_str() {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                if find_option(cat, &value).is_none() {
                    return Err(format!("{cat} 模型不存在: {value}"));
                }
                cur.insert(cat.into(), json!(value));
            }
        }
    }
    write_json(&stack_file(), &current);
    Ok(current)
}

pub fn set_model_provider(provider: &str) -> Result<(), String> {
    if provider != "local" && provider != "cloud" {
        return Err(format!("无效的 provider: {provider}，只支持 local/cloud"));
    }
    set_model_provider_state(provider);
    let preset = if provider == "local" {
        json!({"text": "local-qwen", "tts": "local-cv3", "asr": "local-sensevoice", "image": "local-boogu"})
    } else {
        json!({"text": "openrouter-grok-4.3", "tts": "openrouter-grok-voice-tts-1.0", "asr": "openrouter-qwen3-asr-flash", "image": "openrouter-flux-2-pro"})
    };
    let _ = set_model_stack(&preset);
    Ok(())
}

pub fn category_config(category: &str, fallback_id: &str) -> Value {
    let stack = get_model_stack();
    let id = stack.get(category).and_then(Value::as_str).unwrap_or("");
    find_option(category, id).or_else(|| find_option(category, fallback_id)).unwrap_or(Value::Null)
}

pub fn get_llm_config() -> Value { category_config("text", "local-qwen") }
pub fn get_tts_config() -> Value { category_config("tts", "local-cv3") }
pub fn get_asr_config() -> Value { category_config("asr", "local-sensevoice") }
pub fn get_img_config() -> Value { category_config("image", "local-step-image") }

pub fn get_summary_config() -> Value {
    let llm = get_llm_config();
    let base = llm.get("base_url").and_then(Value::as_str).unwrap_or("");
    let key = llm.get("api_key").and_then(Value::as_str).unwrap_or("");
    if !base.is_empty() && !key.is_empty() {
        return llm;
    }
    let c = cfg();
    json!({"base_url": c.summary_base_url, "api_key": c.summary_api_key, "model": c.summary_model, "provider": "openai_chat"})
}

// ── personas ───────────────────────────────────────────────────────────────

fn default_personas() -> Value {
    json!([
        {"id": "default", "name": "默认助手", "temperature": 1.0, "voice": "",
         "prompt": "你是一个友好的助手。回答围绕用户意图，信息清楚，语气自然，适合语音播报。不要用 markdown 或列表。"},
        {"id": "imaginative", "name": "热情有想象力", "temperature": 1.2, "voice": "",
         "prompt": "你是一个热情洋溢、充满想象力的伙伴。回答生动活泼，多用比喻和画面感，带着好奇心和感染力。可以适度发散。口语化，适合语音播报。不用 markdown。"},
        {"id": "serious", "name": "认真严谨", "temperature": 0.5, "voice": "",
         "prompt": "你是一个认真严谨的助手。回答准确、有条理，不确定时如实说明。语气沉稳克制，不夸张不跑题。口语化，适合语音播报。不用 markdown。"},
        {"id": "gentle", "name": "温柔治愈", "temperature": 1.0, "voice": "",
         "prompt": "你是一个温柔、善解人意的倾听者。语气轻柔体贴，像朋友一样关心对方，回答带安慰和鼓励。口语化，适合语音播报。不用 markdown。"},
        {"id": "sarcastic", "name": "毒舌幽默", "temperature": 1.1, "voice": "",
         "prompt": "你是一个嘴毒心软、爱吐槽的朋友。回答带点黑色幽默和机智的反讽，但不过分刻薄，底色是善意。口语化，适合语音播报。不用 markdown。"},
        {"id": "concise", "name": "利落干练", "temperature": 0.6, "voice": "",
         "prompt": "你是一个利落干练的助手。回答抓住关键，语气清楚，不废话不寒暄。口语化，适合语音播报。不用 markdown。"},
        {"id": "chuuni", "name": "中二热血", "temperature": 1.3, "voice": "",
         "prompt": "你是一个中二又热血的伙伴。说话带着动漫角色的夸张感，动不动就要燃起来，把日常小事说成史诗冒险。保留热血节奏。口语化，适合语音播报。不用 markdown。"},
    ])
}

pub fn get_personas() -> Vec<Value> {
    match read_json(&personas_file()) {
        Some(Value::Array(a)) => a,
        _ => {
            let d = default_personas();
            write_json(&personas_file(), &d);
            d.as_array().cloned().unwrap_or_default()
        }
    }
}

pub fn set_personas(personas: &[Value]) {
    write_json(&personas_file(), &json!(personas));
}

pub fn get_active_persona() -> Value {
    let personas = get_personas();
    let state = read_json(&bot_state_file()).unwrap_or(Value::Null);
    let active_id = state.get("active_persona_id").and_then(Value::as_str);
    for p in &personas {
        if p.get("id").and_then(Value::as_str) == active_id {
            return p.clone();
        }
    }
    personas
        .first()
        .cloned()
        .unwrap_or_else(|| default_personas().as_array().unwrap()[0].clone())
}

pub fn set_active_persona(pid: &str) -> Option<Value> {
    let personas = get_personas();
    for p in &personas {
        if p.get("id").and_then(Value::as_str) == Some(pid) {
            let mut state = read_json(&bot_state_file()).unwrap_or(json!({}));
            if let Value::Object(o) = &mut state {
                o.insert("active_persona_id".into(), json!(pid));
            }
            write_json(&bot_state_file(), &state);
            return Some(p.clone());
        }
    }
    None
}

/// Deterministic stand-in for python's `abs(hash(name)) % 100000`.
pub fn persona_hash(name: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h % 100_000
}

pub static STATE_LOCK: Mutex<()> = Mutex::new(());

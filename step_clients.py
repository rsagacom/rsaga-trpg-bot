"""三个外部服务的客户端封装：ASR / LLM / TTS。

- ASR: StepAudio 2.5 ASR, OpenAI Whisper 兼容 multipart
- LLM: 本地 Qwen, OpenAI 兼容 chat completions
- TTS: StepAudio 2.5 TTS, 返回 mp3 二进制

ASR 端点未实测，按 Whisper 兼容格式实现；失败时抛带状态码+响应体的明确错误。
"""
import json
import subprocess
import tempfile
from pathlib import Path

import httpx

import config


class StepError(RuntimeError):
    """携带状态码与响应体的明确错误，便于上线后调端点。"""

    def __init__(self, what: str, status: int = 0, body: str = ""):
        self.what = what
        self.status = status
        self.body = body
        msg = f"{what} [status={status}] body={body[:500]}"
        super().__init__(msg)


# ── ASR ──────────────────────────────────────────────────────────────
def _to_wav(audio_bytes: bytes, mime: str) -> bytes:
    """任意录音格式 → 16k 单声道 wav。ASR 对 wav 兼容性最稳。"""
    src = tempfile.NamedTemporaryFile(delete=False, suffix=_ext_for_mime(mime))
    src.write(audio_bytes)
    src.close()
    dst = tempfile.NamedTemporaryFile(delete=False, suffix=".wav")
    dst.close()
    try:
        r = subprocess.run(
            [config.FFMPEG_BIN, "-y", "-i", src.name, "-ar", "16000", "-ac", "1", dst.name],
            capture_output=True, timeout=60,
        )
        if r.returncode != 0:
            raise StepError("ffmpeg 转码失败", 0, r.stderr.decode("utf-8", "ignore"))
        return Path(dst.name).read_bytes()
    finally:
        for p in (src.name, dst.name):
            try:
                Path(p).unlink()
            except Exception:
                pass


def _ext_for_mime(mime: str) -> str:
    m = (mime or "").lower()
    if "webm" in m:
        return ".webm"
    if "ogg" in m:
        return ".ogg"
    if "mp4" in m or "m4a" in m:
        return ".m4a"
    if "mp3" in m:
        return ".mp3"
    if "wav" in m:
        return ".wav"
    return ".bin"


def asr(audio_bytes: bytes, mime: str = "audio/webm") -> str:
    """语音 → 文字。返回转写文本。"""
    wav = _to_wav(audio_bytes, mime)
    headers = {"Authorization": f"Bearer {config.STEP_API_KEY}"}
    files = {
        "file": ("audio.wav", wav, "audio/wav"),
        "model": (None, config.ASR_MODEL),
    }
    try:
        r = httpx.post(
            f"{config.STEP_BASE}/audio/transcriptions",
            headers=headers, files=files, timeout=120.0,
        )
    except Exception as e:
        raise StepError(f"ASR 请求异常: {e}")
    if r.status_code != 200:
        raise StepError("ASR HTTP 失败", r.status_code, r.text)
    try:
        data = r.json()
    except Exception:
        # 有的实现直接返回纯文本
        return r.text.strip()
    # 兼容 {"text": "..."} 与 {"result": {"text": "..."}}
    text = data.get("text")
    if text is None and isinstance(data.get("result"), dict):
        text = data["result"].get("text")
    if not text:
        raise StepError("ASR 返回无 text 字段", r.status_code, r.text)
    return text.strip()


# ── LLM ──────────────────────────────────────────────────────────────
def llm(history: list[dict]) -> str:
    """history: [{role, content}, ...]，不含 system。返回回复文本。
    用当前激活角色的 prompt 和 temperature。
    """
    persona = config.get_active_persona()
    messages = [{"role": "system", "content": persona["prompt"]}] + history
    payload = {
        "model": config.QWEN_MODEL,
        "messages": messages,
        "stream": False,
        "temperature": persona.get("temperature", 1.0),
        "top_p": 0.9,
        "max_tokens": config.LLM_MAX_TOKENS,
    }
    headers = {
        "Authorization": f"Bearer {config.QWEN_API_KEY}",
        "Content-Type": "application/json",
    }
    try:
        r = httpx.post(
            f"{config.QWEN_BASE_URL}/chat/completions",
            headers=headers, json=payload, timeout=180.0,
        )
    except Exception as e:
        raise StepError(f"LLM 请求异常: {e}")
    if r.status_code != 200:
        raise StepError("LLM HTTP 失败", r.status_code, r.text)
    try:
        return r.json()["choices"][0]["message"]["content"].strip()
    except Exception:
        raise StepError("LLM 返回结构异常", r.status_code, r.text)


def llm_stream(history: list[dict], image_data_url: str | None = None):
    """流式生成，yield 每个 token 文本片段。用当前激活角色的 prompt+temperature。
    image_data_url: 若提供（data:image/...;base64,...），把最后一条 user 消息
    改成多模态格式 [image, text] 发给 Qwen。
    """
    import json as _json
    persona = config.get_active_persona()
    messages = [{"role": "system", "content": persona["prompt"]}] + [dict(m) for m in history]
    # 若带图，把最后一条 user 消息转多模态
    if image_data_url and messages and messages[-1]["role"] == "user":
        text_part = messages[-1].get("content", "") or "看图回答"
        messages[-1]["content"] = [
            {"type": "image_url", "image_url": {"url": image_data_url}},
            {"type": "text", "text": text_part},
        ]
    payload = {
        "model": config.QWEN_MODEL,
        "messages": messages,
        "stream": True,
        "temperature": persona.get("temperature", 1.0),
        "top_p": 0.9,
        "max_tokens": config.LLM_MAX_TOKENS,
    }
    headers = {
        "Authorization": f"Bearer {config.QWEN_API_KEY}",
        "Content-Type": "application/json",
    }
    try:
        with httpx.stream(
            "POST", f"{config.QWEN_BASE_URL}/chat/completions",
            headers=headers, json=payload, timeout=300.0,
        ) as r:
            if r.status_code != 200:
                body = r.read().decode("utf-8", "ignore")
                raise StepError("LLM HTTP 失败", r.status_code, body)
            for line in r.iter_lines():
                if not line:
                    continue
                if line.startswith("data:"):
                    line = line[5:].strip()
                if line == "[DONE]":
                    break
                try:
                    chunk = _json.loads(line)
                except Exception:
                    continue
                choices = chunk.get("choices") or []
                if not choices:
                    continue
                delta = choices[0].get("delta") or {}
                piece = delta.get("content")
                if piece:
                    yield piece
    except StepError:
        raise
    except Exception as e:
        raise StepError(f"LLM 流式异常: {e}")


def split_sentences(text: str) -> list[str]:
    """把累积文本切成"完整句"和"剩余"。按句末标点切（不按换行切，保留 \\n）。
    返回 (可发送的完整句列表, 剩余未完成片段)。
    """
    import re
    # 只按句末标点切，保留标点和换行
    parts = re.split(r"(?<=[。！？!?…])", text)
    if len(parts) <= 1:
        return [], text
    complete = parts[:-1]  # 最后一段可能未完
    remainder = parts[-1]
    return [p for p in complete if p.strip()], remainder


# ── TTS ──────────────────────────────────────────────────────────────
def tts(text: str, voice: str = "") -> bytes:
    """文字 → mp3 bytes。阶跃 TTS 限 10 RPM，429 时递增重试。
    voice: 指定音色 ID（如角色的音色），空则用全局 TTS_VOICE。
    """
    payload = {
        "model": config.TTS_MODEL,
        "input": text,
        "voice": voice or config.TTS_VOICE,
        "instruction": config.TTS_INSTRUCTION,
    }
    headers = {
        "Authorization": f"Bearer {config.STEP_API_KEY}",
        "Content-Type": "application/json",
    }
    import time as _time
    for attempt in range(5):
        try:
            r = httpx.post(
                f"{config.STEP_BASE}/audio/speech",
                headers=headers, json=payload, timeout=120.0,
            )
        except Exception as e:
            raise StepError(f"TTS 请求异常: {e}")
        if r.status_code == 429:
            # 限流：等递增时间后重试（重试间隙释放线程给其他请求）
            _time.sleep(min(6 * (attempt + 1), 30))
            continue
        if r.status_code != 200:
            raise StepError("TTS HTTP 失败", r.status_code, r.text)
        return r.content
    raise StepError("TTS 多次限流重试仍失败", 429, "rate limited")


# ── 背景图生成（阶跃文生图，黑白漫画）─────────────────────────────────
def summarize_for_image(story_text: str, persona: str = "") -> str:
    """用 step-3.7-flash 把故事/对话内容总结成一句绘图画面描述。
    返回纯画面描述（人物、动作、场景、氛围），便于阶跃绘图。
    """
    sys = "你是东亚漫画素描画师。把给定的故事或对话提炼成一句适合画画的视觉场景描述，"
    "着重刻画画面中女性的美丽容颜与精致五官、细腻表情与情绪神态，通过人物面部细节烘托剧情氛围与情感张力，"
    "同时兼顾动作、场景、光影。不超过50字，只描写画面可见元素，不要任何解释、前缀或引号，不要描述文字、标题、对白、字幕。"
    user = f"角色：{persona}\n内容：{story_text}\n画面描述："
    api_key = config.SUMMARY_API_KEY or config.STEP_API_KEY
    payload = {
        "model": config.SUMMARY_MODEL,
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": user},
        ],
        "stream": False,
        "temperature": 0.4,
        "max_tokens": 80,
    }
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json",
    }
    try:
        r = httpx.post(
            f"{config.SUMMARY_BASE_URL}/chat/completions",
            headers=headers, json=payload, timeout=30.0,
        )
    except Exception as e:
        raise StepError(f"总结请求异常: {e}")
    if r.status_code != 200:
        raise StepError("总结 HTTP 失败", r.status_code, r.text)
    try:
        return r.json()["choices"][0]["message"]["content"].strip()
    except Exception:
        raise StepError("总结返回异常", r.status_code, r.text)


def gen_bg_image(prompt: str) -> str:
    """根据 prompt 生成黑白漫画背景图，返回 base64 编码的 PNG（不含 data: 前缀）。
    用阶跃 /v1/images/generations，800x1280。
    """
    import base64 as _b64
    full_prompt = f"{config.IMG_STYLE_PREFIX}，{prompt}"
    payload = {
        "model": config.IMG_MODEL,
        "prompt": full_prompt,
        "size": config.IMG_SIZE,
        "n": 1,
    }
    headers = {
        "Authorization": f"Bearer {config.STEP_API_KEY}",
        "Content-Type": "application/json",
    }
    try:
        r = httpx.post(
            f"{config.STEP_BASE}/images/generations",
            headers=headers, json=payload, timeout=180.0,
        )
    except Exception as e:
        raise StepError(f"绘图请求异常: {e}")
    if r.status_code != 200:
        raise StepError("绘图 HTTP 失败", r.status_code, r.text)
    data = r.json().get("data", [])
    if not data:
        raise StepError("绘图返回无图", r.status_code, r.text)
    item = data[0]
    # 优先 b64_json，否则下载 url
    b64 = item.get("b64_json")
    if b64:
        return b64
    url = item.get("url")
    if not url:
        raise StepError("绘图返回无 url/b64", r.status_code, r.text)
    try:
        img = httpx.get(url, timeout=60.0).content
    except Exception as e:
        raise StepError(f"下载图片异常: {e}")
    return _b64.b64encode(img).decode("ascii")

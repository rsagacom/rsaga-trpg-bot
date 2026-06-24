"""FastAPI 后端：单 H5 页面 + /api/chat 端点。

会话历史持久化到 sessions.json，重启不清空。
鉴权：单用户静态 token（X-Bot-Token 头），无 token 时开放。
"""
import base64
import json
import logging
import re
from collections import deque
from pathlib import Path

from fastapi import FastAPI, File, Form, Header, HTTPException, Request
from fastapi.responses import FileResponse, JSONResponse, StreamingResponse
from fastapi.staticfiles import StaticFiles

import config
import step_clients
from step_clients import StepError

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("voicebot")

app = FastAPI()

# ── 会话记忆（持久化）────────────────────────────────────────────────
_sessions: dict[str, list[dict]] = {}


def _load_sessions() -> None:
    global _sessions
    try:
        if config.SESSIONS_FILE.exists():
            _sessions = json.loads(config.SESSIONS_FILE.read_text("utf-8"))
            log.info("载入 %d 个会话", len(_sessions))
    except Exception as e:
        log.warning("载入会话失败，从空开始: %s", e)
        _sessions = {}


def _save_sessions() -> None:
    try:
        config.SESSIONS_FILE.write_text(
            json.dumps(_sessions, ensure_ascii=False), encoding="utf-8"
        )
    except Exception as e:
        log.warning("保存会话失败: %s", e)


_load_sessions()


def _history(session_id: str) -> list[dict]:
    if session_id not in _sessions:
        _sessions[session_id] = []
    return _sessions[session_id]


def _trim(session_id: str) -> None:
    h = _sessions[session_id]
    if len(h) > config.MAX_HISTORY:
        _sessions[session_id] = h[-config.MAX_HISTORY:]


# ── 鉴权 ─────────────────────────────────────────────────────────────
def _check_token(x_bot_token: str | None) -> None:
    """配置了 BOT_TOKEN 才校验；没配则完全开放。"""
    if not config.BOT_TOKEN:
        return
    if x_bot_token != config.BOT_TOKEN:
        raise HTTPException(status_code=401, detail="token 无效")


# ── 工具 ─────────────────────────────────────────────────────────────
def _clean(s: str) -> str:
    """去 markdown/多余空白，适配语音播报。"""
    s = re.sub(r"```.*?```", "", s, flags=re.DOTALL)
    s = re.sub(r"[#*`_>]", "", s)
    return s.strip()


# ── 路由 ─────────────────────────────────────────────────────────────
@app.get("/api/health")
def health(x_bot_token: str | None = Header(default=None, alias="X-Bot-Token")):
    # 前端用此端点探测是否需要鉴权门：配了 BOT_TOKEN 且 token 不符 → 401
    _check_token(x_bot_token)
    return {"ok": True, "asr_model": config.ASR_MODEL, "tts_model": config.TTS_MODEL}


@app.get("/api/history")
def history(session_id: str, x_bot_token: str | None = Header(default=None, alias="X-Bot-Token")):
    """返回某会话的历史消息，供前端刷新后回显。"""
    _check_token(x_bot_token)
    return {"messages": _history(session_id)}


# ── 背景图生成（并行，不阻塞主对话）────────────────────────────────
from starlette.concurrency import run_in_threadpool


@app.post("/api/bg/gen")
async def gen_bg(
    request: Request,
    x_bot_token: str | None = Header(default=None, alias="X-Bot-Token"),
):
    """根据故事文本生成黑白漫画背景图。返回 base64 PNG。
    流程：MiniCPM-V 把故事总结成画面描述 → 阶跃文生图。
    前端在 Qwen 回复完(done)后并行 fire，不阻塞对话。总耗时 ~26s。
    body: {text(故事/回复内容), persona?}
    """
    _check_token(x_bot_token)
    body = await request.json()
    text = (body.get("text") or "").strip()
    persona = (body.get("persona") or "").strip()
    if not text:
        raise HTTPException(status_code=400, detail="无 text")
    # 1. MiniCPM 总结成画面描述
    try:
        scene = await run_in_threadpool(step_clients.summarize_for_image, text, persona)
        log.info("MiniCPM 画面描述: %r", scene[:80])
    except StepError as e:
        log.warning("MiniCPM 总结失败，退回原文: %s", e)
        scene = text[:60]  # 退回用原文截断
    # 2. 阶跃绘图
    try:
        b64 = await run_in_threadpool(step_clients.gen_bg_image, scene)
    except StepError as e:
        log.error("绘图失败: %s", e)
        return JSONResponse({"error": "bg", "detail": e.body[:300] or e.what}, status_code=502)
    return {"image": b64, "scene": scene}


@app.post("/api/history/clear")
def clear_history(session_id: str, x_bot_token: str | None = Header(default=None, alias="X-Bot-Token")):
    """清空指定会话的上下文历史。session_id 作 query param 传。"""
    _check_token(x_bot_token)
    if session_id in _sessions:
        del _sessions[session_id]
    _save_sessions()
    log.info("已清空会话 %s", session_id)
    return {"ok": True}


@app.post("/api/history/truncate")
async def truncate_history(
    request: Request,
    x_bot_token: str | None = Header(default=None, alias="X-Bot-Token"),
):
    """截断历史：保留前 keep_count 条，删除其后所有。
    用于"编辑某条重发"——前端编辑第 N 条后，truncate keep_count=N-1，
    再用新文本走正常 stream。
    body: {session_id, keep_count}
    """
    _check_token(x_bot_token)
    body = await request.json()
    sid = body.get("session_id", "default")
    keep = int(body.get("keep_count", 0))
    if sid in _sessions and keep >= 0:
        _sessions[sid] = _sessions[sid][:keep]
        _save_sessions()
    log.info("截断会话 %s 保留前 %d 条", sid, keep)
    return {"ok": True, "kept": len(_sessions.get(sid, []))}


# ── 角色库管理（多角色：名字+提示词+温度）────────────────────────────
@app.get("/api/personas")
def list_personas(x_bot_token: str | None = Header(default=None, alias="X-Bot-Token")):
    _check_token(x_bot_token)
    personas = config.get_personas()
    active = config.get_active_persona()
    return {"personas": personas, "active_id": active["id"],
            "voice_options": config.VOICE_OPTIONS}


@app.post("/api/personas")
async def save_personas_api(
    request: Request,
    x_bot_token: str | None = Header(default=None, alias="X-Bot-Token"),
):
    """整体替换角色库（前端增删改后整体回传）。"""
    _check_token(x_bot_token)
    body = await request.json()
    personas = body.get("personas", [])
    # 基本校验 + 补 id
    for p in personas:
        if "id" not in p or not p["id"]:
            p["id"] = "p" + str(abs(hash(p.get("name", ""))) % 100000)
        p["temperature"] = float(p.get("temperature", 1.0))
        p["prompt"] = (p.get("prompt") or "").strip()
        p["name"] = (p.get("name") or "未命名").strip()
        p["voice"] = (p.get("voice") or "").strip()
    config.set_personas(personas)
    log.info("角色库已更新，共 %d 个", len(personas))
    return {"ok": True, "personas": personas}


@app.post("/api/personas/active")
async def set_active_api(
    request: Request,
    x_bot_token: str | None = Header(default=None, alias="X-Bot-Token"),
):
    """切换当前激活角色。"""
    _check_token(x_bot_token)
    body = await request.json()
    pid = body.get("id")
    p = config.set_active_persona(pid)
    if not p:
        raise HTTPException(status_code=404, detail="角色不存在")
    log.info("切换激活角色 → %s (%s)", p["name"], p["id"])
    return {"ok": True, "active": p}


@app.post("/api/chat")
async def chat(
    request: Request,
    text: str | None = Form(default=None),
    audio: bytes | None = File(default=None),
    session_id: str = Form(default="default"),
    x_bot_token: str | None = Header(default=None, alias="X-Bot-Token"),
):
    _check_token(x_bot_token)

    user_text = (text or "").strip()
    if audio and not user_text:
        mime = "audio/webm"
        ct = None
        try:
            ct = request.headers.get("content-type", "")
        except Exception:
            pass
        # FastAPI/Starlette 把文件名后缀带进 audio 的 content_type 不稳，
        # 这里直接按 webm 走，后端 ffmpeg 兜底转 wav。
        log.info("ASR 输入 audio bytes=%d", len(audio))
        try:
            user_text = step_clients.asr(audio, mime)
        except StepError as e:
            log.error("ASR 失败: %s", e)
            return JSONResponse(
                {"error": "asr", "detail": e.body[:300] or e.what}, status_code=502
            )
        log.info("ASR → %r", user_text)

    if not user_text:
        raise HTTPException(status_code=400, detail="无输入（text/audio 至少一个）")

    hist = _history(session_id)
    hist.append({"role": "user", "content": user_text})

    try:
        reply = step_clients.llm(hist)
    except StepError as e:
        log.error("LLM 失败: %s", e)
        # LLM 失败把刚加的用户消息撤回，避免污染历史
        hist.pop()
        _save_sessions()
        return JSONResponse(
            {"error": "llm", "detail": e.body[:300] or e.what}, status_code=502
        )

    reply_clean = _clean(reply)
    hist.append({"role": "assistant", "content": reply_clean})
    _trim(session_id)
    _save_sessions()

    audio_b64 = ""
    try:
        _voice = (config.get_active_persona() or {}).get("voice", "")
        mp3 = step_clients.tts(reply_clean, _voice)
        audio_b64 = base64.b64encode(mp3).decode("ascii")
    except StepError as e:
        log.error("TTS 失败: %s", e)
        # TTS 失败不致命，文字照样返回
        return JSONResponse(
            {"user_text": user_text, "reply_text": reply_clean,
             "error": "tts", "detail": e.body[:300] or e.what}
        )

    return {"user_text": user_text, "reply_text": reply_clean, "audio": audio_b64}


@app.post("/api/chat/stream")
async def chat_stream(
    request: Request,
    text: str | None = Form(default=None),
    audio: bytes | None = File(default=None),
    image: str | None = Form(default=None),  # data URL: data:image/jpeg;base64,...
    session_id: str = Form(default="default"),
    x_bot_token: str | None = Header(default=None, alias="X-Bot-Token"),
):
    """流式对话：SSE 推送。
    事件：
      data: {"type":"user","text":...}          ASR 转写结果（语音链路）
      data: {"type":"sentence","text":..,"audio":..}  一句话文本+mp3(base64)
      data: {"type":"done","reply":..}          结束，完整回复
      data: {"type":"error","stage":..,"detail":..}
    分句流式 TTS：Qwen 边生成，按标点切句，每句独立 TTS 即时推送。
    image: 压缩后的图片 data URL（前端 Canvas 压到长边800）。仅本轮使用，不入历史。
    """
    _check_token(x_bot_token)

    user_text = (text or "").strip()
    if audio and not user_text:
        log.info("ASR 输入 audio bytes=%d", len(audio))
        try:
            user_text = step_clients.asr(audio, "audio/webm")
        except StepError as e:
            log.error("ASR 失败: %s", e)
            return JSONResponse(
                {"error": "asr", "detail": e.body[:300] or e.what}, status_code=502
            )
        log.info("ASR → %r", user_text)

    if not user_text and not image:
        raise HTTPException(status_code=400, detail="无输入（text/audio/image 至少一个）")

    # 有图时若没文字，给默认提问（明确约束，避免 Qwen 自由发挥跑偏）
    if image and not user_text:
        user_text = "请简短描述这张图片里的内容，两三句话。"

    async def event_gen():
        def sse(obj):
            return f"data: {json.dumps(obj, ensure_ascii=False)}\n\n"

        # 语音链路：先推 ASR 结果让前端显示用户气泡
        if audio:
            yield sse({"type": "user", "text": user_text})

        hist = _history(session_id)
        hist.append({"role": "user", "content": user_text})

        full_reply = []
        buf = ""
        persona_voice = (config.get_active_persona() or {}).get("voice", "")

        def flush_sentence(sent: str) -> str | None:
            """对一句文本做 TTS，返回 sse 字符串或 None。用当前角色音色。"""
            sent_clean = _clean(sent)
            if not sent_clean:
                return None
            try:
                mp3 = step_clients.tts(sent_clean, persona_voice)
                b64 = base64.b64encode(mp3).decode("ascii")
                return sse({"type": "sentence", "text": sent_clean, "audio": b64})
            except StepError as e:
                log.error("TTS 失败(单句): %s", e)
                return sse({"type": "sentence", "text": sent_clean, "audio": "",
                            "tts_error": e.body[:200] or e.what})

        try:
            for piece in step_clients.llm_stream(hist, image_data_url=image):
                buf += piece
                full_reply.append(piece)
                complete, buf = step_clients.split_sentences(buf)
                for sent in complete:
                    ev = flush_sentence(sent)
                    if ev:
                        yield ev
            # 收尾：剩余 buf 里的最后一句
            if buf.strip():
                ev = flush_sentence(buf)
                if ev:
                    yield ev
        except StepError as e:
            log.error("LLM 流式失败: %s", e)
            hist.pop()  # 撤回用户消息
            _save_sessions()
            yield sse({"type": "error", "stage": "llm", "detail": e.body[:300] or e.what})
            return

        reply_clean = _clean("".join(full_reply))
        hist.append({"role": "assistant", "content": reply_clean})
        _trim(session_id)
        _save_sessions()
        yield sse({"type": "done", "reply": reply_clean})

    return StreamingResponse(
        event_gen(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "X-Accel-Buffering": "no",  # 防 nginx 缓冲（CF 透传）
        },
    )


# ── 静态前端 ─────────────────────────────────────────────────────────
STATIC_DIR = Path(__file__).parent / "static"
app.mount("/static", StaticFiles(directory=STATIC_DIR), name="static")


@app.get("/")
def index():
    return FileResponse(STATIC_DIR / "index.html")

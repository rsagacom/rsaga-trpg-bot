"""集中配置 —— 全部从环境变量读取，.env 提供。"""
import json
import os
from pathlib import Path

try:
    from dotenv import load_dotenv
    load_dotenv(Path(__file__).parent / ".env")
except Exception:
    # dotenv 不是硬依赖；没有也能靠系统环境变量
    pass

BASE_DIR = Path(__file__).parent

# ── 阶跃 StepAudio 2.5（ASR/TTS/绘图共用，均 OpenAI 兼容，可换家）──
STEP_API_KEY = os.environ.get("STEP_API_KEY", "")
STEP_BASE = os.environ.get("STEP_BASE", "https://api.stepfun.com/v1")
ASR_MODEL = os.environ.get("ASR_MODEL", "stepaudio-2.5-asr")
TTS_MODEL = os.environ.get("TTS_MODEL", "stepaudio-2.5-tts")
# 默认旁白音色（来自现成脚本 step-tts-chapter9-v8.py）
TTS_VOICE = os.environ.get("TTS_VOICE", "your-voice-id")
TTS_INSTRUCTION = os.environ.get("TTS_INSTRUCTION", "语速中等，自然亲切")

# ── 音色选项（角色管理器下拉，TTS 按角色音色朗读；空则用 TTS_VOICE）──
# 格式：名字=voice_id，逗号分隔。开源版用通用占位，本地 .env 配真实音色 ID。
VOICE_OPTIONS = [
    opt.split("=", 1)
    for opt in os.environ.get(
        "VOICE_OPTIONS",
        "默认女声=,默认男声=",
    ).split(",")
    if opt.strip()
]

# ── LLM 大脑（任意 OpenAI 兼容服务）──
QWEN_BASE_URL = os.environ.get("QWEN_BASE_URL", "https://api.deepseek.com/v1")
QWEN_API_KEY = os.environ.get("QWEN_API_KEY", "")
QWEN_MODEL = os.environ.get("QWEN_MODEL", "deepseek-chat")

# ── Bot 角色库（多角色：名字+提示词+温度，运行时可编辑，热生效）──
PERSONAS_FILE = BASE_DIR / "personas.json"
STATE_FILE = BASE_DIR / "bot_state.json"  # 存当前激活角色 id 等状态

_DEFAULT_PERSONAS = [
    {"id": "default", "name": "默认助手", "temperature": 1.0,
     "prompt": "你是一个简洁友好的助手。回答控制在两三句以内，口语化，适合语音播报。不要用 markdown 或列表。"},
    {"id": "imaginative", "name": "热情有想象力", "temperature": 1.2,
     "prompt": "你是一个热情洋溢、充满想象力的伙伴。回答生动活泼，多用比喻和画面感，带着好奇心和感染力。可以适度发散，但保持两三句以内，口语化，适合语音播报。不用 markdown。"},
    {"id": "serious", "name": "认真严谨", "temperature": 0.5,
     "prompt": "你是一个认真严谨的助手。回答准确、有条理，不确定时如实说明。语气沉稳克制，不夸张不跑题。两三句以内，口语化，适合语音播报。不用 markdown。"},
    {"id": "gentle", "name": "温柔治愈", "temperature": 1.0,
     "prompt": "你是一个温柔、善解人意的倾听者。语气轻柔体贴，像朋友一样关心对方，回答带安慰和鼓励。两三句以内，口语化，适合语音播报。不用 markdown。"},
    {"id": "sarcastic", "name": "毒舌幽默", "temperature": 1.1,
     "prompt": "你是一个嘴毒心软、爱吐槽的朋友。回答带点黑色幽默和机智的反讽，但不过分刻薄，底色是善意。两三句以内，口语化，适合语音播报。不用 markdown。"},
    {"id": "concise", "name": "简洁干练", "temperature": 0.6,
     "prompt": "你是一个极度简洁的助手。只给关键信息，不废话不寒暄，一两句说完。口语化，适合语音播报。不用 markdown。"},
    {"id": "chuuni", "name": "中二热血", "temperature": 1.3,
     "prompt": "你是一个中二又热血的伙伴。说话带着动漫角色的夸张感，动不动就要燃起来，把日常小事说成史诗冒险。两三句以内，口语化，适合语音播报。不用 markdown。"},
]


def _load_personas() -> list[dict]:
    try:
        if PERSONAS_FILE.exists():
            return json.loads(PERSONAS_FILE.read_text("utf-8"))
    except Exception:
        pass
    # 首次：写入默认角色库
    _save_personas(_DEFAULT_PERSONAS)
    return [dict(p) for p in _DEFAULT_PERSONAS]


def _save_personas(personas: list[dict]) -> None:
    try:
        PERSONAS_FILE.write_text(json.dumps(personas, ensure_ascii=False, indent=2), encoding="utf-8")
    except Exception as e:
        print(f"[config] 保存角色库失败: {e}")


def _load_state() -> dict:
    try:
        if STATE_FILE.exists():
            return json.loads(STATE_FILE.read_text("utf-8"))
    except Exception:
        pass
    return {}


def _save_state(state: dict) -> None:
    try:
        STATE_FILE.write_text(json.dumps(state, ensure_ascii=False), encoding="utf-8")
    except Exception as e:
        print(f"[config] 保存状态失败: {e}")


def get_personas() -> list[dict]:
    return _load_personas()


def set_personas(personas: list[dict]) -> None:
    _save_personas(personas)


def get_active_persona() -> dict:
    """返回当前激活角色的 {id,name,temperature,prompt}。"""
    personas = _load_personas()
    state = _load_state()
    active_id = state.get("active_persona_id")
    for p in personas:
        if p["id"] == active_id:
            return p
    # 回退到第一个
    if personas:
        return personas[0]
    return _DEFAULT_PERSONAS[0]


def set_active_persona(pid: str) -> dict | None:
    personas = _load_personas()
    for p in personas:
        if p["id"] == pid:
            _save_state({**_load_state(), "active_persona_id": pid})
            return p
    return None

# ── 鉴权（单用户，静态 token）──
BOT_TOKEN = os.environ.get("BOT_TOKEN", "")

# ── 会话持久化 ──
SESSIONS_FILE = BASE_DIR / "sessions.json"
MAX_HISTORY = int(os.environ.get("MAX_HISTORY", "40"))  # 保留最近 N 条（含双方）

# ── LLM 输出上限 ──
# 流式模式下不受 CF 100s 超时约束（SSE 持续有数据流），可放大让回复说完整。
LLM_MAX_TOKENS = int(os.environ.get("LLM_MAX_TOKENS", "1000"))

# ── ffmpeg ──
FFMPEG_BIN = os.environ.get("FFMPEG_BIN", "ffmpeg")

# ── 背景图生成（阶跃 Step 文生图，黑白漫画风格）──
IMG_MODEL = os.environ.get("IMG_MODEL", "step-2x-large")
IMG_SIZE = os.environ.get("IMG_SIZE", "256x256")  # 256 正方形，更小更快
IMG_STYLE_PREFIX = os.environ.get(
    "IMG_STYLE_PREFIX",
    "东亚漫画风黑白素描，日韩漫画与铅笔素描结合的质感，清晰有力的线条勾勒人物轮廓与表情，"
    "细腻排线与淡阴影表现明暗层次，人物比例协调富有漫画感，"
    "充满剧情张力与叙事感的构图，捕捉故事最具戏剧性的瞬间，人物互动与情绪冲突跃然纸上，"
    "电影级分镜构图主次分明，景深虚实突出主体，背景细节服务于叙事，"
    "单色灰阶过渡自然，8k画质，大师级漫画素描，"
    "纯画面，绝对无任何文字、无对白、无对话框、无字幕、无logo、无水印",
)

# ── 故事→画面描述 总结模型（用 Step step-1-8k，1.9s 最快最省）──
# 密钥用环境变量 STEP_API_KEY，不硬编码。step-3.7-flash 思考链太重 content 空、
# MiniCPM 免费 key 失效，均弃用；本地 Qwen 虽稳但慢且抢主链路，改用 Step。
SUMMARY_BASE_URL = os.environ.get("SUMMARY_BASE_URL", "https://api.stepfun.com/v1")
SUMMARY_API_KEY = os.environ.get("SUMMARY_API_KEY", STEP_API_KEY)
SUMMARY_MODEL = os.environ.get("SUMMARY_MODEL", "step-1-8k")

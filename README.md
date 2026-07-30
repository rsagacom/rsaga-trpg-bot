# Rsaga TRPG Bot 🎭

**中文** | [English](#english)

一个手机友好的 H5 多模态 AI 角色扮演 bot —— 语音、文字、图片三模交互，AI 回复带情感语音播报，并根据剧情实时生成黑白漫画背景画。专为 TRPG / 文字冒险 / 角色扮演场景设计。

A mobile-friendly H5 multimodal AI roleplay bot — voice, text, and image interaction, with emotional TTS playback and real-time black-and-white manga background art generated from the story. Built for TRPG / text adventure / roleplay scenarios.

---

## 中文

### ✨ 功能特性

**三模输入**
- 🎙️ **语音输入**：手机录音 → ASR 语音识别 → AI 回复 → TTS 情感朗读
- ⌨️ **文字输入**：多轮对话，上下文持久化（刷新/重启不丢）
- 🖼️ **图片识别**：拍照/选图发送，AI 看图回答（前端 Canvas 压缩省流量）
- 🧩 **图文混合**：可同时发图 + 提问

**角色扮演**
- 🎭 **多角色管理**：创建多个角色，各自独立人设 / 温度 / 音色 / 会话，随时切换
- 🎤 **按角色音色朗读**：男声 / 女声分别配置，不同角色不同嗓音
- ✏️ **编辑重发**：编辑任意历史消息重新生成（ChatGPT 式）
- 🗑️ **清空上下文**：一键重新开始
- 🎮 **文字游戏快捷键**：A / B / C / D / 1 / 2 / 3 / 4 / 继续 一键选项

**语音体验**
- 🔊 **流式朗读**：AI 边生成边播（攒够 ~70% 再开始播，避免开头断续）
- 🔁 **重播按钮**：每条回复可重新播放
- 💬 **情感 TTS 指令**：可配置朗读情感（如"剧中角色语气、带感情、语调起伏"）

**剧情背景画**
- 🎨 **并行生成**：AI 回复完，后台并行生成黑白漫画场景画铺底层，不阻塞对话
- 🖌️ **智能 prompt**：先用轻量 LLM 把回复浓缩成画面描述（着重人物表情与剧情张力），再绘图
- 📐 **可配置风格 / 分辨率**：素描 / 漫画 / 线条等风格，256~1024 分辨率

**其他**
- 🔒 **单用户 token 鉴权**：口令保护
- 📱 **移动端适配**：单文件 H5，无构建，Android/iOS 浏览器均可用
- 🌐 **全接口 OpenAI 兼容**：换服务商只需改 `.env`，代码零改

### 🏗️ 架构

```
手机 H5 ──文字/录音/图片──▶ FastAPI 后端 (SSE 流式)
  ├─ ASR    语音→文字   (OpenAI 兼容 /audio/transcriptions)
  ├─ LLM    对话大脑    (OpenAI 兼容 /chat/completions)   ← 可换任意家
  ├─ TTS    文字→语音   (OpenAI 兼容 /audio/speech)
  ├─ 总结   回复→画面描述 (OpenAI 兼容 /chat/completions)
  └─ 绘图   画面→背景画 (OpenAI 兼容 /images/generations)
```

### 🔌 接口兼容性

| 接口 | 默认 | 可换为 |
|------|------|--------|
| LLM 大脑 | 阶跃 step-1-8k | DeepSeek、Kimi、GLM、OpenAI、Ollama、llama.cpp 等任意 OpenAI 兼容 |
| 画面总结 | 阶跃 step-1-8k | 同上（轻量模型即可） |
| ASR | 阶跃 stepaudio-2.5-asr | OpenAI Whisper、阿里等 Whisper 兼容 |
| TTS | 阶跃 stepaudio-2.5-tts | OpenAI TTS、阿里等 OpenAI 兼容 |
| 绘图 | 阶跃 step-2x-large | OpenAI DALL·E、阿里 wan 等 OpenAI 兼容 |

> 💰 **最省 token 配置**：LLM 用 DeepSeek（缓存命中 ¥0.1/百万 token）。

### 🚀 部署

```bash
# 1. 环境
python3 -m venv venv && source venv/bin/activate
pip install -r requirements.txt
sudo apt install ffmpeg          # ASR 转码用

# 2. 配置
cp .env.example .env
# 编辑 .env 填入各服务商 key

# 3. 运行
uvicorn app:app --host 127.0.0.1 --port 8788

# 4. 常驻 (systemd)
sudo cp voicebot.service /etc/systemd/system/
sudo systemctl enable --now voicebot
```

**公网访问**：手机录音/拍照需 HTTPS。推荐用 [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/) 把 `localhost:8788` 暴露到自定义域名，免费且自动 HTTPS。

### 📱 使用

浏览器打开 `https://你的域名`，输入 `BOT_TOKEN` 口令进入。

> ⚠️ 微信内置浏览器对音频自动播放限制严，建议用系统浏览器（Safari/Chrome）打开，或"添加到主屏幕"当 App 用。

### 🛠️ 技术栈

- 后端：Python + FastAPI + httpx + SSE 流式
- 前端：单文件原生 HTML/JS（无构建），Canvas 图片压缩，MediaRecorder 录音
- 模型：任意 OpenAI 兼容云端 API

---

<a name="english"></a>
## English

### ✨ Features

**Triple-Modal Input**
- 🎙️ **Voice input**: phone recording → ASR → AI reply → emotional TTS playback
- ⌨️ **Text input**: multi-turn dialogue with persistent context (survives refresh/restart)
- 🖼️ **Image recognition**: photo/pick image, AI sees and answers (frontend Canvas compression)
- 🧩 **Image + text mixed**: send image with a question

**Roleplay**
- 🎭 **Multi-character management**: create multiple characters, each with independent persona / temperature / voice / session
- 🎤 **Per-character voice**: male/female voices configurable per character
- ✏️ **Edit & resend**: edit any historical message and regenerate (ChatGPT-style)
- 🗑️ **Clear context**: one-click restart
- 🎮 **Quick-choice buttons**: A / B / C / D / 1 / 2 / 3 / 4 / Continue for text games

**Voice Experience**
- 🔊 **Streaming playback**: AI streams as it generates (~70% buffered before playback to avoid stutter)
- 🔁 **Replay button**: replay any reply
- 💬 **Emotional TTS instruction**: configurable reading emotion

**Story Background Art**
- 🎨 **Parallel generation**: after AI replies, background manga art is generated in parallel without blocking dialogue
- 🖌️ **Smart prompt**: a lightweight LLM condenses the reply into a visual scene description, then draws
- 📐 **Configurable style / resolution**: sketch / manga / line art, 256–1024

**Other**
- 🔒 **Single-user token auth**
- 📱 **Mobile-first**: single-file H5, no build step, works on Android/iOS browsers
- 🌐 **Fully OpenAI-compatible APIs**: switch providers by editing `.env`, zero code change

### 🏗️ Architecture

```
Mobile H5 ──text/voice/image──▶ FastAPI backend (SSE streaming)
  ├─ ASR    voice→text   (OpenAI-compatible /audio/transcriptions)
  ├─ LLM    brain        (OpenAI-compatible /chat/completions)
  ├─ TTS    text→voice   (OpenAI-compatible /audio/speech)
  ├─ Summarize reply→scene (OpenAI-compatible /chat/completions)
  └─ Image  scene→art    (OpenAI-compatible /images/generations)
```

### 🔌 API Compatibility

| API | Default | Switch to |
|-----|---------|-----------|
| LLM brain | Step step-1-8k | DeepSeek, Kimi, GLM, OpenAI, Ollama, llama.cpp, any OpenAI-compatible |
| Summarize | Step step-1-8k | same as above (lightweight model) |
| ASR | Step stepaudio-2.5-asr | OpenAI Whisper, Alibaba, etc. |
| TTS | Step stepaudio-2.5-tts | OpenAI TTS, Alibaba, etc. |
| Image | Step step-2x-large | OpenAI DALL·E, Alibaba wan, etc. |

> 💰 **Cheapest config**: LLM via DeepSeek (cache hit ¥0.1/M tokens).

### 🚀 Deploy

```bash
python3 -m venv venv && source venv/bin/activate
pip install -r requirements.txt
sudo apt install ffmpeg
cp .env.example .env   # edit with your keys
uvicorn app:app --host 127.0.0.1 --port 8788
```

For public access over HTTPS (required for mic/camera), use [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/).

### 📱 Usage

Open `https://your-domain` in a mobile browser, enter the `BOT_TOKEN`.

> ⚠️ WeChat's in-app browser restricts audio autoplay; use the system browser (Safari/Chrome) or "Add to Home Screen".

### 🛠️ Tech Stack

- Backend: Python + FastAPI + httpx + SSE streaming
- Frontend: single-file vanilla HTML/JS (no build), Canvas image compression, MediaRecorder
- Models: any OpenAI-compatible cloud API

## License

MIT

---

## 入群交流

扫码加入 AJW 微信群，交流项目和产品进展。

![AJW 微信群二维码](assets/ajw-wechat-group-qr.png)

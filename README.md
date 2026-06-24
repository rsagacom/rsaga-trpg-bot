# Rsaga TRPG bot

一个手机友好的 H5 语音/文字/图片多模态 AI 角色扮演 bot。支持语音输入、文字输入、图片识别，AI 回复带情感语音播报，并根据剧情实时生成抽象线条背景画。

## 特性

- 🎙️ **语音输入**：手机录音 → ASR 转文字 → AI 回复 → TTS 语音播报
- ⌨️ **文字输入**：文字对话，支持多轮上下文（持久化，刷新不丢）
- 🖼️ **图片识别**：拍照/选图发送，AI 看图回答（前端压缩省流量）
- 🎭 **多角色管理**：创建多个角色，各自独立人设/温度/会话，随时切换
- ✏️ **编辑重发**：编辑任意历史消息重新生成
- 🎨 **剧情背景画**：AI 回复完，并行生成抽象线条场景画铺底层（不阻塞对话）
- 🎮 **文字游戏快捷键**：A/B/C/D/1/2/3/4/继续 一键选项
- 🔊 **流式朗读 + 重播**：边生成边播，每条回复可重播
- 🔒 **单用户 token 鉴权**：简单口令保护

## 架构

```
手机 H5 ──文字/录音/图片──▶ FastAPI 后端
  ├─ ASR   (OpenAI 兼容 /audio/transcriptions)
  ├─ LLM   (OpenAI 兼容 /chat/completions)   ← 大脑，可换任意家
  ├─ TTS   (OpenAI 兼容 /audio/speech)
  ├─ 总结  (OpenAI 兼容 /chat/completions)   ← 把回复浓缩成绘图 prompt
  └─ 绘图  (OpenAI 兼容 /images/generations)
```

所有云端接口均 OpenAI 兼容，换服务商只需改 `.env`，代码零改。

## 接口兼容性

| 接口 | 默认 | 可换为 |
|------|------|--------|
| LLM 大脑 | 阶跃 step-1-8k / 本地 Qwen | DeepSeek、Kimi、GLM、OpenAI、Ollama、llama.cpp 等任意 OpenAI 兼容 |
| 画面总结 | 阶跃 step-1-8k | 同上（轻量模型即可） |
| ASR | 阶跃 stepaudio-2.5-asr | OpenAI Whisper、阿里等 OpenAI Whisper 兼容 |
| TTS | 阶跃 stepaudio-2.5-tts | OpenAI TTS、阿里等 OpenAI 兼容（音色 ID 需对应调整） |
| 绘图 | 阶跃 step-2x-large | OpenAI DALL·E、阿里 wan 等 OpenAI 兼容 |

**最省 token 配置**：LLM 用 DeepSeek（缓存命中 ¥0.1/百万 token）。

## 部署

### 1. 环境
```bash
python3 -m venv venv && source venv/bin/activate
pip install -r requirements.txt
sudo apt install ffmpeg   # ASR 转码用
```

### 2. 配置
```bash
cp .env.example .env
# 编辑 .env 填入各服务商 key
```

### 3. 运行
```bash
uvicorn app:app --host 127.0.0.1 --port 8788
```

### 4. 常驻（systemd）
```bash
sudo cp voicebot.service /etc/systemd/system/
sudo systemctl enable --now voicebot
```

### 5. 公网访问（Cloudflare Tunnel，可选）
手机访问需 HTTPS（录音/拍照要求 secure context）。用 Cloudflare named tunnel 把 `localhost:8788` 暴露到自定义域名，免费且自动 HTTPS。详见 [Cloudflare Tunnel Runbook](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/)。

## 手机使用

浏览器打开 `https://你的域名`，输入 `BOT_TOKEN` 口令进入。

> 微信内置浏览器对音频自动播放限制严，建议用系统浏览器（Safari/Chrome）打开，或"添加到主屏幕"当 App 用。

## 配置项说明

见 `.env.example` 注释。关键项：
- `BOT_TOKEN`：访问口令
- `QWEN_*`：LLM 大脑
- `STEP_BASE` + `STEP_API_KEY`：ASR/TTS/绘图 共用（阶跃）
- `IMG_STYLE_PREFIX`：背景画风格提示词
- `TTS_INSTRUCTION`：语音朗读情感指令

## 技术栈

- 后端：Python + FastAPI + httpx + SSE 流式
- 前端：单文件原生 HTML/JS（无构建），Canvas 图片压缩，MediaRecorder 录音
- 模型：任意 OpenAI 兼容云端 API（默认阶跃星辰 StepAudio 2.5 + Step 文生图）

## License

MIT

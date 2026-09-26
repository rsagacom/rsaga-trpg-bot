#!/usr/bin/env bash
# 生产切换：voicebot FastAPI/uvicorn(8788) → voicebot-rs。
# ⚠️ 仅在用户明确确认上线后执行（QUICKSTART §7.1）。
# 执行内容：rust 二进制装入生产目录(共享 .env/sessions/personas 等状态) → 备份并改写 unit → 重启 → 验证。
# 回滚：bash cutover_voicebot.sh rollback
set -eu
PROD=/home/ec2-user/rsaga-trpg-bot
UNIT=/etc/systemd/system/voicebot.service
RUST_SRC=/home/ec2-user/voicebot-rs-test/voicebot-rs

rollback() {
  sudo cp "$(ls -t /etc/systemd/system/voicebot.service.bak-py-* | head -1)" "$UNIT"
  sudo systemctl daemon-reload
  sudo systemctl restart voicebot.service
  sleep 2
  systemctl is-active voicebot.service
  TOKEN=$(grep -oP '^BOT_TOKEN=\K.*' "$PROD/.env" | head -1)
  curl -s -m 5 -o /dev/null -w "8788=%{http_code}\n" -H "X-Bot-Token: $TOKEN" http://127.0.0.1:8788/api/health
  echo ROLLBACK_DONE
  exit 0
}

[ "${1:-}" = "rollback" ] && rollback

# 1. 二进制进生产目录（BASE_DIR=exe 目录 → 直接复用 .env/sessions.json/personas.json/tts_cache）
install -m 0755 "$RUST_SRC" "$PROD/voicebot-rs"
grep -q '^VB_PORT=' "$PROD/.env" && sed -i 's/^VB_PORT=.*/VB_PORT=8788/' "$PROD/.env" || echo 'VB_PORT=8788' >> "$PROD/.env"
grep -q '^VB_HOST=' "$PROD/.env" && sed -i 's/^VB_HOST=.*/VB_HOST=127.0.0.1/' "$PROD/.env" || echo 'VB_HOST=127.0.0.1' >> "$PROD/.env"

# 2. 备份并改写 unit
BAK=/etc/systemd/system/voicebot.service.bak-py-$(date +%Y%m%d)
sudo cp "$UNIT" "$BAK"
sudo tee "$UNIT" >/dev/null <<EOF
[Unit]
Description=Voice Bot (rust rewrite: Qwen/Grok + StepAudio)
After=network.target

[Service]
Type=simple
User=ec2-user
WorkingDirectory=$PROD
ExecStart=$PROD/voicebot-rs
Restart=on-failure
RestartSec=3
EnvironmentFile=$PROD/.env

[Install]
WantedBy=multi-user.target
EOF

# 3. 重启 + 验证
sudo systemctl daemon-reload
sudo systemctl restart voicebot.service
sleep 2
systemctl is-active voicebot.service
TOKEN=$(grep -oP '^BOT_TOKEN=\K.*' "$PROD/.env" | head -1)
curl -s -m 5 -o /dev/null -w "8788_health=%{http_code}\n" -H "X-Bot-Token: $TOKEN" http://127.0.0.1:8788/api/health
curl -s -m 5 -o /dev/null -w "8788_index=%{http_code}\n" http://127.0.0.1:8788/
echo "CUTOVER_DONE rust@8788 (python 备份: $BAK; 回滚: bash $0 rollback)"

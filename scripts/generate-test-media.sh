#!/usr/bin/env bash
# Gera a mídia sintética da ETAPA 0 em tests/fixtures/media/.
#
# Vídeo: H.264 sem B-frames, keyframe a cada 1 s, AUD em cada quadro e
# SPS/PPS repetidos antes de cada IDR (necessário para novos espectadores).
# Áudio: Opus 48 kHz estéreo, pacotes de 20 ms, bipe de 1 kHz a cada segundo
# alinhado ao contador visual do testsrc2 (verificação manual de sincronia).
#
# Requer ffmpeg com libx264 e libopus. Use FFMPEG=/caminho/ffmpeg para outro binário.
set -euo pipefail

FFMPEG="${FFMPEG:-ffmpeg}"
DURATION="${DURATION:-10}"
OUT="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/media"
mkdir -p "$OUT"

video() { # nome largura altura fps bitrate_kbps
  local name=$1 w=$2 h=$3 fps=$4 kbps=$5
  "$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "testsrc2=size=${w}x${h}:rate=${fps}:duration=${DURATION}" \
    -c:v libx264 -preset veryfast -tune zerolatency -profile:v high -pix_fmt yuv420p \
    -bf 0 -g "$fps" -keyint_min "$fps" -sc_threshold 0 \
    -b:v "${kbps}k" -maxrate "${kbps}k" -bufsize "$((kbps / 2))k" \
    -x264-params "aud=1:repeat-headers=1:slices=1" \
    -f h264 "$OUT/$name.h264"
  cat > "$OUT/$name.json" <<JSON
{ "name": "$name", "width": $w, "height": $h, "fps": $fps, "video": "$name.h264", "audio": "audio.opus" }
JSON
  echo "gerado: $name ($(du -h "$OUT/$name.h264" | cut -f1))"
}

"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=1000:beep_factor=4:sample_rate=48000:duration=${DURATION}" \
  -ac 2 -c:a libopus -b:a 96k -frame_duration 20 -application lowdelay \
  "$OUT/audio.opus"
echo "gerado: audio.opus"

video 720p30 1280 720 30 4000
video 1080p60 1920 1080 60 8000

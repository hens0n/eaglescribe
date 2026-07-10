#!/usr/bin/env bash
# Download a small English Whisper ggml model for the EagleScribe spike.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="${ROOT}/models"
MODEL="${1:-ggml-base.en.bin}"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${MODEL}"

mkdir -p "${OUT_DIR}"
DEST="${OUT_DIR}/${MODEL}"

if [[ -f "${DEST}" ]]; then
  echo "Already exists: ${DEST}"
  exit 0
fi

echo "Downloading ${MODEL} → ${DEST}"
curl -fL --progress-bar -o "${DEST}" "${URL}"
echo "Done. Set EAGLESCRIBE_WHISPER_MODEL=${DEST} if needed."

#!/usr/bin/env bash
# Fetch the benchmark data into a cache outside the repo:
#   - LibriSpeech test-clean (CC-BY-4.0): the first 100 utterances by sorted
#     id, converted to 16 kHz mono WAV, plus a reference transcript TSV.
#   - ggml-base.en.bin for the whisper.cpp baseline (bench only, never shipped).
# Usage: bench/fetch.sh [cache-dir]   (default ~/Library/Caches/purl-bench)
set -euo pipefail

CACHE="${1:-${PURL_BENCH_CACHE:-$HOME/Library/Caches/purl-bench}}"
LIBRI_URL="https://www.openslr.org/resources/12/test-clean.tar.gz"
LIBRI_MD5="32fa31d27d2e1cad72775fee3f4849a9"
WHISPER_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
WHISPER_SHA256="a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"
COUNT=100

mkdir -p "$CACHE/librispeech-100"

md5_of() { if command -v md5 >/dev/null; then md5 -q "$1"; else md5sum "$1" | cut -d' ' -f1; fi; }
sha256_of() { shasum -a 256 "$1" | cut -d' ' -f1; }

to_wav16k() {
  if command -v ffmpeg >/dev/null; then
    ffmpeg -nostdin -loglevel error -y -i "$1" -ar 16000 -ac 1 -c:a pcm_s16le "$2"
  else
    afconvert -f WAVE -d LEI16@16000 -c 1 "$1" "$2"
  fi
}

# 1. LibriSpeech test-clean
tarball="$CACHE/test-clean.tar.gz"
if [ ! -f "$tarball.ok" ]; then
  curl -fL --retry 3 -C - -o "$tarball" "$LIBRI_URL"
  [ "$(md5_of "$tarball")" = "$LIBRI_MD5" ] || { echo "test-clean md5 mismatch" >&2; exit 1; }
  touch "$tarball.ok"
fi
if [ ! -d "$CACHE/LibriSpeech/test-clean" ]; then
  tar -xzf "$tarball" -C "$CACHE"
fi

# First COUNT utterances by sorted id, from the per-chapter transcript files.
refs="$CACHE/librispeech-100/manifest.tsv"
find "$CACHE/LibriSpeech/test-clean" -name '*.trans.txt' -exec cat {} + \
  | LC_ALL=C sort \
  | awk -v n="$COUNT" 'NR <= n { id=$1; $1=""; sub(/^ /, ""); print id ".wav\t" $0 }' > "$refs"

while IFS=$'\t' read -r wav _; do
  id="${wav%.wav}"
  IFS=- read -r spk chap _ <<< "$id"
  out="$CACHE/librispeech-100/$wav"
  [ -f "$out" ] || to_wav16k "$CACHE/LibriSpeech/test-clean/$spk/$chap/$id.flac" "$out"
done < "$refs"
echo "LibriSpeech: $(wc -l < "$refs" | tr -d ' ') clips in $CACHE/librispeech-100"

# 2. whisper.cpp baseline model
ggml="$CACHE/ggml-base.en.bin"
if [ ! -f "$ggml" ] || [ "$(sha256_of "$ggml")" != "$WHISPER_SHA256" ]; then
  curl -fL --retry 3 -o "$ggml" "$WHISPER_URL"
  [ "$(sha256_of "$ggml")" = "$WHISPER_SHA256" ] || { echo "ggml-base.en.bin sha256 mismatch" >&2; exit 1; }
fi
echo "whisper baseline: $ggml"

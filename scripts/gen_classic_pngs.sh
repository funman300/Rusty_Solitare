#!/usr/bin/env bash
# Rasterize the classic SVG theme into the web game's card PNG assets.
set -euo pipefail

CLASSIC="solitaire_engine/assets/themes/classic"
OUT="assets/cards/faces"
BACKS_OUT="assets/cards/backs"

declare -A SUIT=([clubs]=C [diamonds]=D [hearts]=H [spades]=S)
declare -A RANK=([ace]=A [2]=2 [3]=3 [4]=4 [5]=5 [6]=6 [7]=7 [8]=8 [9]=9 [10]=10 [jack]=J [queen]=Q [king]=K)

mkdir -p "$OUT" "$BACKS_OUT"

for svg in "$CLASSIC"/*_*.svg; do
    base=$(basename "$svg" .svg)          # e.g. clubs_ace
    suit_name="${base%%_*}"               # clubs
    rank_name="${base#*_}"                # ace
    suit_code="${SUIT[$suit_name]:-}"
    rank_code="${RANK[$rank_name]:-}"
    if [ -z "$suit_code" ] || [ -z "$rank_code" ]; then
        echo "skip: $base"
        continue
    fi
    out="$OUT/${rank_code}${suit_code}.png"
    rsvg-convert -w 256 -h 384 "$svg" -o "$out"
    echo "  $base -> $out"
done

# Back
rsvg-convert -w 256 -h 384 "$CLASSIC/back.svg" -o "$BACKS_OUT/back_0.png"
echo "  back -> $BACKS_OUT/back_0.png"

echo "Done."

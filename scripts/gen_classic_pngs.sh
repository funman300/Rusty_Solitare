#!/usr/bin/env bash
# Rasterize card SVG themes into the web game's PNG asset subdirectories.
# Usage: bash scripts/gen_classic_pngs.sh
# Outputs: assets/cards/faces/{classic,dark}/ and assets/cards/backs/{classic,dark}/
set -euo pipefail

declare -A SUIT=([clubs]=C [diamonds]=D [hearts]=H [spades]=S)
declare -A RANK=([ace]=A [2]=2 [3]=3 [4]=4 [5]=5 [6]=6 [7]=7 [8]=8 [9]=9 [10]=10 [jack]=J [queen]=Q [king]=K)

rasterize_theme() {
    local theme="$1"
    local src="solitaire_engine/assets/themes/$theme"
    local faces_out="assets/cards/faces/$theme"
    local backs_out="assets/cards/backs/$theme"
    mkdir -p "$faces_out" "$backs_out"

    for svg in "$src"/*_*.svg; do
        local base suit_name rank_name suit_code rank_code
        base=$(basename "$svg" .svg)
        suit_name="${base%%_*}"
        rank_name="${base#*_}"
        suit_code="${SUIT[$suit_name]:-}"
        rank_code="${RANK[$rank_name]:-}"
        [ -z "$suit_code" ] || [ -z "$rank_code" ] && continue
        rsvg-convert -w 256 -h 384 "$svg" -o "$faces_out/${rank_code}${suit_code}.png"
    done

    rsvg-convert -w 256 -h 384 "$src/back.svg" -o "$backs_out/back_0.png"
    echo "done: $theme ($(ls "$faces_out" | wc -l) faces)"
}

rasterize_theme classic
rasterize_theme dark

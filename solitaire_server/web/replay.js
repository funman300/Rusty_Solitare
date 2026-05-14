// Ferrous Solitaire replay viewer.
//
// Pulls the replay JSON from `/api/replays/:id`, hands it to the
// `solitaire_wasm` ReplayPlayer (which owns a real solitaire_core
// `GameState` compiled to WebAssembly), and renders each step's pile
// snapshot as plain HTML cards. The WASM module is the single source
// of truth for the rules engine — we don't re-implement Klondike in JS.
//
// Card flight animation: each card's DOM element persists across
// re-renders, keyed by `card.id`. `render()` updates each card's
// `transform: translate(...)` to its new (pile, index) coordinates;
// the CSS `transition` on `transform` animates the flight. Cards that
// disappear from the snapshot fade and remove; new cards fade in at
// their target position.

import init, { ReplayPlayer } from "/web/pkg/solitaire_wasm.js";

const STEP_INTERVAL_MS = 600;
const FAN_OFFSET_PX = 28;
const CARD_W = 80;
const CARD_H = 112;
const GAP = 12;

// Pile origin (top-left of the slot, in board-relative pixels).
// Top row: stock at column 0, waste at column 1, foundations at 3-6.
// Bottom row: tableau columns 0-6.
const TOP_ROW_Y = 0;
const TABLEAU_ROW_Y = CARD_H + 32;
const colX = (col) => col * (CARD_W + GAP);

const PILE_ORIGIN = {
    stock: { x: colX(0), y: TOP_ROW_Y },
    waste: { x: colX(1), y: TOP_ROW_Y },
    "foundation-0": { x: colX(3), y: TOP_ROW_Y },
    "foundation-1": { x: colX(4), y: TOP_ROW_Y },
    "foundation-2": { x: colX(5), y: TOP_ROW_Y },
    "foundation-3": { x: colX(6), y: TOP_ROW_Y },
    "tableau-0": { x: colX(0), y: TABLEAU_ROW_Y },
    "tableau-1": { x: colX(1), y: TABLEAU_ROW_Y },
    "tableau-2": { x: colX(2), y: TABLEAU_ROW_Y },
    "tableau-3": { x: colX(3), y: TABLEAU_ROW_Y },
    "tableau-4": { x: colX(4), y: TABLEAU_ROW_Y },
    "tableau-5": { x: colX(5), y: TABLEAU_ROW_Y },
    "tableau-6": { x: colX(6), y: TABLEAU_ROW_Y },
};

const SUIT_GLYPHS = {
    clubs: "♣",
    diamonds: "♦",
    hearts: "♥",
    spades: "♠",
};
const RED_SUITS = new Set(["diamonds", "hearts"]);
const RANK_LABELS = ["", "A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K"];

const board = document.getElementById("board");
const captionEl = document.getElementById("caption");
const progressEl = document.getElementById("progress");
const scoreEl = document.getElementById("score");
const movesEl = document.getElementById("moves");
const resultEl = document.getElementById("result");
const btnPlay = document.getElementById("btn-play");
const btnStep = document.getElementById("btn-step");
const btnPrev = document.getElementById("btn-prev");

let player = null;
let replayJson = null;
let playInterval = null;

// Persistent map: card.id → DOM element. Reused across renders so the
// browser interpolates the `transform` change rather than rebuilding
// nodes every step.
const cardEls = new Map();

async function bootstrap() {
    const id = window.location.pathname.split("/").pop();
    if (!id) {
        captionEl.textContent = "No replay id in URL.";
        return;
    }

    let response;
    try {
        response = await fetch(`/api/replays/${id}`);
    } catch (e) {
        captionEl.textContent = `Network error: ${e}`;
        return;
    }
    if (!response.ok) {
        captionEl.textContent = `Server returned ${response.status}.`;
        return;
    }
    const replay = await response.json();
    replayJson = JSON.stringify(replay);

    captionEl.textContent =
        `Seed ${replay.seed} · ${replay.draw_mode} · ${replay.mode} ` +
        `· ${formatDuration(replay.time_seconds)} win on ${replay.recorded_at} ` +
        `· final score ${replay.final_score}`;

    spawnEmptySlots();
    await init();
    resetPlayer();
}

/// Spawn the dashed empty-pile placeholders once. They never move and
/// never get keyed to card ids, so they're outside the cardEls map.
function spawnEmptySlots() {
    Object.entries(PILE_ORIGIN).forEach(([name, { x, y }]) => {
        const slot = document.createElement("div");
        slot.className = `slot slot-${name}`;
        slot.style.transform = `translate(${x}px, ${y}px)`;
        board.appendChild(slot);
    });
}

function resetPlayer() {
    if (playInterval) {
        clearInterval(playInterval);
        playInterval = null;
        btnPlay.textContent = "▶ Play";
    }
    player = new ReplayPlayer(replayJson);
    btnPrev.disabled = true;
    btnStep.disabled = false;
    btnPlay.disabled = false;
    render(player.state());
}

function step() {
    const snap = player.step();
    if (snap === null) {
        finish();
        return null;
    }
    btnPrev.disabled = false;
    render(snap);
    return snap;
}

function finish() {
    if (playInterval) {
        clearInterval(playInterval);
        playInterval = null;
    }
    btnPlay.textContent = "▶ Play";
    btnPlay.disabled = true;
    btnStep.disabled = true;
}

/// Apply `snap` to the persistent card-element map.
///
/// Phase 1: collect every card present in this snapshot, computing its
/// target board-relative (x, y) from its pile + index.
/// Phase 2: for each card, find or create its DOM element and update
/// its visual state + transform. Persistent elements interpolate via
/// CSS transition; freshly-created ones fade in.
/// Phase 3: any card present in `cardEls` but absent from `snap` (rare
/// but happens during stat resets) fades out and is removed.
function render(snap) {
    if (!snap) return;

    const targets = new Map(); // card.id → { card, x, y }

    function placePile(name, cards, fan) {
        const origin = PILE_ORIGIN[name];
        cards.forEach((card, idx) => {
            const yOffset = fan ? idx * FAN_OFFSET_PX : 0;
            targets.set(card.id, {
                card,
                x: origin.x,
                y: origin.y + yOffset,
                z: idx,
            });
        });
    }

    placePile("stock", snap.stock, false);
    placePile("waste", snap.waste, false);
    snap.foundations.forEach((cards, idx) =>
        placePile(`foundation-${idx}`, cards, false));
    snap.tableaus.forEach((cards, idx) =>
        placePile(`tableau-${idx}`, cards, true));

    // Apply or create.
    targets.forEach(({ card, x, y, z }) => {
        let el = cardEls.get(card.id);
        if (!el) {
            el = createCardElement(card);
            // Spawn off-screen with opacity 0 so the entry transition
            // fades in at the destination rather than popping.
            el.style.transform = `translate(${x}px, ${y}px)`;
            el.style.opacity = "0";
            board.appendChild(el);
            cardEls.set(card.id, el);
            // Force the browser to commit the off-screen frame before
            // we set the visible state, so the transition runs.
            requestAnimationFrame(() => {
                el.style.opacity = "1";
            });
        } else {
            updateCardElement(el, card);
            el.style.transform = `translate(${x}px, ${y}px)`;
        }
        el.style.zIndex = String(z + 1);
    });

    // Drop any cards no longer in play (e.g. on player reset).
    cardEls.forEach((el, id) => {
        if (!targets.has(id)) {
            el.style.opacity = "0";
            // Remove after the fade transition completes.
            setTimeout(() => {
                el.remove();
                cardEls.delete(id);
            }, 220);
        }
    });

    progressEl.textContent = `step ${snap.step_idx} / ${snap.total_steps}`;
    scoreEl.textContent = `Score ${snap.score}`;
    movesEl.textContent = `Moves ${snap.move_count}`;
    if (snap.is_won) {
        resultEl.textContent = "✨ Won";
        resultEl.classList.add("win");
    } else {
        resultEl.textContent = "";
        resultEl.classList.remove("win");
    }
}

function createCardElement(card) {
    const el = document.createElement("div");
    el.className = "card";
    el.dataset.cardId = String(card.id);
    populateCardFace(el, card);
    return el;
}

/// Cheap "is this still the same visual state" check. Face-up cards
/// only need a re-paint if their face_up flag flipped (rank/suit are
/// immutable per id), so we can skip rebuilding the inner DOM for the
/// 99% case where only the transform changed.
function updateCardElement(el, card) {
    const wasFaceDown = el.classList.contains("face-down");
    const isFaceDown = !card.face_up;
    if (wasFaceDown !== isFaceDown) {
        el.replaceChildren();
        el.classList.remove("red", "black", "face-down");
        populateCardFace(el, card);
    }
}

function populateCardFace(el, card) {
    if (!card.face_up) {
        el.classList.add("face-down");
        return;
    }
    el.classList.add(RED_SUITS.has(card.suit) ? "red" : "black");
    const label = RANK_LABELS[card.rank] || "?";
    const glyph = SUIT_GLYPHS[card.suit] || "?";

    const top = document.createElement("span");
    top.className = "corner top";
    top.textContent = `${label}\n${glyph}`;
    el.appendChild(top);

    const center = document.createElement("span");
    center.className = "center";
    center.textContent = glyph;
    el.appendChild(center);

    const bottom = document.createElement("span");
    bottom.className = "corner bottom";
    bottom.textContent = `${label}\n${glyph}`;
    el.appendChild(bottom);
}

function formatDuration(seconds) {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
}

btnStep.addEventListener("click", () => {
    if (player) step();
});

btnPlay.addEventListener("click", () => {
    if (!player) return;
    if (playInterval) {
        clearInterval(playInterval);
        playInterval = null;
        btnPlay.textContent = "▶ Play";
        return;
    }
    btnPlay.textContent = "⏸ Pause";
    playInterval = setInterval(() => {
        const snap = step();
        if (snap === null) finish();
    }, STEP_INTERVAL_MS);
});

btnPrev.addEventListener("click", () => {
    if (!replayJson) return;
    // Drop every existing card so the next render fades them all in
    // at the freshly-dealt positions. Without this, cards from the
    // current state would slide to wherever the new deal puts them
    // — confusing since the deal is supposed to look like a fresh
    // start, not a continuation.
    cardEls.forEach((el) => el.remove());
    cardEls.clear();
    resetPlayer();
});

bootstrap();

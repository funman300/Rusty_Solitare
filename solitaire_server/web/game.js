// Solitaire Quest — interactive browser game.
//
// Architecture:
//   - `SolitaireGame` (Rust/WASM via solitaire_core) owns all rule logic.
//   - This file owns the DOM renderer, drag-and-drop input, and the game loop.
//   - Cards are persistent DOM elements keyed by `card.id`; positions are
//     updated via `transform: translate(...)` so the browser can animate
//     flights on the compositor thread.
//
// Pile name convention (mirrors solitaire_wasm::SolitaireGame::move_cards):
//   "stock" | "waste" | "foundation-0".."foundation-3" | "tableau-0".."tableau-6"

import init, { SolitaireGame } from "/web/pkg/solitaire_wasm.js";

// ── Layout constants (must match game.css --card-w / --card-h / --gap / --pad)
const CARD_W    = 80;
const CARD_H    = 112;
const GAP       = 12;
const PAD       = 20;   // board inner padding — cards start at (PAD, PAD)
const FAN       = 28;   // vertical offset per fanned tableau card
const WASTE_FAN = 18;   // horizontal offset for draw-3 waste fan

// Pile origins in board-element coordinates (include PAD so (0,0) = board edge).
const TOP_Y    = PAD;
const BOTTOM_Y = PAD + CARD_H + 28;

const colX = (c) => PAD + c * (CARD_W + GAP);

const PILE_ORIGIN = {
    stock:          { x: colX(0), y: TOP_Y },
    waste:          { x: colX(1), y: TOP_Y },
    "foundation-0": { x: colX(3), y: TOP_Y },
    "foundation-1": { x: colX(4), y: TOP_Y },
    "foundation-2": { x: colX(5), y: TOP_Y },
    "foundation-3": { x: colX(6), y: TOP_Y },
    "tableau-0":    { x: colX(0), y: BOTTOM_Y },
    "tableau-1":    { x: colX(1), y: BOTTOM_Y },
    "tableau-2":    { x: colX(2), y: BOTTOM_Y },
    "tableau-3":    { x: colX(3), y: BOTTOM_Y },
    "tableau-4":    { x: colX(4), y: BOTTOM_Y },
    "tableau-5":    { x: colX(5), y: BOTTOM_Y },
    "tableau-6":    { x: colX(6), y: BOTTOM_Y },
};

// Foundation suit hints shown when the slot is empty.
const FOUND_SUIT_HINT = ["♠", "♥", "♦", "♣"];

const SUIT_GLYPH  = { clubs: "♣", diamonds: "♦", hearts: "♥", spades: "♠" };
const RANK_LABELS = ["","A","2","3","4","5","6","7","8","9","10","J","Q","K"];
const RED_SUITS   = new Set(["diamonds", "hearts"]);

// ── State ────────────────────────────────────────────────────────────────────
let game      = null;
let snap      = null;   // last rendered GameSnapshot
let drawThree = false;

// Persistent card-id → DOM element map.
const cardEls = new Map();

// Drag state
let drag = null;
// drag = {
//   fromPile: string,
//   fromIndex: number,       // index of bottom dragged card in its pile
//   cardIds: number[],       // ids bottom→top
//   startX: number, startY: number,   // board-relative pointer start
// }

// Timer
let timerInterval = null;
let elapsedSecs   = 0;

// Auto-complete
let acTimer = null;

// ── DOM refs ─────────────────────────────────────────────────────────────────
const board      = document.getElementById("board");
const hudScore   = document.getElementById("hud-score");
const hudMoves   = document.getElementById("hud-moves");
const hudTimer   = document.getElementById("hud-timer");
const hudStock   = document.getElementById("hud-stock");
const hudSeed    = document.getElementById("hud-seed");
const btnUndo    = document.getElementById("btn-undo");
const btnNew     = document.getElementById("btn-new");
const chkDraw3   = document.getElementById("chk-draw3");
const winOverlay = document.getElementById("win-overlay");
const winScore   = document.getElementById("win-score");
const winMoves   = document.getElementById("win-moves");
const winTime    = document.getElementById("win-time");
const btnWinNew  = document.getElementById("btn-win-new");

// ── Bootstrap ────────────────────────────────────────────────────────────────
async function bootstrap() {
    await init();

    const params  = new URLSearchParams(window.location.search);
    const urlSeed = params.has("seed") ? Number(params.get("seed")) : randomSeed();
    drawThree     = params.has("draw3");
    chkDraw3.checked = drawThree;

    buildSlots();
    startGame(urlSeed);
    attachHandlers();
}

function randomSeed() {
    return Math.floor(Math.random() * 9007199254740991);
}

function startGame(seed) {
    if (acTimer)    { clearInterval(acTimer);    acTimer    = null; }
    if (timerInterval) { clearInterval(timerInterval); timerInterval = null; }
    elapsedSecs = 0;
    updateTimerDisplay();

    game = new SolitaireGame(seed, drawThree);
    snap = game.state();

    const displaySeed = Math.round(game.seed());
    hudSeed.textContent = `seed ${displaySeed}`;
    winOverlay.classList.add("hidden");
    cardEls.clear();
    board.querySelectorAll(".card, .recycle-label").forEach(el => el.remove());

    // Persist seed in URL so the game can be shared / refreshed.
    const url = new URL(window.location);
    url.searchParams.set("seed", displaySeed);
    if (drawThree) url.searchParams.set("draw3", "");
    else           url.searchParams.delete("draw3");
    history.replaceState(null, "", url);

    render(snap);
    startTimer();
}

// ── Timer ────────────────────────────────────────────────────────────────────
function startTimer() {
    timerInterval = setInterval(() => {
        elapsedSecs++;
        updateTimerDisplay();
    }, 1000);
}

function stopTimer() {
    if (timerInterval) { clearInterval(timerInterval); timerInterval = null; }
}

function updateTimerDisplay() {
    const m = Math.floor(elapsedSecs / 60);
    const s = elapsedSecs % 60;
    if (hudTimer) hudTimer.textContent = `${m}:${s.toString().padStart(2, "0")}`;
}

// ── Slot placeholders ─────────────────────────────────────────────────────────
function buildSlots() {
    for (const [pile, origin] of Object.entries(PILE_ORIGIN)) {
        const el = document.createElement("div");
        el.className = "slot";
        el.dataset.pile = pile;
        el.style.transform = `translate(${origin.x}px, ${origin.y}px)`;

        if (pile.startsWith("foundation-")) {
            const slot = parseInt(pile.split("-")[1]);
            const hint = document.createElement("div");
            hint.className = "slot-hint";
            hint.textContent = FOUND_SUIT_HINT[slot];
            el.appendChild(hint);
        }
        board.appendChild(el);
    }
}

// ── Card position math ────────────────────────────────────────────────────────
function cardPos(pileName, indexInPile, pileLength) {
    const origin = PILE_ORIGIN[pileName];
    let x = origin.x;
    let y = origin.y;

    if (pileName === "waste" && drawThree && pileLength >= 2) {
        const fanStart = Math.max(0, pileLength - 3);
        const fanPos   = indexInPile - fanStart;
        if (fanPos > 0) x += fanPos * WASTE_FAN;
    } else if (pileName.startsWith("tableau-")) {
        y += indexInPile * FAN;
    }
    return { x, y };
}

function cardZ(pileName, indexInPile) {
    return 10 + indexInPile;
}

// ── Renderer ──────────────────────────────────────────────────────────────────
function render(s) {
    snap = s;

    hudScore.textContent = `Score: ${s.score}`;
    hudMoves.textContent = `Moves: ${s.move_count}`;
    if (hudStock) hudStock.textContent = `Stock: ${s.stock.length}`;
    btnUndo.disabled     = s.undo_stack_len === 0;

    const visible = new Map();
    const addPile = (name, cards) =>
        cards.forEach((c, i) => visible.set(c.id, { pile: name, idx: i, card: c, total: cards.length }));

    addPile("stock",  s.stock);
    addPile("waste",  s.waste);
    s.foundations.forEach((f, i) => addPile(`foundation-${i}`, f));
    s.tableaus.forEach((t, i)    => addPile(`tableau-${i}`, t));

    for (const [id, info] of visible) {
        let el = cardEls.get(id);
        if (!el) {
            el = document.createElement("div");
            el.dataset.cardId = id;
            cardEls.set(id, el);
            board.appendChild(el);
        }
        updateCardEl(el, info.card, info.pile, info.idx, info.total);
    }

    for (const [id, el] of cardEls) {
        if (!visible.has(id)) { el.remove(); cardEls.delete(id); }
    }

    // Foundation suit hints: hide when pile has cards.
    s.foundations.forEach((f, i) => {
        const slotEl = board.querySelector(`.slot[data-pile="foundation-${i}"]`);
        if (slotEl) {
            const hint = slotEl.querySelector(".slot-hint");
            if (hint) hint.style.visibility = f.length > 0 ? "hidden" : "";
        }
    });

    // Recycle indicator on empty stock.
    let recycleEl = board.querySelector(".recycle-label");
    if (s.stock.length === 0 && s.waste.length > 0) {
        if (!recycleEl) {
            recycleEl = document.createElement("div");
            recycleEl.className = "recycle-label";
            recycleEl.textContent = "↺";
            board.appendChild(recycleEl);
        }
        const o = PILE_ORIGIN.stock;
        recycleEl.style.transform = `translate(${o.x + CARD_W / 2}px, ${o.y + CARD_H / 2}px)`;
    } else if (recycleEl) {
        recycleEl.remove();
    }

    // Clear drag highlights left from pointer-move.
    board.querySelectorAll(".slot.drop-active").forEach(e => e.classList.remove("drop-active"));
    board.querySelectorAll(".card.drop-target").forEach(e => e.classList.remove("drop-target"));

    if (s.is_auto_completable && !s.is_won && !acTimer) {
        acTimer = setInterval(doAutoCompleteStep, 380);
    }
    if (s.is_won) {
        stopTimer();
        if (acTimer) { clearInterval(acTimer); acTimer = null; }
        showWin(s);
    }
}

function updateCardEl(el, card, pileName, idx, total) {
    const pos = cardPos(pileName, idx, total);
    el.style.transform = `translate(${pos.x}px, ${pos.y}px)`;
    el.style.zIndex    = cardZ(pileName, idx);

    if (!card.face_up) {
        el.className = "card face-down";
        el.innerHTML  = "";
    } else {
        const isRed = RED_SUITS.has(card.suit);
        el.className = `card ${isRed ? "red" : "black"}`;
        const r = RANK_LABELS[card.rank];
        const s = SUIT_GLYPH[card.suit];
        el.innerHTML = `<div class="corner top">${r}<br>${s}</div>
                        <div class="center">${s}</div>
                        <div class="corner bottom">${r}<br>${s}</div>`;
    }
}

// ── Win overlay ───────────────────────────────────────────────────────────────
function showWin(s) {
    winScore.textContent = `Score: ${s.score}`;
    winMoves.textContent = `${s.move_count} moves`;
    const m = Math.floor(elapsedSecs / 60);
    const sec = elapsedSecs % 60;
    if (winTime) winTime.textContent = `${m}:${sec.toString().padStart(2, "0")}`;
    winOverlay.classList.remove("hidden");
}

// ── Auto-complete ─────────────────────────────────────────────────────────────
function doAutoCompleteStep() {
    if (!game || !snap?.is_auto_completable) {
        clearInterval(acTimer); acTimer = null; return;
    }
    const result = game.auto_complete_step();
    if (result?.ok) render(result.snapshot);
    else { clearInterval(acTimer); acTimer = null; }
}

// ── Illegal move flash ────────────────────────────────────────────────────────
function flashIllegal(cardIds) {
    for (const id of cardIds) {
        const el = cardEls.get(id);
        if (!el) continue;
        // Store current translate so the shake keyframe can reference it.
        el.style.setProperty("--card-tx", el.style.transform || "translate(0,0)");
        el.classList.add("illegal");
        el.addEventListener("animationend", () => {
            el.classList.remove("illegal");
            el.style.removeProperty("--card-tx");
        }, { once: true });
    }
}

// ── Input ─────────────────────────────────────────────────────────────────────
function attachHandlers() {
    btnUndo.addEventListener("click", () => {
        const r = game.undo();
        if (r.ok) render(r.snapshot);
    });
    btnNew.addEventListener("click", () => startGame(randomSeed()));
    btnWinNew.addEventListener("click", () => startGame(randomSeed()));
    chkDraw3.addEventListener("change", () => {
        drawThree = chkDraw3.checked;
        startGame(randomSeed());
    });

    document.addEventListener("keydown", (e) => {
        if (e.target.tagName === "INPUT") return;
        if (e.key === "z" || e.key === "Z") { const r = game.undo(); if (r.ok) render(r.snapshot); }
        if (e.key === "n" || e.key === "N") startGame(randomSeed());
    });

    board.addEventListener("pointerdown",   onPointerDown);
    board.addEventListener("pointermove",   onPointerMove);
    board.addEventListener("pointerup",     onPointerUp);
    board.addEventListener("pointercancel", onPointerCancel);
    board.addEventListener("click",         onBoardClick);
    board.addEventListener("dblclick",      onBoardDblClick);
}

// ── Coordinate helpers ────────────────────────────────────────────────────────
// Returns cursor position in board-element coordinates
// (0,0 = board element top-left corner, which is the padding edge).
function boardRelative(clientX, clientY) {
    const rect = board.getBoundingClientRect();
    return { x: clientX - rect.left, y: clientY - rect.top };
}

function hitTestCard(bx, by) {
    const pileOrder = [
        "waste",
        "foundation-0","foundation-1","foundation-2","foundation-3",
        "tableau-0","tableau-1","tableau-2","tableau-3","tableau-4","tableau-5","tableau-6",
        "stock",
    ];

    let best = null, bestZ = -1;

    for (const pileName of pileOrder) {
        const cards = getPileCards(pileName);
        if (!cards) continue;
        for (let i = 0; i < cards.length; i++) {
            const pos = cardPos(pileName, i, cards.length);
            if (bx >= pos.x && bx <= pos.x + CARD_W &&
                by >= pos.y && by <= pos.y + CARD_H) {
                const z = cardZ(pileName, i);
                if (z > bestZ) {
                    bestZ = z;
                    best  = { pileName, cardIndex: i, card: cards[i] };
                }
            }
        }
    }
    return best;
}

function getPileCards(pileName) {
    if (!snap) return null;
    if (pileName === "stock") return snap.stock;
    if (pileName === "waste") return snap.waste;
    if (pileName.startsWith("foundation-")) return snap.foundations[parseInt(pileName.split("-")[1])];
    if (pileName.startsWith("tableau-"))    return snap.tableaus   [parseInt(pileName.split("-")[1])];
    return null;
}

// Drop-target: tableau has tall hit areas; foundations use their slot box.
function findDropTarget(bx, by) {
    for (let c = 0; c < 7; c++) {
        const pile    = `tableau-${c}`;
        const cards   = snap.tableaus[c];
        const origin  = PILE_ORIGIN[pile];
        const bottomY = cards.length > 0
            ? origin.y + (cards.length - 1) * FAN + CARD_H
            : origin.y + CARD_H;
        if (bx >= origin.x && bx <= origin.x + CARD_W && by >= origin.y && by <= bottomY)
            return pile;
    }
    for (let s = 0; s < 4; s++) {
        const pile   = `foundation-${s}`;
        const origin = PILE_ORIGIN[pile];
        if (bx >= origin.x && bx <= origin.x + CARD_W &&
            by >= origin.y && by <= origin.y + CARD_H)
            return pile;
    }
    return null;
}

// ── Pointer handlers ──────────────────────────────────────────────────────────
function onPointerDown(e) {
    if (e.button !== 0 && e.pointerType === "mouse") return;
    if (drag) return;

    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const hit = hitTestCard(bx, by);
    if (!hit || !hit.card.face_up) return;

    const cards = getPileCards(hit.pileName);
    if (!cards) return;

    let fromIndex = hit.cardIndex;
    if (!hit.pileName.startsWith("tableau-")) {
        fromIndex = cards.length - 1;
        if (hit.cardIndex !== fromIndex) return;
    }

    const draggedCards = cards.slice(fromIndex);
    if (draggedCards.some(c => !c.face_up)) return;

    drag = {
        fromPile:  hit.pileName,
        fromIndex,
        cardIds:   draggedCards.map(c => c.id),
        startX: bx,
        startY: by,
    };

    drag.cardIds.forEach((id, i) => {
        const el = cardEls.get(id);
        if (el) { el.classList.add("selected"); el.style.zIndex = 500 + i; }
    });

    board.setPointerCapture(e.pointerId);
    e.preventDefault();
}

function onPointerMove(e) {
    if (!drag) return;
    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const dx = bx - drag.startX;
    const dy = by - drag.startY;

    const cards = getPileCards(drag.fromPile);
    drag.cardIds.forEach((id, i) => {
        const el = cardEls.get(id);
        if (!el) return;
        const base = cardPos(drag.fromPile, drag.fromIndex + i, cards ? cards.length : 1);
        el.style.transform = `translate(${base.x + dx}px, ${base.y + dy}px)`;
    });

    // Highlight drop target.
    board.querySelectorAll(".slot.drop-active").forEach(e => e.classList.remove("drop-active"));
    board.querySelectorAll(".card.drop-target").forEach(e => e.classList.remove("drop-target"));
    const targetPile = findDropTarget(bx, by);
    if (targetPile) {
        const slotEl = board.querySelector(`.slot[data-pile="${targetPile}"]`);
        if (slotEl) slotEl.classList.add("drop-active");
        const targetCards = getPileCards(targetPile);
        if (targetCards?.length > 0) {
            const topEl = cardEls.get(targetCards[targetCards.length - 1].id);
            if (topEl) topEl.classList.add("drop-target");
        }
    }

    e.preventDefault();
}

function onPointerUp(e) {
    if (!drag) return;
    board.querySelectorAll(".slot.drop-active").forEach(e => e.classList.remove("drop-active"));
    board.querySelectorAll(".card.drop-target").forEach(e => e.classList.remove("drop-target"));

    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const targetPile = findDropTarget(bx, by);

    let moved = false;
    if (targetPile && targetPile !== drag.fromPile) {
        const r = game.move_cards(drag.fromPile, targetPile, drag.cardIds.length);
        if (r.ok) {
            moved = true;
            drag.cardIds.forEach(id => cardEls.get(id)?.classList.remove("selected"));
            render(r.snapshot);
        } else {
            flashIllegal(drag.cardIds);
        }
    }

    if (!moved) {
        drag.cardIds.forEach(id => cardEls.get(id)?.classList.remove("selected"));
        render(snap); // snap cards back to their pre-drag positions
    }

    drag = null;
}

function onPointerCancel() {
    if (!drag) return;
    drag.cardIds.forEach(id => cardEls.get(id)?.classList.remove("selected"));
    render(snap);
    drag = null;
}

// ── Click / dblclick ──────────────────────────────────────────────────────────
function onBoardClick(e) {
    if (drag) return;
    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const stock = PILE_ORIGIN.stock;
    if (bx >= stock.x && bx <= stock.x + CARD_W && by >= stock.y && by <= stock.y + CARD_H) {
        const r = game.draw();
        if (r.ok) render(r.snapshot);
    }
}

function onBoardDblClick(e) {
    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const hit = hitTestCard(bx, by);
    if (!hit || !hit.card.face_up) return;

    const cards = getPileCards(hit.pileName);
    if (!cards || hit.cardIndex !== cards.length - 1) return;

    for (let s = 0; s < 4; s++) {
        const r = game.move_cards(hit.pileName, `foundation-${s}`, 1);
        if (r.ok) { render(r.snapshot); return; }
    }
}

// ── Start ─────────────────────────────────────────────────────────────────────
bootstrap().catch(console.error);

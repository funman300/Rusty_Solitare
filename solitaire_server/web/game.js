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

// ── Layout constants (must match game.css --card-w / --card-h / --gap) ──────
const CARD_W  = 80;
const CARD_H  = 112;
const GAP     = 12;
const PAD     = 20;   // board padding
const FAN     = 28;   // vertical offset per fanned tableau card
const WASTE_FAN = 18; // horizontal offset for draw-3 waste fan

// Top-row Y origin (relative to board interior = after padding).
const TOP_Y    = 0;
const BOTTOM_Y = CARD_H + 28;   // tableau row

const colX = (c) => c * (CARD_W + GAP);

// Absolute position of each pile's origin (top-left of its slot),
// relative to the board's padded interior (0, 0).
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

const SUIT_GLYPH  = { clubs: "♣", diamonds: "♦", hearts: "♥", spades: "♠" };
const RANK_LABELS = ["","A","2","3","4","5","6","7","8","9","10","J","Q","K"];
const RED_SUITS   = new Set(["diamonds", "hearts"]);

// ── State ────────────────────────────────────────────────────────────────────
let game   = null;
let snap   = null;   // last rendered GameSnapshot
let drawThree = false;

// Persistent card → DOM element map (keyed by card.id).
const cardEls = new Map();

// Drag state
let drag = null;
// drag = {
//   fromPile: string,
//   fromIndex: number,       // index of bottom card of the dragged run in its pile
//   cardIds: number[],       // ids of cards being dragged (bottom → top)
//   startX: number, startY: number,   // pointer start (board-relative)
//   offsetX: number, offsetY: number, // cursor offset within the grabbed card
// }

// Auto-complete timer handle
let acTimer = null;

// ── DOM refs ─────────────────────────────────────────────────────────────────
const board      = document.getElementById("board");
const hudScore   = document.getElementById("hud-score");
const hudMoves   = document.getElementById("hud-moves");
const hudSeed    = document.getElementById("hud-seed");
const btnUndo    = document.getElementById("btn-undo");
const btnNew     = document.getElementById("btn-new");
const chkDraw3   = document.getElementById("chk-draw3");
const winOverlay = document.getElementById("win-overlay");
const winScore   = document.getElementById("win-score");
const winMoves   = document.getElementById("win-moves");
const btnWinNew  = document.getElementById("btn-win-new");

// ── Bootstrap ────────────────────────────────────────────────────────────────
async function bootstrap() {
    await init();

    // Seed from URL param ?seed=N, otherwise random.
    const params = new URLSearchParams(window.location.search);
    const urlSeed = params.has("seed") ? Number(params.get("seed")) : randomSeed();
    drawThree = params.has("draw3");
    chkDraw3.checked = drawThree;

    buildSlots();
    startGame(urlSeed);
    attachHandlers();
}

function randomSeed() {
    // Math.random gives a float in [0,1); multiply to get a large integer.
    return Math.floor(Math.random() * 9007199254740991);
}

function startGame(seed) {
    if (acTimer) { clearInterval(acTimer); acTimer = null; }
    game = new SolitaireGame(seed, drawThree);
    snap = game.state();
    hudSeed.textContent = `seed ${Math.round(game.seed())}`;
    winOverlay.classList.add("hidden");
    cardEls.clear();
    board.querySelectorAll(".card").forEach(el => el.remove());
    render(snap);
}

// ── Slot placeholders ────────────────────────────────────────────────────────
function buildSlots() {
    for (const [pile, origin] of Object.entries(PILE_ORIGIN)) {
        const el = document.createElement("div");
        el.className = "slot";
        el.dataset.pile = pile;
        el.style.transform = `translate(${origin.x}px, ${origin.y}px)`;
        board.appendChild(el);
    }
}

// ── Card position math ────────────────────────────────────────────────────────
function cardPos(pileName, indexInPile, pileLength, pileCards) {
    const origin = PILE_ORIGIN[pileName];
    let x = origin.x;
    let y = origin.y;

    if (pileName === "waste" && drawThree && pileLength >= 2) {
        // Show top-3 of waste fanned horizontally.
        const fanStart = Math.max(0, pileLength - 3);
        const fanPos   = indexInPile - fanStart;
        if (fanPos >= 0) {
            x += fanPos * WASTE_FAN;
        } else {
            // Cards below the fan window are stacked at origin.
        }
    } else if (pileName.startsWith("tableau-")) {
        y += indexInPile * FAN;
    }
    // Stock, foundations: stack (no offset).
    return { x, y };
}

// Z-index: higher index in pile = drawn on top.
function cardZ(pileName, indexInPile, total) {
    if (pileName === "stock")          return 10 + indexInPile;
    if (pileName === "waste")          return 10 + indexInPile;
    if (pileName.startsWith("found"))  return 10 + indexInPile;
    return 10 + indexInPile;
}

// ── Renderer ─────────────────────────────────────────────────────────────────
function render(s) {
    snap = s;

    // Update HUD
    hudScore.textContent = `Score: ${s.score}`;
    hudMoves.textContent = `Moves: ${s.move_count}`;
    btnUndo.disabled = s.move_count === 0;

    // Collect all cards visible in this snapshot, keyed by id → {pile, idx}.
    const visible = new Map();
    const addPile = (pileName, cards) => {
        cards.forEach((c, i) => visible.set(c.id, { pile: pileName, idx: i, card: c, total: cards.length }));
    };
    addPile("stock", s.stock);
    addPile("waste", s.waste);
    s.foundations.forEach((f, i) => addPile(`foundation-${i}`, f));
    s.tableaus.forEach((t, i)    => addPile(`tableau-${i}`, t));

    // Create or update card elements.
    for (const [id, info] of visible) {
        let el = cardEls.get(id);
        if (!el) {
            el = createCardEl(info.card);
            cardEls.set(id, el);
            board.appendChild(el);
        }

        updateCardEl(el, info.card, info.pile, info.idx, info.total, s);
    }

    // Remove cards no longer in the snapshot (shouldn't happen in solitaire
    // but guards against stale state after undo).
    for (const [id, el] of cardEls) {
        if (!visible.has(id)) {
            el.remove();
            cardEls.delete(id);
        }
    }

    // Update slot drop-active highlights (cleared on every render).
    board.querySelectorAll(".slot.drop-active").forEach(el => el.classList.remove("drop-active"));
    board.querySelectorAll(".card.drop-target").forEach(el => el.classList.remove("drop-target"));

    // Show recycle indicator on empty stock.
    let recycleEl = board.querySelector(".recycle-label");
    if (s.stock.length === 0 && s.waste.length > 0) {
        if (!recycleEl) {
            recycleEl = document.createElement("div");
            recycleEl.className = "recycle-label";
            recycleEl.textContent = "↺";
            board.appendChild(recycleEl);
        }
        const o = PILE_ORIGIN.stock;
        recycleEl.style.left = `${o.x + CARD_W / 2}px`;
        recycleEl.style.top  = `${o.y + CARD_H / 2}px`;
    } else if (recycleEl) {
        recycleEl.remove();
    }

    // Trigger auto-complete if applicable.
    if (s.is_auto_completable && !s.is_won && !acTimer) {
        acTimer = setInterval(doAutoCompleteStep, 400);
    }
    if (s.is_won) {
        if (acTimer) { clearInterval(acTimer); acTimer = null; }
        showWin(s);
    }
}

function createCardEl(card) {
    const el = document.createElement("div");
    el.dataset.cardId = card.id;
    return el;
}

function updateCardEl(el, card, pileName, idx, total, s) {
    const pos = cardPos(pileName, idx, total, null);
    const z   = cardZ(pileName, idx, total);

    el.style.transform = `translate(${pos.x}px, ${pos.y}px)`;
    el.style.zIndex    = z;

    const isTop = idx === total - 1;

    if (!card.face_up) {
        el.className = "card face-down";
        if (pileName === "stock") el.classList.add("stock-card");
        el.innerHTML = "";
    } else {
        const isRed = RED_SUITS.has(card.suit);
        el.className = `card ${isRed ? "red" : "black"}`;
        if (pileName === "stock") el.classList.add("stock-card");

        const rankLabel = RANK_LABELS[card.rank];
        const suit      = SUIT_GLYPH[card.suit];
        el.innerHTML = `
            <div class="corner top">${rankLabel}<br>${suit}</div>
            <div class="center">${suit}</div>
            <div class="corner bottom">${rankLabel}<br>${suit}</div>`;
    }
}

// ── Win overlay ───────────────────────────────────────────────────────────────
function showWin(s) {
    winScore.textContent = `Score: ${s.score}`;
    winMoves.textContent = `${s.move_count} moves`;
    winOverlay.classList.remove("hidden");
}

// ── Auto-complete ─────────────────────────────────────────────────────────────
function doAutoCompleteStep() {
    if (!game || !snap || !snap.is_auto_completable) {
        clearInterval(acTimer);
        acTimer = null;
        return;
    }
    const result = game.auto_complete_step();
    if (result && result.ok) {
        render(result.snapshot);
    } else {
        clearInterval(acTimer);
        acTimer = null;
    }
}

// ── Input handling ────────────────────────────────────────────────────────────
function attachHandlers() {
    // Buttons
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

    // Keyboard shortcuts
    document.addEventListener("keydown", (e) => {
        if (e.key === "z" || e.key === "Z") {
            const r = game.undo();
            if (r.ok) render(r.snapshot);
        }
        if (e.key === "n" || e.key === "N") {
            startGame(randomSeed());
        }
    });

    // Board pointer events (handles both mouse and touch via PointerEvents API)
    board.addEventListener("pointerdown", onPointerDown);
    board.addEventListener("pointermove", onPointerMove);
    board.addEventListener("pointerup",   onPointerUp);
    board.addEventListener("pointercancel", onPointerCancel);
    board.addEventListener("click", onBoardClick);
    board.addEventListener("dblclick", onBoardDblClick);
}

// ── Coordinate helpers ────────────────────────────────────────────────────────
function boardRelative(clientX, clientY) {
    const rect = board.getBoundingClientRect();
    // Subtract board padding to get interior coordinates.
    return {
        x: clientX - rect.left - PAD,
        y: clientY - rect.top  - PAD,
    };
}

function hitTestCard(bx, by) {
    // Walk all visible piles, find the topmost card at (bx, by).
    // Returns { pileName, cardIndex, cardId } or null.
    const pileOrder = [
        "waste",
        "foundation-0","foundation-1","foundation-2","foundation-3",
        "tableau-0","tableau-1","tableau-2","tableau-3","tableau-4","tableau-5","tableau-6",
        "stock",
    ];

    let best = null;
    let bestZ = -1;

    for (const pileName of pileOrder) {
        const cards = getPileCards(pileName);
        if (!cards) continue;

        for (let i = 0; i < cards.length; i++) {
            const pos = cardPos(pileName, i, cards.length, cards);
            if (bx >= pos.x && bx <= pos.x + CARD_W &&
                by >= pos.y && by <= pos.y + CARD_H) {
                const z = cardZ(pileName, i, cards.length);
                if (z > bestZ) {
                    bestZ = z;
                    best  = { pileName, cardIndex: i, cardId: cards[i].id, card: cards[i] };
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
    if (pileName.startsWith("foundation-")) {
        const slot = parseInt(pileName.split("-")[1]);
        return snap.foundations[slot];
    }
    if (pileName.startsWith("tableau-")) {
        const col = parseInt(pileName.split("-")[1]);
        return snap.tableaus[col];
    }
    return null;
}

function hitTestSlot(bx, by) {
    // Returns the pile name whose slot rect contains (bx, by), favouring
    // tableau column slots over top-row slots when both overlap.
    for (const [pile, origin] of Object.entries(PILE_ORIGIN)) {
        if (bx >= origin.x && bx <= origin.x + CARD_W &&
            by >= origin.y && by <= origin.y + CARD_H) {
            return pile;
        }
    }
    return null;
}

// For a tableau pile, hit-test for drop: the drop zone extends to the last
// card's bottom edge (or the slot if empty).
function findDropTarget(bx, by) {
    // Check tableau columns first (they have tall hit areas).
    for (let c = 0; c < 7; c++) {
        const pile = `tableau-${c}`;
        const cards = snap.tableaus[c];
        const origin = PILE_ORIGIN[pile];
        // Top boundary: origin.y. Bottom boundary: last card bottom or empty slot.
        const bottomY = cards.length > 0
            ? origin.y + (cards.length - 1) * FAN + CARD_H
            : origin.y + CARD_H;
        if (bx >= origin.x && bx <= origin.x + CARD_W &&
            by >= origin.y && by <= bottomY) {
            return pile;
        }
    }
    // Foundation slots (top row).
    for (let s = 0; s < 4; s++) {
        const pile = `foundation-${s}`;
        const origin = PILE_ORIGIN[pile];
        if (bx >= origin.x && bx <= origin.x + CARD_W &&
            by >= origin.y && by <= origin.y + CARD_H) {
            return pile;
        }
    }
    return null;
}

// ── Pointer event handlers ────────────────────────────────────────────────────
function onPointerDown(e) {
    if (e.button !== 0 && e.pointerType === "mouse") return;
    if (drag) return; // ignore second finger

    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const hit = hitTestCard(bx, by);
    if (!hit) return;
    if (!hit.card.face_up) return; // can't drag face-down cards

    const cards = getPileCards(hit.pileName);
    if (!cards) return;

    // For tableau, allow dragging a run from any face-up card.
    // For waste/foundation, only the top card.
    let fromIndex = hit.cardIndex;
    if (!hit.pileName.startsWith("tableau-")) {
        fromIndex = cards.length - 1; // only top card
        if (hit.cardIndex !== fromIndex) return;
    }

    const draggedCards = cards.slice(fromIndex);
    if (draggedCards.some(c => !c.face_up)) return; // face-down in run — blocked

    const cardOriginPos = cardPos(hit.pileName, fromIndex, cards.length, cards);

    drag = {
        fromPile:  hit.pileName,
        fromIndex,
        cardIds:   draggedCards.map(c => c.id),
        startX: bx,
        startY: by,
        offsetX: bx - cardOriginPos.x,
        offsetY: by - cardOriginPos.y,
    };

    // Lift the dragged cards visually.
    drag.cardIds.forEach((id, i) => {
        const el = cardEls.get(id);
        if (el) {
            el.classList.add("selected");
            el.style.zIndex = 500 + i;
        }
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
        const basePos = cardPos(drag.fromPile, drag.fromIndex + i, (cards ? cards.length : 1), null);
        el.style.transform = `translate(${basePos.x + dx}px, ${basePos.y + dy}px)`;
    });

    // Highlight drop target.
    board.querySelectorAll(".slot.drop-active").forEach(el => el.classList.remove("drop-active"));
    board.querySelectorAll(".card.drop-target").forEach(el => el.classList.remove("drop-target"));
    const targetPile = findDropTarget(bx, by);
    if (targetPile) {
        const slotEl = board.querySelector(`.slot[data-pile="${targetPile}"]`);
        if (slotEl) slotEl.classList.add("drop-active");
        const targetCards = getPileCards(targetPile);
        if (targetCards && targetCards.length > 0) {
            const topCard = cardEls.get(targetCards[targetCards.length - 1].id);
            if (topCard) topCard.classList.add("drop-target");
        }
    }

    e.preventDefault();
}

function onPointerUp(e) {
    if (!drag) return;
    board.querySelectorAll(".slot.drop-active").forEach(el => el.classList.remove("drop-active"));
    board.querySelectorAll(".card.drop-target").forEach(el => el.classList.remove("drop-target"));

    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const targetPile = findDropTarget(bx, by);

    let moved = false;
    if (targetPile && targetPile !== drag.fromPile) {
        const count = drag.cardIds.length;
        const r = game.move_cards(drag.fromPile, targetPile, count);
        if (r.ok) {
            render(r.snapshot);
            moved = true;
        }
    }

    if (!moved) {
        // Snap cards back to their original positions.
        drag.cardIds.forEach(id => {
            const el = cardEls.get(id);
            if (el) el.classList.remove("selected");
        });
        render(snap); // re-render restores transforms
    }

    drag = null;
}

function onPointerCancel() {
    if (drag) {
        drag.cardIds.forEach(id => {
            const el = cardEls.get(id);
            if (el) el.classList.remove("selected");
        });
        render(snap);
        drag = null;
    }
}

// ── Click handlers ────────────────────────────────────────────────────────────
function onBoardClick(e) {
    if (drag) return; // swallowed by pointer-up

    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);

    // Stock click → draw.
    const stockOrigin = PILE_ORIGIN.stock;
    if (bx >= stockOrigin.x && bx <= stockOrigin.x + CARD_W &&
        by >= stockOrigin.y && by <= stockOrigin.y + CARD_H) {
        const r = game.draw();
        if (r.ok) render(r.snapshot);
        return;
    }
}

function onBoardDblClick(e) {
    const { x: bx, y: by } = boardRelative(e.clientX, e.clientY);
    const hit = hitTestCard(bx, by);
    if (!hit || !hit.card.face_up) return;

    // Only try to move the top card of its pile.
    const cards = getPileCards(hit.pileName);
    if (!cards || hit.cardIndex !== cards.length - 1) return;

    // Try each foundation slot.
    for (let s = 0; s < 4; s++) {
        const r = game.move_cards(hit.pileName, `foundation-${s}`, 1);
        if (r.ok) { render(r.snapshot); return; }
    }
}

// ── Start ─────────────────────────────────────────────────────────────────────
bootstrap().catch(console.error);

/* @ts-self-types="./solitaire_wasm.d.ts" */

/**
 * Browser-side replay state machine. Owns a live `GameState` and the
 * replay's move list; each `step()` applies the next move.
 */
export class ReplayPlayer {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ReplayPlayerFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_replayplayer_free(ptr, 0);
    }
    /**
     * Returns `true` once every move has been applied.
     * @returns {boolean}
     */
    is_finished() {
        const ret = wasm.replayplayer_is_finished(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Construct from a raw replay JSON string.
     * @param {string} replay_json
     */
    constructor(replay_json) {
        const ptr0 = passStringToWasm0(replay_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.replayplayer_new(ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        ReplayPlayerFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Snapshot the current `GameState` as a JS object (see `StateSnapshot`).
     * @returns {any}
     */
    state() {
        const ret = wasm.replayplayer_state(this.__wbg_ptr);
        return ret;
    }
    /**
     * Apply the next move; returns the post-step snapshot, or `null`
     * once the move list is exhausted.
     * @returns {any}
     */
    step() {
        const ret = wasm.replayplayer_step(this.__wbg_ptr);
        return ret;
    }
    /**
     * 0-indexed position of the next move to apply.
     * @returns {number}
     */
    step_idx() {
        const ret = wasm.replayplayer_step_idx(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Total number of moves the replay contains.
     * @returns {number}
     */
    total_steps() {
        const ret = wasm.replayplayer_total_steps(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) ReplayPlayer.prototype[Symbol.dispose] = ReplayPlayer.prototype.free;

/**
 * Interactive Klondike game backed by the real `solitaire_core` rules engine.
 *
 * Construct with `new(seed, draw_three)`, then call `draw()`, `move_cards()`,
 * `undo()`, `auto_complete_step()` to advance the game. `state()` returns the
 * full pile snapshot at any time without mutating state.
 */
export class SolitaireGame {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        SolitaireGameFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_solitairegame_free(ptr, 0);
    }
    /**
     * Apply one auto-complete move (only valid when `is_auto_completable`).
     *
     * If no card can go directly to a foundation this step, advances the
     * waste by calling `draw()` so the next step can try again. Returns the
     * post-move snapshot, or `null` when no progress is possible.
     * @returns {any}
     */
    auto_complete_step() {
        const ret = wasm.solitairegame_auto_complete_step(this.__wbg_ptr);
        return ret;
    }
    /**
     * Draw from stock to waste (or recycle waste → stock when stock is empty).
     * Returns `{ok, error?, snapshot?}`.
     * @returns {any}
     */
    draw() {
        const ret = wasm.solitairegame_draw(this.__wbg_ptr);
        return ret;
    }
    /**
     * Move `count` cards from pile `from` to pile `to`.
     *
     * Pile names: `"stock"`, `"waste"`, `"foundation-0"` .. `"foundation-3"`,
     * `"tableau-0"` .. `"tableau-6"`.
     *
     * Returns `{ok, error?, snapshot?}`.
     * @param {string} from
     * @param {string} to
     * @param {number} count
     * @returns {any}
     */
    move_cards(from, to, count) {
        const ptr0 = passStringToWasm0(from, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(to, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.solitairegame_move_cards(this.__wbg_ptr, ptr0, len0, ptr1, len1, count);
        return ret;
    }
    /**
     * Create a new DrawOne or DrawThree Classic game from the given seed.
     *
     * `seed` is a JS `number` (f64); values up to 2^53 are represented exactly.
     * Pass `Date.now()` or a random integer from JS for variety.
     * @param {number} seed
     * @param {boolean} draw_three
     */
    constructor(seed, draw_three) {
        const ret = wasm.solitairegame_new(seed, draw_three);
        this.__wbg_ptr = ret;
        SolitaireGameFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * The seed used to deal this game.
     * @returns {number}
     */
    seed() {
        const ret = wasm.solitairegame_seed(this.__wbg_ptr);
        return ret;
    }
    /**
     * Full pile snapshot as a JS object.
     * @returns {any}
     */
    state() {
        const ret = wasm.solitairegame_state(this.__wbg_ptr);
        return ret;
    }
    /**
     * Undo the last move. Returns `{ok, error?, snapshot?}`.
     * @returns {any}
     */
    undo() {
        const ret = wasm.solitairegame_undo(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) SolitaireGame.prototype[Symbol.dispose] = SolitaireGame.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Error_3639a60ed15f87e7: function(arg0, arg1) {
            const ret = Error(getStringFromWasm0(arg0, arg1));
            return ret;
        },
        __wbg___wbindgen_throw_9c75d47bf9e7731e: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_new_2fad8ca02fd00684: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_3baa8d9866155c79: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_set_6be42768c690e380: function(arg0, arg1, arg2) {
            arg0[arg1] = arg2;
        },
        __wbg_set_f614f6a0608d1d1d: function(arg0, arg1, arg2) {
            arg0[arg1 >>> 0] = arg2;
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbindgen_cast_0000000000000001: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000002: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000003: function(arg0) {
            // Cast intrinsic for `U64 -> Externref`.
            const ret = BigInt.asUintN(64, arg0);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./solitaire_wasm_bg.js": import0,
    };
}

const ReplayPlayerFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_replayplayer_free(ptr, 1));
const SolitaireGameFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_solitairegame_free(ptr, 1));

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('solitaire_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };

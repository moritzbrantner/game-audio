/* tslint:disable */
/* eslint-disable */

export class GameAudio {
    free(): void;
    [Symbol.dispose](): void;
    activeVoices(): number;
    /**
     * Applies one `settings` value by id (`audio.master_volume`, `audio.muted`, ...).
     * Numbers are 0..=1 unless `percent` is set. Returns false for ids it does not own.
     */
    applySetting(id: string, value: any, percent: boolean): boolean;
    /**
     * `{ master, effects, ui, muted }`, volumes in 0..=1; missing fields default.
     */
    applySettings(settings: any): void;
    forgetCueKeys(): void;
    /**
     * `manifest` is the sound manifest JSON text; `config` is an optional
     * `DirectorConfig`-shaped object; `base_url` prefixes relative clip URLs.
     */
    constructor(manifest: string, config: any, base_url?: string | null);
    /**
     * Frees finished voices and flushes queued commands. Every mutating method already
     * pumps; call this once per frame so ended voices free their slots promptly.
     */
    pump(): void;
    /**
     * Session replace / seek / reconnect. Seen cue keys are kept.
     */
    reset(): void;
    setEntityPosition(id: number, position: Float32Array): void;
    setListener(position: Float32Array, right: Float32Array): void;
    setSuspended(suspended: boolean): void;
    /**
     * `"running"`, `"suspended"` (locked or hidden) or `"unavailable"`.
     */
    state(): string;
    stopEntity(id: number): void;
    /**
     * Clip load errors since the last call.
     */
    takeErrors(): string[];
    /**
     * Sounds `{ key, kind, entity?, position?: [x, y, z] }` (extra fields ignored).
     * Returns the outcome name, e.g. `"started"` or `"duplicate"`.
     */
    trigger(cue: any, now_ms: number): string;
    /**
     * Call from a user gesture.
     */
    unlock(): void;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_gameaudio_free: (a: number, b: number) => void;
    readonly gameaudio_activeVoices: (a: number) => number;
    readonly gameaudio_applySetting: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly gameaudio_applySettings: (a: number, b: number, c: number) => void;
    readonly gameaudio_forgetCueKeys: (a: number) => void;
    readonly gameaudio_new: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly gameaudio_pump: (a: number) => void;
    readonly gameaudio_reset: (a: number) => void;
    readonly gameaudio_setEntityPosition: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly gameaudio_setListener: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly gameaudio_setSuspended: (a: number, b: number) => void;
    readonly gameaudio_state: (a: number, b: number) => void;
    readonly gameaudio_stopEntity: (a: number, b: number, c: number) => void;
    readonly gameaudio_takeErrors: (a: number, b: number) => void;
    readonly gameaudio_trigger: (a: number, b: number, c: number, d: number) => void;
    readonly gameaudio_unlock: (a: number) => void;
    readonly __wasm_bindgen_func_elem_846: (a: number, b: number, c: number, d: number) => void;
    readonly __wasm_bindgen_func_elem_335: (a: number, b: number) => void;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number) => void;
    readonly __wbindgen_export4: (a: number, b: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export5: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;

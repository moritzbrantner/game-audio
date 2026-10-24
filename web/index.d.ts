export type Vec3 = [number, number, number] | Float32Array;

export type AudioState = "running" | "suspended" | "unavailable";

export type TriggerOutcome =
  | "started"
  | "started-stealing"
  | "duplicate"
  | "unknown-cue"
  | "suspended"
  | "cooling-down"
  | "inaudible"
  | "rejected"
  | "unavailable";

/** A presentation cue. Extra fields (tick, order, target, ...) are ignored. */
export interface CueEvent {
  /** Stable gameplay identity; a key is sounded at most once. Empty keys are never deduplicated. */
  key: string;
  /** Cue kind from the manifest, e.g. "combat.hit". */
  kind: string;
  /** Entity the voice follows (non-negative safe integer). */
  entity?: number | null;
  position?: [number, number, number] | null;
}

export interface AudioSettings {
  master?: number;
  effects?: number;
  ui?: number;
  muted?: boolean;
}

export interface DirectorConfig {
  max_voices?: number;
  seed?: number;
  seen_key_capacity?: number;
  stop_fade_ms?: number;
  steal_fade_ms?: number;
  bus_fade_ms?: number;
}

export interface GameAudioOptions {
  /** Sound manifest object or JSON text (see the README). */
  manifest: unknown;
  config?: DirectorConfig;
  /** Prefix for relative clip URLs, e.g. `${import.meta.env.BASE_URL}sounds/`. */
  baseUrl?: string;
  /** Override the wasm location (URL, Response, bytes or module). */
  wasm?: unknown;
  /** Where first-gesture unlock listens. Default: window. */
  gestureTarget?: EventTarget;
  /** Whose `hidden`/`visibilitychange` suspends audio. Default: document. */
  document?: { hidden: boolean } & EventTarget;
  /** Clock for cooldowns, in ms. Default: performance.now(). */
  now?: () => number;
  onError?: (error: unknown) => void;
}

export interface GameAudioHandle {
  trigger(cue: CueEvent, nowMs?: number): TriggerOutcome;
  /** Listener position and the world direction of the listener's right ear. */
  setListener(position: Vec3, right: Vec3): void;
  setEntityPosition(entity: number, position: Vec3): void;
  /** Entity removed: fade out its voices. */
  stopEntity(entity: number): void;
  /** Session replace / seek / reconnect. Seen cue keys are kept. */
  reset(): void;
  forgetCueKeys(): void;
  applySettings(settings: AudioSettings): void;
  /** Applies one `settings` value by id, e.g. ("audio.master_volume", 80, true). */
  applySetting(id: string, value: number | boolean, percent?: boolean): boolean;
  setSuspended(suspended: boolean): void;
  unlock(): void;
  /** Call once per frame so finished voices free their slots. */
  pump(): void;
  state(): AudioState;
  activeVoices(): number;
  takeErrors(): string[];
  dispose(): void;
}

export function createGameAudio(options: GameAudioOptions): Promise<GameAudioHandle>;
export function wrapGameAudio(
  audio: unknown,
  options?: Partial<GameAudioOptions>,
): GameAudioHandle;

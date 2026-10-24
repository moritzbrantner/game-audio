// Browser glue for the game-audio wasm facade.
//
// - Loads the wasm module and creates one `GameAudio` (director + Web Audio backend).
// - Unlocks the AudioContext on the first pointer/key/touch gesture.
// - Suspends while the page is hidden and resumes when it is visible again.
// - Never throws into the game loop: if wasm or Web Audio is missing or blocked, every
//   call is a silent no-op and `state()` reports "unavailable".

const GESTURES = ["pointerdown", "keydown", "touchend"];

// One wasm instance per page. wasm-bindgen's init only short-circuits after instantiation
// finishes, so overlapping creates (React StrictMode) would instantiate twice and the second
// instance would replace the memory the first handle points into (issue #4). Every create
// shares this promise; the first caller's `wasm` option wins. A failed init is forgotten so
// a later create can retry.
let ready = null;

function loadBindings(wasm) {
  ready ??= (async () => {
    const bindings = await import("./pkg/game_audio_wasm.js");
    await bindings.default(wasm === undefined ? undefined : { module_or_path: wasm });
    return bindings;
  })().catch((error) => {
    ready = null;
    throw error;
  });
  return ready;
}

/**
 * Loads the wasm facade (once per page) and returns a ready audio handle. Concurrent and
 * repeated calls share one wasm instance; each returns its own independent handle.
 * @param {import("./index.d.ts").GameAudioOptions} options
 * @returns {Promise<import("./index.d.ts").GameAudioHandle>}
 */
export async function createGameAudio(options) {
  const { manifest, config, baseUrl, wasm } = options;
  let audio = null;
  try {
    const bindings = await loadBindings(wasm);
    const text = typeof manifest === "string" ? manifest : JSON.stringify(manifest);
    audio = new bindings.GameAudio(text, config ?? null, baseUrl ?? "");
  } catch (error) {
    (options.onError ?? console.warn)(error);
  }
  return wrapGameAudio(audio, options);
}

/**
 * Wraps an existing `GameAudio` (or `null` for silence) with gesture unlock, visibility
 * suspension and error isolation. Exposed for tests and custom loaders.
 * @param {any} audio
 * @param {Partial<import("./index.d.ts").GameAudioOptions>} [options]
 * @returns {import("./index.d.ts").GameAudioHandle}
 */
export function wrapGameAudio(audio, options = {}) {
  const gestureTarget = options.gestureTarget ?? globalThis.window;
  const visibilityTarget = options.document ?? globalThis.document;
  const now = options.now ?? (() => globalThis.performance?.now() ?? Date.now());
  const onError = options.onError ?? console.warn;
  let disposed = false;

  const guard = (fallback, run) => {
    if (!audio || disposed) return fallback;
    try {
      return run(audio);
    } catch (error) {
      onError(error);
      return fallback;
    }
  };

  const unlock = () => {
    guard(undefined, (a) => a.unlock());
    if (guard("unavailable", (a) => a.state()) !== "suspended") removeGestures();
  };
  // Some browsers resolve resume() asynchronously; keep listening until running.
  const onGesture = () => unlock();
  const addGestures = () => {
    for (const type of GESTURES) gestureTarget?.addEventListener?.(type, onGesture, true);
  };
  const removeGestures = () => {
    for (const type of GESTURES) gestureTarget?.removeEventListener?.(type, onGesture, true);
  };
  const onVisibility = () => guard(undefined, (a) => a.setSuspended(Boolean(visibilityTarget.hidden)));

  if (audio) {
    addGestures();
    visibilityTarget?.addEventListener?.("visibilitychange", onVisibility);
    if (visibilityTarget?.hidden) onVisibility();
  }

  return {
    trigger: (cue, nowMs = now()) => guard("unavailable", (a) => a.trigger(cue, nowMs)),
    setListener: (position, right) => guard(undefined, (a) => a.setListener(position, right)),
    setEntityPosition: (entity, position) =>
      guard(undefined, (a) => a.setEntityPosition(entity, position)),
    stopEntity: (entity) => guard(undefined, (a) => a.stopEntity(entity)),
    reset: () => guard(undefined, (a) => a.reset()),
    forgetCueKeys: () => guard(undefined, (a) => a.forgetCueKeys()),
    applySettings: (settings) => guard(undefined, (a) => a.applySettings(settings)),
    applySetting: (id, value, percent = false) =>
      guard(false, (a) => a.applySetting(id, value, percent)),
    setSuspended: (suspended) => guard(undefined, (a) => a.setSuspended(suspended)),
    unlock,
    pump: () => guard(undefined, (a) => a.pump()),
    state: () => guard("unavailable", (a) => a.state()),
    activeVoices: () => guard(0, (a) => a.activeVoices()),
    takeErrors: () => guard([], (a) => a.takeErrors()),
    dispose() {
      if (disposed) return;
      removeGestures();
      visibilityTarget?.removeEventListener?.("visibilitychange", onVisibility);
      guard(undefined, (a) => {
        a.reset();
        a.pump();
        a.free?.();
      });
      disposed = true;
    },
  };
}

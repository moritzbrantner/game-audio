// Runs against the real wasm build (scripts/build-web.sh writes web/pkg). Bun has no Web
// Audio, so the backend is silent, but the director still runs inside wasm memory.
import { expect, test } from "bun:test";
import { existsSync } from "node:fs";

import { createGameAudio } from "./index.js";

const built = existsSync(new URL("./pkg/game_audio_wasm_bg.wasm", import.meta.url));

const manifest = {
  clips: [{ id: "c", source: { tone: { frequency_hz: 440, duration_ms: 10 } } }],
  cues: [{ kind: "hit", clips: ["c"], max_voices: 8 }],
};
const options = () => ({
  manifest,
  gestureTarget: new EventTarget(),
  document: Object.assign(new EventTarget(), { hidden: false }),
  onError: (error) => {
    throw error;
  },
});

test.skipIf(!built)("overlapping creates share one wasm instance (issue #4)", async () => {
  const [first, second] = await Promise.all([
    createGameAudio(options()),
    createGameAudio(options()),
  ]);
  expect(first.state()).toBe("unavailable");
  for (let i = 0; i < 50; i++) {
    expect(first.trigger({ key: `a${i}`, kind: "hit" }, i)).not.toBe("unavailable");
    expect(second.trigger({ key: `b${i}`, kind: "hit" }, i)).not.toBe("unavailable");
  }
  // Handles are independent: keys seen by one are new to the other.
  expect(second.trigger({ key: "a0", kind: "hit" }, 100)).toBe("started");
  expect(first.trigger({ key: "a0", kind: "hit" }, 100)).toBe("duplicate");

  // Disposing one (StrictMode's first mount) leaves the other working.
  first.dispose();
  expect(first.trigger({ key: "x", kind: "hit" })).toBe("unavailable");
  second.pump();
  expect(second.trigger({ key: "after-dispose", kind: "hit" }, 200)).toBe("started");
  second.applySettings({ master: 0.5 });
  second.dispose();

  const third = await createGameAudio(options());
  expect(third.trigger({ key: "third", kind: "hit" }, 0)).toBe("started");
  third.dispose();
});

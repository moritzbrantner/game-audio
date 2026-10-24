import { describe, expect, test } from "bun:test";

import { wrapGameAudio } from "./index.js";

class FakeAudio {
  calls = [];
  current = "suspended";
  unlock() {
    this.calls.push(["unlock"]);
    this.current = "running";
  }
  state() {
    return this.current;
  }
  trigger(cue, now) {
    this.calls.push(["trigger", cue.key, now]);
    return "started";
  }
  setSuspended(suspended) {
    this.calls.push(["setSuspended", suspended]);
  }
  reset() {
    this.calls.push(["reset"]);
  }
  pump() {}
  free() {
    this.calls.push(["free"]);
  }
}

const fakeDocument = (hidden = false) => Object.assign(new EventTarget(), { hidden });

describe("wrapGameAudio", () => {
  test("first gesture unlocks once, then stops listening", () => {
    const audio = new FakeAudio();
    const gestureTarget = new EventTarget();
    wrapGameAudio(audio, { gestureTarget, document: fakeDocument() });
    gestureTarget.dispatchEvent(new Event("pointerdown"));
    gestureTarget.dispatchEvent(new Event("keydown"));
    expect(audio.calls.filter(([name]) => name === "unlock")).toHaveLength(1);
  });

  test("visibility suspends and resumes", () => {
    const audio = new FakeAudio();
    const document = fakeDocument(true);
    wrapGameAudio(audio, { gestureTarget: new EventTarget(), document });
    document.hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(audio.calls).toEqual([
      ["setSuspended", true],
      ["setSuspended", false],
    ]);
  });

  test("trigger uses the injected clock", () => {
    const audio = new FakeAudio();
    const handle = wrapGameAudio(audio, {
      gestureTarget: new EventTarget(),
      document: fakeDocument(),
      now: () => 42,
    });
    expect(handle.trigger({ key: "k", kind: "hit" })).toBe("started");
    expect(audio.calls).toEqual([["trigger", "k", 42]]);
  });

  test("missing audio is a silent no-op", () => {
    const handle = wrapGameAudio(null, { gestureTarget: new EventTarget(), document: fakeDocument() });
    expect(handle.state()).toBe("unavailable");
    expect(handle.trigger({ key: "k", kind: "hit" })).toBe("unavailable");
    expect(handle.activeVoices()).toBe(0);
    handle.setListener([0, 0, 0], [1, 0, 0]);
    handle.dispose();
  });

  test("errors from the facade never reach the game loop", () => {
    const errors = [];
    const audio = new FakeAudio();
    audio.trigger = () => {
      throw new Error("bad cue");
    };
    const handle = wrapGameAudio(audio, {
      gestureTarget: new EventTarget(),
      document: fakeDocument(),
      onError: (error) => errors.push(error.message),
    });
    expect(handle.trigger({ key: "k", kind: "hit" })).toBe("unavailable");
    expect(errors).toEqual(["bad cue"]);
  });

  test("dispose stops voices, frees and detaches", () => {
    const audio = new FakeAudio();
    const gestureTarget = new EventTarget();
    const handle = wrapGameAudio(audio, { gestureTarget, document: fakeDocument() });
    handle.dispose();
    gestureTarget.dispatchEvent(new Event("pointerdown"));
    expect(audio.calls).toEqual([["reset"], ["free"]]);
    expect(handle.trigger({ key: "k", kind: "hit" })).toBe("unavailable");
  });
});

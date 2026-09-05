let loaded = false;
let enabled = false;

export function load(context) {
  if (typeof context.pluginId !== "bigint" || !Object.isFrozen(context)) {
    throw new Error("invalid Aura context");
  }
  loaded = true;
}

export function enable() {
  if (!loaded || enabled) throw new Error("invalid lifecycle");
  enabled = true;
}

export function invoke(operation, input) {
  if (!enabled || !(input instanceof Map)) {
    throw new Error("invalid callback invocation");
  }
  // Observe only. Never modify the request or retain invocation-local handles.
  if (operation === "hook.before-game-launch") {
    return new Map([["contractVersion", 1n], ["action", "unchanged"]]);
  }
  if (operation === "aura.patch.v1") {
    return new Map([["schemaVersion", 1n], ["action", "unchanged"]]);
  }
  throw new Error("unsupported callback operation");
}

export function disable() {
  if (!enabled) throw new Error("invalid lifecycle");
  enabled = false;
}

export function unload() {
  if (!loaded || enabled) throw new Error("invalid lifecycle");
  loaded = false;
}

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
  if (!enabled || operation !== "before-game-launch" || !(input instanceof Map)) {
    throw new Error("unsupported Hook invocation");
  }
  if (input.has("workingDirectory")) {
    input.set("workingDirectory", input.get("workingDirectory"));
  }
  return input;
}

export function disable() {
  if (!enabled) throw new Error("invalid lifecycle");
  enabled = false;
}

export function unload() {
  if (!loaded || enabled) throw new Error("invalid lifecycle");
  loaded = false;
}

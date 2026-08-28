export function load() {}
export function enable() {
  console.log("must never reach process stdout");
}
export function invoke(_operation, input) { return input; }
export function disable() {}
export function unload() {}

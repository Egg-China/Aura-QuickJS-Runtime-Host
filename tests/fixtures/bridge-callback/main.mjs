import { bridge } from "aura:runtime";

export function load() {}
export function enable() {}
export async function invoke(_operation, input) {
  return await bridge.invoke("launcher.test.echo", input);
}
export function disable() {}
export function unload() {}

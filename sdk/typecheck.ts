import type { AuraPluginContext, AuraValue } from "aura:runtime";

let loadedContext: AuraPluginContext | null = null;

export async function load(context: AuraPluginContext): Promise<void> {
  loadedContext = context;
}

export async function enable(): Promise<void> {
  if (loadedContext === null) throw new Error("not loaded");
}

export async function invoke(
  operation: string,
  input: AuraValue,
  callbackId: bigint,
): Promise<AuraValue> {
  void operation;
  void callbackId;
  return input;
}

export async function disable(): Promise<void> {}

export async function unload(): Promise<void> {
  loadedContext = null;
}

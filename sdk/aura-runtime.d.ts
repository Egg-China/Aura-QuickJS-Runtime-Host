/** Values accepted by Aura Bridge Value v1. */
export type AuraValue =
  | null
  | boolean
  | bigint
  | number
  | string
  | Uint8Array
  | AuraValue[]
  | Map<string, AuraValue>
  | AuraHandle
  | AuraError;

/** Immutable reference to one host-managed object. */
export declare class AuraHandle {
  private constructor();
  readonly objectId: bigint;
  readonly generation: bigint;
  readonly typeName: string;
}

/** Stable, redacted Bridge failure. */
export declare class AuraError {
  private constructor();
  readonly code:
    | "invalid-argument"
    | "invalid-result"
    | "permission-denied"
    | "stale-handle"
    | "type-mismatch"
    | "cancelled"
    | "callback-failed"
    | "unavailable"
    | "internal";
}

/** Bridge callbacks reauthorized by Aura for the active payload. */
export interface AuraBridge {
  invoke(operation: string, input: AuraValue): Promise<AuraValue>;
  retain(handle: AuraHandle): Promise<void>;
  release(handle: AuraHandle): Promise<void>;
}

/** Immutable context passed only to the payload's load function. */
export interface AuraPluginContext {
  readonly pluginId: bigint;
  readonly bridge: AuraBridge;
}

/** Bridge singleton exported by the built-in aura:runtime module. */
export declare const bridge: AuraBridge;

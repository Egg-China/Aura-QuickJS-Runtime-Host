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

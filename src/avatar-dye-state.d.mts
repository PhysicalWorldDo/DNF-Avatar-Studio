export type DyeRgb = readonly [number, number, number];

export type AvatarDyeEdit = Readonly<{
  targetId: string;
  paletteIndex: number;
  colorIndex: number;
  replacementRgb: DyeRgb;
}>;

export type AvatarDyeState = Readonly<{
  edits: readonly AvatarDyeEdit[];
  persistedEdits: readonly AvatarDyeEdit[];
  hasDye: boolean;
  isDirty: boolean;
  revision: number;
  undoStack: readonly (readonly AvatarDyeEdit[])[];
  redoStack: readonly (readonly AvatarDyeEdit[])[];
  selectedPaletteIndices: readonly number[];
  paletteSelectionAnchor: number | null;
  latestRequestId: number;
  latestAcceptedRequestId: number;
}>;

export type DyeRequestIssue = Readonly<{
  requestId: number;
  state: AvatarDyeState;
}>;

export type DyeResponseAcceptance = Readonly<{
  accepted: boolean;
  state: AvatarDyeState;
}>;

export type DyeCommandPayload = Readonly<{
  sessionId: string;
  targetId: string;
  requestId: number;
  revision: number;
  operation: Readonly<{
    kind: "replace_rgb";
    paletteIndex: number;
    colorIndices: readonly number[];
    replacementRgb: DyeRgb;
  }>;
}>;

export function createAvatarDyeState(options?: {
  edits?: readonly AvatarDyeEdit[];
}): AvatarDyeState;

export function commitDyeEdits(
  state: AvatarDyeState,
  edits: readonly AvatarDyeEdit[],
): AvatarDyeState;

export function markDyePersisted(state: AvatarDyeState): AvatarDyeState;

export function resetDyeEdits(state: AvatarDyeState): AvatarDyeState;

export function undoDyeEdit(state: AvatarDyeState): AvatarDyeState;

export function redoDyeEdit(state: AvatarDyeState): AvatarDyeState;

export function selectPaletteIndex(
  state: AvatarDyeState,
  index: number,
  options?: {
    ctrlKey?: boolean;
    shiftKey?: boolean;
    maxIndex?: number;
  },
): AvatarDyeState;

export function issueDyeRequest(state: AvatarDyeState): DyeRequestIssue;

export function acceptDyeResponse(
  state: AvatarDyeState,
  requestId: number,
): DyeResponseAcceptance;

export function buildDyeCommandPayload(input: {
  sessionId: string;
  targetId: string;
  requestId: number;
  revision: number;
  paletteIndex: number;
  colorIndices: readonly number[];
  replacementRgb: DyeRgb;
}): DyeCommandPayload;

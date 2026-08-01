export type DyeObjectMode = "all" | "part" | "img";
export type DyeRgb = [number, number, number];

export type DyeAdjustment = {
  enabled: boolean;
  targetColor: DyeRgb;
  strength: number;
  brightness: number;
  hue: number;
  saturation: number;
  lightness: number;
  contrast: number;
  preserveLuminance: boolean;
};

export type DyeInteractionState = {
  objectMode: DyeObjectMode;
  selectedParts: string[];
  selectedTargetId: string;
  bakedLegacy: boolean;
  adjustments: {
    all: DyeAdjustment;
    parts: Record<string, DyeAdjustment>;
    images: Record<string, DyeAdjustment>;
  };
  exactOverrides: Record<string, Record<string, DyeRgb>>;
};

export type DyeAdjustmentTarget = {
  opaque_target_id: string;
  canonical_target_id?: string;
  slot?: string;
  active_palette: number;
  editable?: boolean;
  capability?: string;
  resource_status?: string;
  preview_available?: boolean;
  palettes?: Array<{
    palette_index: number;
    colors: Array<{
      color_index: number;
      original_rgba: [number, number, number, number];
      baseline_rgba?: [number, number, number, number];
      current_rgba: [number, number, number, number];
    }>;
  }>;
};

export function defaultDyeAdjustment(): DyeAdjustment;
export function createDyeInteractionState(options?: Partial<DyeInteractionState>): DyeInteractionState;
export function setDyeObjectMode(state: DyeInteractionState, mode: DyeObjectMode): DyeInteractionState;
export function toggleDyePart(state: DyeInteractionState, slot: string): DyeInteractionState;
export function selectDyeImage(state: DyeInteractionState, targetId: string): DyeInteractionState;
export function canonicalDyeTargetId(target: DyeAdjustmentTarget): string;
export function isTargetEditable(target: DyeAdjustmentTarget): boolean;
export function selectedDyeTargets<T extends DyeAdjustmentTarget>(state: DyeInteractionState, targets: readonly T[]): T[];
export function setDyeAdjustmentForSelection(
  state: DyeInteractionState,
  targets: readonly DyeAdjustmentTarget[],
  patch: Partial<DyeAdjustment>,
): DyeInteractionState;
export function dyeAdjustmentForSelection(
  state: DyeInteractionState,
  targets: readonly DyeAdjustmentTarget[],
): DyeAdjustment;
export function mixedDyeAdjustmentKeys(
  state: DyeInteractionState,
  targets: readonly DyeAdjustmentTarget[],
): string[];
export function applyDyeAdjustmentStack(sourceRgb: DyeRgb, adjustments: readonly DyeAdjustment[]): DyeRgb;
export function setExactDyeOverride(state: DyeInteractionState, canonicalTargetId: string, colorIndex: number, rgb: DyeRgb): DyeInteractionState;
export function clearExactDyeOverride(state: DyeInteractionState, canonicalTargetId: string, colorIndex: number): DyeInteractionState;
export function restoreDyeSemanticState(state: DyeInteractionState, semanticState: unknown): DyeInteractionState;
export function buildDyeBatchMutation(state: DyeInteractionState, targets: readonly DyeAdjustmentTarget[]): unknown;
export function serializeDyeSemanticState(state: DyeInteractionState, targets: readonly DyeAdjustmentTarget[]): unknown;

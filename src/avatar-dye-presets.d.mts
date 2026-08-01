import type {
  DyeAdjustmentTarget,
  DyeInteractionState,
  DyeRgb,
} from "./avatar-dye-adjustments.mjs";

export type DyeColorPreset = {
  id: string;
  number?: number;
  name: string;
  rgb: DyeRgb;
  createdAtMs?: number;
  updatedAtMs?: number;
};

export type DyeColorPresetCatalog = {
  schemaVersion: number;
  revision: number;
  builtinPresets: DyeColorPreset[];
  customPresets: DyeColorPreset[];
  storageError: string | null;
};

export type DyeColorPresetMatch = {
  source: "builtin" | "custom";
  preset: DyeColorPreset;
};

export type DyeRgbEncoding = "argb1555" | "argb4444" | "argb8888" | null;

export type ExactDyeColorPresetSelection = {
  canonicalTargetId: string;
  colorIndex: number;
  currentRgb?: readonly number[];
  rgbEncoding?: DyeRgbEncoding;
};

export function emptyDyeColorPresetCatalog(): DyeColorPresetCatalog;
export function normalizeDyeColorPresetCatalog(value: unknown): DyeColorPresetCatalog;
export function findMatchingDyeColorPreset(
  catalog: DyeColorPresetCatalog,
  rgb: readonly number[],
  rgbEncoding?: DyeRgbEncoding,
): DyeColorPresetMatch | null;
export function dyeColorPresetMatchLabel(
  catalog: DyeColorPresetCatalog,
  rgb: readonly number[],
  mixed?: boolean,
  rgbEncoding?: DyeRgbEncoding,
): string;
export function quantizeDyeRgbForEncoding(
  rgb: readonly number[],
  rgbEncoding?: DyeRgbEncoding,
): DyeRgb;
export function findDyeColorPresetById(
  catalog: DyeColorPresetCatalog,
  presetId: string,
): DyeColorPresetMatch | null;
export function applyDyeColorPreset(
  state: DyeInteractionState,
  targets: readonly DyeAdjustmentTarget[],
  rgb: readonly number[],
  exactSelection?: ExactDyeColorPresetSelection | null,
): DyeInteractionState;

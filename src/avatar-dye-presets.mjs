import {
  setDyeAdjustmentForSelection,
  setExactDyeOverride,
} from "./avatar-dye-adjustments.mjs";

function normalizedRgb(value) {
  if (!Array.isArray(value) || value.length !== 3) {
    throw new TypeError("RGB must contain exactly three channels");
  }
  return value.map((channel) => {
    if (typeof channel !== "number" || !Number.isFinite(channel)) {
      throw new TypeError("RGB channels must be finite numbers");
    }
    return Math.max(0, Math.min(255, Math.round(channel)));
  });
}

export function quantizeDyeRgbForEncoding(value, rgbEncoding = null) {
  const rgb = normalizedRgb(value);
  if (rgbEncoding === "argb1555") {
    return rgb.map((channel) => {
      const value5 = channel >> 3;
      return (value5 << 3) | (value5 >> 2);
    });
  }
  if (rgbEncoding === "argb4444") {
    return rgb.map((channel) => channel & 0xf0);
  }
  return rgb;
}

function normalizePreset(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("dye color preset must be an object");
  }
  const id = typeof value.id === "string" ? value.id.trim() : "";
  const name = typeof value.name === "string" ? value.name.trim() : "";
  if (!id || !name) throw new TypeError("dye color preset id and name are required");
  return {
    ...value,
    id,
    name,
    rgb: normalizedRgb(value.rgb),
  };
}

export function emptyDyeColorPresetCatalog() {
  return {
    schemaVersion: 1,
    revision: 0,
    builtinPresets: [],
    customPresets: [],
    storageError: null,
  };
}

export function normalizeDyeColorPresetCatalog(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("dye color preset catalog must be an object");
  }
  const schemaVersion = Number(value.schemaVersion);
  const revision = Number(value.revision);
  if (!Number.isInteger(schemaVersion) || schemaVersion < 1) {
    throw new TypeError("invalid dye color preset schema version");
  }
  if (!Number.isInteger(revision) || revision < 0) {
    throw new TypeError("invalid dye color preset revision");
  }
  return {
    schemaVersion,
    revision,
    builtinPresets: (Array.isArray(value.builtinPresets) ? value.builtinPresets : []).map(normalizePreset),
    customPresets: (Array.isArray(value.customPresets) ? value.customPresets : []).map(normalizePreset),
    storageError: typeof value.storageError === "string" && value.storageError.trim()
      ? value.storageError.trim()
      : null,
  };
}

function sameRgb(left, right) {
  return left[0] === right[0] && left[1] === right[1] && left[2] === right[2];
}

export function findMatchingDyeColorPreset(catalog, rgb, rgbEncoding = null) {
  const normalized = normalizedRgb(rgb);
  const matches = (preset) => sameRgb(
    quantizeDyeRgbForEncoding(preset.rgb, rgbEncoding),
    normalized,
  );
  const builtin = catalog.builtinPresets.find(matches);
  if (builtin) return { source: "builtin", preset: builtin };
  const custom = catalog.customPresets.find(matches);
  return custom ? { source: "custom", preset: custom } : null;
}

export function dyeColorPresetMatchLabel(catalog, rgb, mixed = false, rgbEncoding = null) {
  if (mixed) return "多种颜色";
  const match = findMatchingDyeColorPreset(catalog, rgb, rgbEncoding);
  if (!match) return "自定义颜色";
  return `${match.source === "builtin" ? "基础预设" : "我的预设"}：${match.preset.name}`;
}

export function findDyeColorPresetById(catalog, presetId) {
  const builtin = catalog.builtinPresets.find((preset) => preset.id === presetId);
  if (builtin) return { source: "builtin", preset: builtin };
  const custom = catalog.customPresets.find((preset) => preset.id === presetId);
  return custom ? { source: "custom", preset: custom } : null;
}

export function applyDyeColorPreset(state, targets, rgb, exactSelection = null) {
  const normalized = normalizedRgb(rgb);
  if (exactSelection) {
    const encoded = quantizeDyeRgbForEncoding(normalized, exactSelection.rgbEncoding);
    const current = state.exactOverrides[exactSelection.canonicalTargetId]?.[String(exactSelection.colorIndex)];
    const effective = current ?? (exactSelection.currentRgb ? normalizedRgb(exactSelection.currentRgb) : null);
    if (effective && sameRgb(effective, encoded)) return state;
    return setExactDyeOverride(
      state,
      exactSelection.canonicalTargetId,
      exactSelection.colorIndex,
      encoded,
    );
  }
  const next = setDyeAdjustmentForSelection(state, targets, {
    targetColor: normalized,
    enabled: true,
  });
  return JSON.stringify(next) === JSON.stringify(state) ? state : next;
}

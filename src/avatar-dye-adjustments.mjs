const OBJECT_MODES = new Set(["all", "part", "img"]);
const ADJUSTMENT_KEYS = [
  "enabled",
  "targetColor",
  "strength",
  "brightness",
  "hue",
  "saturation",
  "lightness",
  "contrast",
  "preserveLuminance",
];

function clamp(value, minimum, maximum) {
  return Math.max(minimum, Math.min(maximum, value));
}

function channel(value) {
  return clamp(Math.round(Number(value) || 0), 0, 255);
}

function normalizedRgb(value) {
  if (!Array.isArray(value) || value.length !== 3) {
    throw new TypeError("RGB must contain exactly three channels");
  }
  return value.map(channel);
}

export function defaultDyeAdjustment() {
  return {
    enabled: false,
    targetColor: [45, 212, 191],
    strength: 100,
    brightness: 0,
    hue: 0,
    saturation: 0,
    lightness: 0,
    contrast: 0,
    preserveLuminance: true,
  };
}

function normalizedAdjustmentNumber(value, fallback, minimum, maximum) {
  return typeof value === "number" && Number.isFinite(value)
    ? clamp(Math.round(value), minimum, maximum)
    : fallback;
}

function normalizedAdjustmentRgb(value, fallback) {
  return Array.isArray(value)
    && value.length === 3
    && value.every((channelValue) => typeof channelValue === "number" && Number.isFinite(channelValue))
    ? normalizedRgb(value)
    : [...fallback];
}

function cloneAdjustment(value = defaultDyeAdjustment()) {
  const defaults = defaultDyeAdjustment();
  const source = value && typeof value === "object" && !Array.isArray(value) ? value : {};
  return {
    enabled: typeof source.enabled === "boolean" ? source.enabled : defaults.enabled,
    targetColor: normalizedAdjustmentRgb(source.targetColor, defaults.targetColor),
    strength: normalizedAdjustmentNumber(source.strength, defaults.strength, 0, 100),
    brightness: normalizedAdjustmentNumber(source.brightness, defaults.brightness, -100, 100),
    hue: normalizedAdjustmentNumber(source.hue, defaults.hue, -180, 180),
    saturation: normalizedAdjustmentNumber(source.saturation, defaults.saturation, -100, 100),
    lightness: normalizedAdjustmentNumber(source.lightness, defaults.lightness, -100, 100),
    contrast: normalizedAdjustmentNumber(source.contrast, defaults.contrast, -100, 100),
    preserveLuminance: typeof source.preserveLuminance === "boolean"
      ? source.preserveLuminance
      : defaults.preserveLuminance,
  };
}

function cloneState(state) {
  return {
    ...state,
    bakedLegacy: state.bakedLegacy === true,
    selectedParts: [...state.selectedParts],
    adjustments: {
      all: cloneAdjustment(state.adjustments.all),
      parts: Object.fromEntries(Object.entries(state.adjustments.parts).map(([key, value]) => [key, cloneAdjustment(value)])),
      images: Object.fromEntries(Object.entries(state.adjustments.images).map(([key, value]) => [key, cloneAdjustment(value)])),
    },
    exactOverrides: Object.fromEntries(Object.entries(state.exactOverrides).map(([key, values]) => [
      key,
      Object.fromEntries(Object.entries(values).map(([index, rgb]) => [index, normalizedRgb(rgb)])),
    ])),
  };
}

export function createDyeInteractionState(options = {}) {
  return cloneState({
    objectMode: OBJECT_MODES.has(options.objectMode) ? options.objectMode : "all",
    selectedParts: [...new Set(options.selectedParts ?? [])].sort(),
    selectedTargetId: typeof options.selectedTargetId === "string" ? options.selectedTargetId : "",
    bakedLegacy: options.bakedLegacy === true,
    adjustments: {
      all: cloneAdjustment(options.adjustments?.all),
      parts: options.adjustments?.parts ?? {},
      images: options.adjustments?.images ?? {},
    },
    exactOverrides: options.exactOverrides ?? {},
  });
}

export function setDyeObjectMode(state, objectMode) {
  if (!OBJECT_MODES.has(objectMode)) throw new RangeError("unknown dye object mode");
  return { ...state, objectMode };
}

export function toggleDyePart(state, slot) {
  const normalized = String(slot).trim().toLowerCase();
  if (!normalized) return state;
  const next = cloneState(state);
  const parts = new Set(state.selectedParts);
  if (parts.has(normalized)) parts.delete(normalized);
  else parts.add(normalized);
  next.selectedParts = [...parts].sort();
  return next;
}

export function selectDyeImage(state, targetId) {
  return { ...state, selectedTargetId: String(targetId || "") };
}

function targetCanonicalKey(target) {
  return target.canonical_target_id || `${target.opaque_target_id}:${target.active_palette ?? 0}`;
}

export function canonicalDyeTargetId(target) {
  return targetCanonicalKey(target);
}

export function isTargetEditable(target) {
  if (typeof target?.editable === "boolean") return target.editable;
  if (!target || target.preview_available === false || !target.palettes?.length) return false;
  const resourceStatus = String(target.resource_status ?? "valid")
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "_");
  if (resourceStatus !== "valid") return false;
  const capability = String(target.capability ?? "")
    .trim()
    .toLowerCase()
    .replaceAll("_", "");
  return !["readonly", "unsupported", "missing", "conflict"].some((token) => capability.includes(token));
}

export function selectedDyeTargets(state, targets) {
  const selected = targets.filter((target) => {
    if (!isTargetEditable(target)) return false;
    if (state.objectMode === "all") return true;
    if (state.objectMode === "part") return state.selectedParts.includes(String(target.slot).toLowerCase());
    return target.opaque_target_id === state.selectedTargetId;
  });
  const canonical = new Map();
  for (const target of selected) {
    const key = targetCanonicalKey(target);
    if (!canonical.has(key)) canonical.set(key, target);
  }
  return [...canonical.values()];
}

function mergeAdjustment(current, patch) {
  const next = { ...cloneAdjustment(current), ...patch };
  if (Object.hasOwn(patch, "targetColor")) next.targetColor = normalizedRgb(patch.targetColor);
  next.strength = clamp(Math.round(next.strength), 0, 100);
  next.brightness = clamp(Math.round(next.brightness), -100, 100);
  next.hue = clamp(Math.round(next.hue), -180, 180);
  next.saturation = clamp(Math.round(next.saturation), -100, 100);
  next.lightness = clamp(Math.round(next.lightness), -100, 100);
  next.contrast = clamp(Math.round(next.contrast), -100, 100);
  next.enabled = next.enabled === true;
  next.preserveLuminance = next.preserveLuminance !== false;
  return next;
}

export function setDyeAdjustmentForSelection(state, targets, patch) {
  const next = cloneState(state);
  if (state.objectMode === "all") {
    next.adjustments.all = mergeAdjustment(next.adjustments.all, patch);
  } else if (state.objectMode === "part") {
    for (const target of selectedDyeTargets(state, targets)) {
      const key = targetCanonicalKey(target);
      next.adjustments.parts[key] = mergeAdjustment(next.adjustments.parts[key], patch);
    }
  } else {
    for (const target of selectedDyeTargets(state, targets)) {
      const key = targetCanonicalKey(target);
      next.adjustments.images[key] = mergeAdjustment(next.adjustments.images[key], patch);
    }
  }
  return next;
}

function valuesEqual(left, right) {
  return Array.isArray(left) || Array.isArray(right)
    ? JSON.stringify(left) === JSON.stringify(right)
    : left === right;
}

function selectionAdjustments(state, targets) {
  if (state.objectMode === "all") return [cloneAdjustment(state.adjustments.all)];
  return selectedDyeTargets(state, targets).map((target) => {
    const key = targetCanonicalKey(target);
    return cloneAdjustment(
      state.objectMode === "part"
        ? state.adjustments.parts[key]
        : state.adjustments.images[key],
    );
  });
}

export function dyeAdjustmentForSelection(state, targets) {
  return selectionAdjustments(state, targets)[0] ?? defaultDyeAdjustment();
}

export function mixedDyeAdjustmentKeys(state, targets) {
  if (state.objectMode !== "part") return [];
  const values = selectionAdjustments(state, targets);
  if (values.length < 2) return [];
  return ADJUSTMENT_KEYS.filter((key) => values.some((value) => !valuesEqual(value[key], values[0][key])));
}

function rgbToHsl(rgb) {
  const [red, green, blue] = normalizedRgb(rgb).map((value) => value / 255);
  const maximum = Math.max(red, green, blue);
  const minimum = Math.min(red, green, blue);
  const delta = maximum - minimum;
  let hue = 0;
  if (delta > 0) {
    if (maximum === red) hue = 60 * (((green - blue) / delta) % 6);
    else if (maximum === green) hue = 60 * (((blue - red) / delta) + 2);
    else hue = 60 * (((red - green) / delta) + 4);
  }
  if (hue < 0) hue += 360;
  const lightness = (maximum + minimum) / 2;
  const saturation = delta === 0 ? 0 : delta / (1 - Math.abs(2 * lightness - 1));
  return { hue, saturation: saturation * 100, lightness: lightness * 100 };
}

function hslToRgb(hsl) {
  const hue = ((hsl.hue % 360) + 360) % 360;
  const saturation = clamp(hsl.saturation, 0, 100) / 100;
  const lightness = clamp(hsl.lightness, 0, 100) / 100;
  const chroma = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const x = chroma * (1 - Math.abs(((hue / 60) % 2) - 1));
  const offset = lightness - chroma / 2;
  let channels;
  if (hue < 60) channels = [chroma, x, 0];
  else if (hue < 120) channels = [x, chroma, 0];
  else if (hue < 180) channels = [0, chroma, x];
  else if (hue < 240) channels = [0, x, chroma];
  else if (hue < 300) channels = [x, 0, chroma];
  else channels = [chroma, 0, x];
  return channels.map((value) => channel((value + offset) * 255));
}

function mixRgb(left, right, amount) {
  const ratio = clamp(amount, 0, 1);
  return left.map((value, index) => channel(value + (right[index] - value) * ratio));
}

function applyOneAdjustment(sourceRgb, rawAdjustment) {
  const adjustment = cloneAdjustment(rawAdjustment);
  let output = normalizedRgb(sourceRgb);
  if (adjustment.enabled && adjustment.strength > 0) {
    const sourceHsl = rgbToHsl(output);
    const targetHsl = rgbToHsl(adjustment.targetColor);
    const colorized = hslToRgb({
      hue: targetHsl.hue,
      saturation: Math.max(targetHsl.saturation, sourceHsl.saturation * 0.35),
      lightness: adjustment.preserveLuminance ? sourceHsl.lightness : targetHsl.lightness,
    });
    output = mixRgb(output, colorized, adjustment.strength / 100);
  }
  let tuned = rgbToHsl(output);
  tuned = {
    hue: tuned.hue + adjustment.hue,
    saturation: tuned.saturation + adjustment.saturation,
    lightness: tuned.lightness + adjustment.lightness + adjustment.brightness,
  };
  output = hslToRgb(tuned);
  const contrast = 1 + adjustment.contrast / 100;
  return output.map((value) => channel((value - 128) * contrast + 128));
}

export function applyDyeAdjustmentStack(sourceRgb, adjustments) {
  return adjustments.reduce((rgb, adjustment) => applyOneAdjustment(rgb, adjustment), normalizedRgb(sourceRgb));
}

export function setExactDyeOverride(state, canonicalTargetId, colorIndex, rgb) {
  const next = cloneState(state);
  const key = String(canonicalTargetId);
  next.exactOverrides[key] ??= {};
  next.exactOverrides[key][String(colorIndex)] = normalizedRgb(rgb);
  return next;
}

export function clearExactDyeOverride(state, canonicalTargetId, colorIndex) {
  const next = cloneState(state);
  const key = String(canonicalTargetId);
  if (!next.exactOverrides[key]) return next;
  delete next.exactOverrides[key][String(colorIndex)];
  if (!Object.keys(next.exactOverrides[key]).length) delete next.exactOverrides[key];
  return next;
}

export function restoreDyeSemanticState(state, semanticState) {
  const next = cloneState(state);
  next.adjustments = {
    all: defaultDyeAdjustment(),
    parts: {},
    images: {},
  };
  next.exactOverrides = {};
  next.bakedLegacy = Boolean(semanticState?.baked_legacy ?? semanticState?.bakedLegacy);
  if (!semanticState || typeof semanticState !== "object") return next;
  if (semanticState.all) next.adjustments.all = cloneAdjustment(semanticState.all);
  for (const item of Array.isArray(semanticState.parts) ? semanticState.parts : []) {
    const key = String(item?.canonicalTargetId ?? "");
    if (key && item?.adjustment) next.adjustments.parts[key] = cloneAdjustment(item.adjustment);
  }
  for (const item of Array.isArray(semanticState.images) ? semanticState.images : []) {
    const key = String(item?.canonicalTargetId ?? "");
    if (key && item?.adjustment) next.adjustments.images[key] = cloneAdjustment(item.adjustment);
  }
  for (const item of Array.isArray(semanticState.exactOverrides) ? semanticState.exactOverrides : []) {
    const key = String(item?.canonicalTargetId ?? "");
    if (!key || !Number.isInteger(item?.colorIndex)) continue;
    next.exactOverrides[key] ??= {};
    next.exactOverrides[key][String(item.colorIndex)] = normalizedRgb(item.rgb);
  }
  return next;
}

function adjustmentIsDefault(adjustment) {
  const normalized = cloneAdjustment(adjustment);
  const defaults = defaultDyeAdjustment();
  return ADJUSTMENT_KEYS.every((key) => valuesEqual(normalized[key], defaults[key]));
}

function targetPalette(target) {
  return target.palettes?.find((palette) => palette.palette_index === target.active_palette) ?? null;
}

function targetAdjustmentStack(state, target) {
  const key = targetCanonicalKey(target);
  return [
    state.adjustments.all,
    state.adjustments.parts[key] ?? defaultDyeAdjustment(),
    state.adjustments.images[key] ?? defaultDyeAdjustment(),
  ];
}

export function buildDyeBatchMutation(state, targets) {
  const canonicalTargets = new Map();
  for (const target of targets) {
    if (!isTargetEditable(target) || !targetPalette(target)) continue;
    const key = targetCanonicalKey(target);
    if (!canonicalTargets.has(key)) canonicalTargets.set(key, target);
  }
  return {
    targets: [...canonicalTargets.values()].map((target) => {
      const palette = targetPalette(target);
      const overrides = state.exactOverrides[targetCanonicalKey(target)] ?? {};
      const colors = palette.colors.flatMap((color) => {
        const baselineRgba = color.baseline_rgba ?? color.original_rgba;
        if (baselineRgba[3] === 0) return [];
        const baseline = baselineRgba.slice(0, 3);
        const original = color.original_rgba.slice(0, 3);
        const adjusted = applyDyeAdjustmentStack(baseline, targetAdjustmentStack(state, target));
        const rgb = overrides[String(color.color_index)] ?? adjusted;
        return valuesEqual(rgb, original) ? [] : [{ colorIndex: color.color_index, rgb: normalizedRgb(rgb) }];
      });
      return {
        targetId: target.opaque_target_id,
        paletteIndex: target.active_palette,
        colors,
      };
    }),
    semanticState: serializeDyeSemanticState(state, targets),
  };
}

export function serializeDyeSemanticState(state, targets) {
  const targetByCanonicalId = new Map();
  for (const target of targets) {
    const key = targetCanonicalKey(target);
    if (!targetByCanonicalId.has(key)) targetByCanonicalId.set(key, target);
  }
  return {
    version: 1,
    baked_legacy: state.bakedLegacy === true,
    all: adjustmentIsDefault(state.adjustments.all) ? null : cloneAdjustment(state.adjustments.all),
    parts: Object.entries(state.adjustments.parts)
      .filter(([canonicalTargetId, adjustment]) => targetByCanonicalId.has(canonicalTargetId) && !adjustmentIsDefault(adjustment))
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([canonicalTargetId, adjustment]) => ({ canonicalTargetId, adjustment: cloneAdjustment(adjustment) })),
    images: Object.entries(state.adjustments.images)
      .filter(([canonicalTargetId, adjustment]) => targetByCanonicalId.has(canonicalTargetId) && !adjustmentIsDefault(adjustment))
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([canonicalTargetId, adjustment]) => ({
        canonicalTargetId,
        paletteIndex: targetByCanonicalId.get(canonicalTargetId).active_palette,
        adjustment: cloneAdjustment(adjustment),
      })),
    exactOverrides: Object.entries(state.exactOverrides)
      .flatMap(([canonicalTargetId, overrides]) => Object.entries(overrides).map(([colorIndex, rgb]) => ({
        canonicalTargetId,
        colorIndex: Number(colorIndex),
        rgb: normalizedRgb(rgb),
      })))
      .sort((left, right) => left.canonicalTargetId.localeCompare(right.canonicalTargetId) || left.colorIndex - right.colorIndex),
  };
}

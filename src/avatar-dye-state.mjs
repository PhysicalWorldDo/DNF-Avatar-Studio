const OPAQUE_IDENTIFIER_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/;

function assertIntegerInRange(value, minimum, maximum, label) {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new RangeError(`${label} must be an integer from ${minimum} to ${maximum}`);
  }
  return value;
}

function assertOpaqueIdentifier(value, label) {
  if (typeof value !== "string" || !OPAQUE_IDENTIFIER_PATTERN.test(value)) {
    throw new TypeError(`${label} must be an opaque identifier, not a path`);
  }
  return value;
}

function normalizeRgb(value) {
  if (!Array.isArray(value) || value.length !== 3) {
    throw new TypeError("replacementRgb must contain exactly three channels");
  }
  return Object.freeze(value.map((channel, index) => (
    assertIntegerInRange(channel, 0, 255, `replacementRgb[${index}]`)
  )));
}

function normalizeDyeEdit(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("each dye edit must be an object");
  }
  return Object.freeze({
    targetId: assertOpaqueIdentifier(value.targetId, "targetId"),
    paletteIndex: assertIntegerInRange(value.paletteIndex, 0, 255, "paletteIndex"),
    colorIndex: assertIntegerInRange(value.colorIndex, 0, 255, "colorIndex"),
    replacementRgb: normalizeRgb(value.replacementRgb),
  });
}

function editKey(edit) {
  return `${edit.targetId}\u0000${edit.paletteIndex}\u0000${edit.colorIndex}`;
}

function compareEdits(left, right) {
  const leftKey = editKey(left);
  const rightKey = editKey(right);
  if (leftKey < rightKey) return -1;
  if (leftKey > rightKey) return 1;
  return 0;
}

function normalizeEdits(edits) {
  if (!Array.isArray(edits)) throw new TypeError("edits must be an array");
  const unique = new Map();
  for (const rawEdit of edits) {
    const edit = normalizeDyeEdit(rawEdit);
    unique.set(editKey(edit), edit);
  }
  return Object.freeze([...unique.values()].sort(compareEdits));
}

function editsEqual(left, right) {
  if (left.length !== right.length) return false;
  return left.every((edit, index) => {
    const other = right[index];
    return edit.targetId === other.targetId
      && edit.paletteIndex === other.paletteIndex
      && edit.colorIndex === other.colorIndex
      && edit.replacementRgb.every((channel, channelIndex) => (
        channel === other.replacementRgb[channelIndex]
      ));
  });
}

function frozenStack(stack) {
  return Object.freeze([...stack]);
}

function frozenState(state) {
  return Object.freeze(state);
}

function recipeState(state, edits, undoStack, redoStack) {
  return frozenState({
    ...state,
    edits,
    hasDye: edits.length > 0,
    isDirty: !editsEqual(edits, state.persistedEdits),
    revision: state.revision + 1,
    undoStack: frozenStack(undoStack),
    redoStack: frozenStack(redoStack),
  });
}

export function createAvatarDyeState(options = {}) {
  const edits = normalizeEdits(options.edits ?? []);
  return frozenState({
    edits,
    persistedEdits: edits,
    hasDye: edits.length > 0,
    isDirty: false,
    revision: 0,
    undoStack: Object.freeze([]),
    redoStack: Object.freeze([]),
    selectedPaletteIndices: Object.freeze([]),
    paletteSelectionAnchor: null,
    latestRequestId: 0,
    latestAcceptedRequestId: 0,
  });
}

export function commitDyeEdits(state, edits) {
  const normalized = normalizeEdits(edits);
  if (editsEqual(state.edits, normalized)) return state;
  return recipeState(
    state,
    normalized,
    [...state.undoStack, state.edits],
    [],
  );
}

export function markDyePersisted(state) {
  if (!state.isDirty && editsEqual(state.persistedEdits, state.edits)) return state;
  return frozenState({
    ...state,
    persistedEdits: state.edits,
    isDirty: false,
  });
}

export function resetDyeEdits(state) {
  return commitDyeEdits(state, []);
}

export function undoDyeEdit(state) {
  if (state.undoStack.length === 0) return state;
  const edits = state.undoStack[state.undoStack.length - 1];
  return recipeState(
    state,
    edits,
    state.undoStack.slice(0, -1),
    [...state.redoStack, state.edits],
  );
}

export function redoDyeEdit(state) {
  if (state.redoStack.length === 0) return state;
  const edits = state.redoStack[state.redoStack.length - 1];
  return recipeState(
    state,
    edits,
    [...state.undoStack, state.edits],
    state.redoStack.slice(0, -1),
  );
}

function sortedUniqueIndices(indices) {
  return Object.freeze([...new Set(indices)].sort((left, right) => left - right));
}

export function selectPaletteIndex(state, index, options = {}) {
  const maxIndex = options.maxIndex ?? 255;
  assertIntegerInRange(maxIndex, 0, 255, "maxIndex");
  assertIntegerInRange(index, 0, maxIndex, "palette index");

  const ctrlKey = options.ctrlKey === true;
  const shiftKey = options.shiftKey === true;
  let selected;
  let anchor;

  if (shiftKey && state.paletteSelectionAnchor !== null) {
    const start = Math.min(state.paletteSelectionAnchor, index);
    const end = Math.max(state.paletteSelectionAnchor, index);
    const range = Array.from({ length: end - start + 1 }, (_, offset) => start + offset);
    selected = sortedUniqueIndices(ctrlKey ? [...state.selectedPaletteIndices, ...range] : range);
    anchor = state.paletteSelectionAnchor;
  } else if (ctrlKey) {
    const toggled = new Set(state.selectedPaletteIndices);
    if (toggled.has(index)) toggled.delete(index);
    else toggled.add(index);
    selected = sortedUniqueIndices(toggled);
    anchor = index;
  } else {
    selected = Object.freeze([index]);
    anchor = index;
  }

  return frozenState({
    ...state,
    selectedPaletteIndices: selected,
    paletteSelectionAnchor: anchor,
  });
}

export function issueDyeRequest(state) {
  if (state.latestRequestId === Number.MAX_SAFE_INTEGER) {
    throw new RangeError("dye request sequence is exhausted");
  }
  const requestId = state.latestRequestId + 1;
  return Object.freeze({
    requestId,
    state: frozenState({ ...state, latestRequestId: requestId }),
  });
}

export function acceptDyeResponse(state, requestId) {
  assertIntegerInRange(requestId, 1, Number.MAX_SAFE_INTEGER, "requestId");
  if (requestId !== state.latestRequestId || requestId <= state.latestAcceptedRequestId) {
    return Object.freeze({ accepted: false, state });
  }
  return Object.freeze({
    accepted: true,
    state: frozenState({ ...state, latestAcceptedRequestId: requestId }),
  });
}

export function buildDyeCommandPayload(input) {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new TypeError("dye command input must be an object");
  }
  const colorIndices = sortedUniqueIndices(input.colorIndices.map((index) => (
    assertIntegerInRange(index, 0, 255, "color index")
  )));
  if (colorIndices.length === 0) throw new RangeError("colorIndices must not be empty");

  return Object.freeze({
    sessionId: assertOpaqueIdentifier(input.sessionId, "sessionId"),
    targetId: assertOpaqueIdentifier(input.targetId, "targetId"),
    requestId: assertIntegerInRange(input.requestId, 1, Number.MAX_SAFE_INTEGER, "requestId"),
    revision: assertIntegerInRange(input.revision, 0, Number.MAX_SAFE_INTEGER, "revision"),
    operation: Object.freeze({
      kind: "replace_rgb",
      paletteIndex: assertIntegerInRange(input.paletteIndex, 0, 255, "paletteIndex"),
      colorIndices,
      replacementRgb: normalizeRgb(input.replacementRgb),
    }),
  });
}

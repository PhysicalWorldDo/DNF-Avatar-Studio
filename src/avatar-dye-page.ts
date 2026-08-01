import {
  acceptDyeResponse,
  commitDyeEdits,
  createAvatarDyeState,
  issueDyeRequest,
  markDyePersisted,
  redoDyeEdit,
  selectPaletteIndex,
  undoDyeEdit,
  type AvatarDyeEdit,
  type AvatarDyeState,
} from "./avatar-dye-state.mjs";
import {
  buildDyeBatchMutation,
  canonicalDyeTargetId,
  clearExactDyeOverride,
  createDyeInteractionState,
  defaultDyeAdjustment,
  dyeAdjustmentForSelection,
  isTargetEditable,
  mixedDyeAdjustmentKeys,
  restoreDyeSemanticState,
  selectDyeImage,
  selectedDyeTargets,
  setDyeAdjustmentForSelection,
  setDyeObjectMode,
  setExactDyeOverride,
  toggleDyePart,
  type DyeAdjustment,
  type DyeAdjustmentTarget,
  type DyeInteractionState,
  type DyeObjectMode,
} from "./avatar-dye-adjustments.mjs";
import {
  applyDyeColorPreset,
  dyeColorPresetMatchLabel,
  emptyDyeColorPresetCatalog,
  findDyeColorPresetById,
  findMatchingDyeColorPreset,
  normalizeDyeColorPresetCatalog,
  quantizeDyeRgbForEncoding,
  type DyeColorPreset,
  type DyeColorPresetCatalog,
} from "./avatar-dye-presets.mjs";
import { previewFrameIndexForProfession } from "./preview-frame.mjs";

type PreviewImage = {
  width: number;
  height: number;
  rgba_base64?: string;
  rgba?: number[];
  missing_imgs?: string[];
};

type DyePreviewPair<T> = {
  original: T;
  dyed: T;
};

export function retainedDyeAnimationFrame(image: PreviewImage): PreviewImage {
  return {
    width: image.width,
    height: image.height,
    missing_imgs: [...(image.missing_imgs ?? [])],
  };
}

function releaseDyePreviewPairPayload(pair: DyePreviewPair<PreviewImage>) {
  delete pair.original.rgba_base64;
  delete pair.original.rgba;
  delete pair.dyed.rgba_base64;
  delete pair.dyed.rgba;
}

export type DyePaletteColorView = {
  color_index: number;
  original_rgba: [number, number, number, number];
  baseline_rgba?: [number, number, number, number];
  current_rgba: [number, number, number, number];
  used_pixel_count: number;
  used_frame_count?: number;
  used_frame_bitmap_base64?: string;
};

export type DyePaletteView = {
  palette_index: number;
  rgb_encoding?: "argb1555" | "argb4444" | "argb8888" | null;
  colors: DyePaletteColorView[];
};

export type DyeTargetKindView = "Base" | "base" | {
  Mask?: { number?: number };
  mask?: { number?: number };
  number?: number;
};

export type DyeTargetView = {
  opaque_target_id: string;
  canonical_target_id?: string;
  reference_count?: number;
  is_shared?: boolean;
  resource_status?: string;
  preview_available?: boolean;
  slot_label?: string;
  slot?: string;
  layer_label?: string;
  kind: DyeTargetKindView;
  active_palette: number;
  img_version: number;
  frame_count: number;
  capability: string;
  palettes: DyePaletteView[];
};

export type DyeWorkspaceSummaryView = {
  label?: string;
  occupied_slot_count?: number;
  has_dye?: boolean;
  dye_status?: string;
  dyed_slot_count?: number;
  dyed_base_count?: number;
  dyed_mask_count?: number;
  recipe_revision?: string | null;
};

export type AvatarDyeSessionView = {
  session_id: string;
  playable_role_key: string;
  revision: number | string;
  recipe_revision: string | null;
  has_dye: boolean;
  is_dirty: boolean;
  can_undo: boolean;
  can_redo: boolean;
  resource_status: string;
  editing_blocked?: boolean;
  persistence_blocked?: boolean;
  targets: DyeTargetView[];
  semantic_state?: unknown;
  baked_legacy?: boolean;
  current?: DyeWorkspaceSummaryView | null;
  target?: DyeWorkspaceSummaryView | null;
};

export type DyeComparisonFrameResponse = DyePreviewPair<PreviewImage> & {
  request_id?: number;
  requestId?: number;
};

export type DyeComparisonAnimationResponse = {
  frame_count: number;
  missing_imgs?: string[];
  frames: Array<DyePreviewPair<PreviewImage>>;
  request_id?: number;
  requestId?: number;
};

export function alignedDyePreviewPair(pair: DyePreviewPair<Pick<PreviewImage, "width" | "height">>) {
  return pair.original.width > 0
    && pair.original.height > 0
    && pair.original.width === pair.dyed.width
    && pair.original.height === pair.dyed.height;
}

export type DyeColorOperation =
  | { kind: "set_rgb"; colorIndices: number[]; rgb: [number, number, number] }
  | {
      kind: "relative_hsl";
      colorIndices: number[];
      hueDelta: number;
      saturationDelta: number;
      lightnessDelta: number;
      contrastDelta: number;
      scope: "current" | "zone" | "family" | "part";
    }
  | { kind: "copy_reference_palette"; referencePaletteIndex: number };

export type DyeBatchMutation = {
  targets: Array<{
    targetId: string;
    paletteIndex: number;
    colors: Array<{ colorIndex: number; rgb: [number, number, number] }>;
  }>;
  semanticState: unknown;
};

export type DyeSnapshotSummary = {
  label: string;
  occupiedSlotCount: number;
  hasDye: boolean;
  dyeStatus: string;
  dyedSlotCount: number;
  dyedBaseCount: number;
  dyedMaskCount: number;
  recipeRevision: string | null;
  professionDir?: string;
  pathFamily?: string;
};

export type DyeWardrobeSourceIdentity = {
  snapshotRevision: string;
  recipeRevision: string | null;
  professionDir?: string;
  pathFamily?: string;
};

export type DyeWorkspaceCompatibilityIdentity = {
  professionDir: string;
  pathFamily: string;
};

export function dyeWorkspaceStatusPresentation(status: string, hasDye: boolean) {
  const normalized = status.trim().toLowerCase();
  if (!hasDye || normalized === "none" || normalized === "no_dye") {
    return { label: "原色", tone: "original", persistenceBlocked: false };
  }
  switch (normalized) {
    case "valid":
      return { label: "染色", tone: "valid", persistenceBlocked: false };
    case "changed":
      return { label: "资源已变化", tone: "error", persistenceBlocked: true };
    case "partial":
      return { label: "部分失效", tone: "error", persistenceBlocked: true };
    case "unsupported":
      return { label: "不支持", tone: "error", persistenceBlocked: true };
    default:
      return { label: "待校验", tone: "warn", persistenceBlocked: true };
  }
}

export type AvatarDyePageHost = {
  commands: {
    loadDyeColorPresets(): Promise<DyeColorPresetCatalog>;
    createDyeColorPreset(args: {
      expectedRevision: number;
      name: string;
      rgb: [number, number, number];
    }): Promise<DyeColorPresetCatalog>;
    updateDyeColorPreset(args: {
      expectedRevision: number;
      id: string;
      name: string;
      rgb: [number, number, number];
    }): Promise<DyeColorPresetCatalog>;
    deleteDyeColorPreset(args: {
      expectedRevision: number;
      id: string;
    }): Promise<DyeColorPresetCatalog>;
    openFromStudio(args: { gameDir: string; snapshot: unknown }): Promise<AvatarDyeSessionView>;
    openFromWardrobe(args: { gameDir: string; entryId: string }): Promise<AvatarDyeSessionView>;
    getView(args: { sessionId: string }): Promise<AvatarDyeSessionView>;
    updateColors(args: { sessionId: string; targetId: string; operation: DyeColorOperation }): Promise<AvatarDyeSessionView>;
    updateBatch(args: { sessionId: string; mutation: DyeBatchMutation }): Promise<AvatarDyeSessionView>;
    previewComparisonColorsFrame(args: {
      sessionId: string;
      frameIndex: number;
      previewScope: string;
      targetId: string;
      operation: DyeColorOperation;
      hideSelectedMask: boolean;
      hiddenTargetIds: string[];
      requestId: number;
    }): Promise<DyeComparisonFrameResponse>;
    previewBatchComparison(args: {
      sessionId: string;
      frameIndex: number;
      previewScope: string;
      targetId: string;
      mutation: DyeBatchMutation;
      hideSelectedMask: boolean;
      hiddenTargetIds: string[];
      requestId: number;
    }): Promise<DyeComparisonFrameResponse>;
    repairTarget(args: { sessionId: string; targetId: string }): Promise<AvatarDyeSessionView>;
    resetScope(args: { sessionId: string; scope: string; targetId?: string; colorIndices?: number[] }): Promise<AvatarDyeSessionView>;
    undo(args: { sessionId: string }): Promise<AvatarDyeSessionView>;
    redo(args: { sessionId: string }): Promise<AvatarDyeSessionView>;
    renderComparisonFrame(args: {
      sessionId: string;
      frameIndex: number;
      previewScope: string;
      targetId: string;
      hideSelectedMask: boolean;
      hiddenTargetIds: string[];
      requestId: number;
    }): Promise<DyeComparisonFrameResponse>;
    renderComparisonAnimation(args: {
      sessionId: string;
      previewScope: string;
      targetId: string;
      hideSelectedMask: boolean;
      hiddenTargetIds: string[];
      requestId: number;
    }): Promise<DyeComparisonAnimationResponse>;
    saveToWardrobe(args: { sessionId: string; draft: unknown; allowDuplicate: boolean }): Promise<unknown>;
    saveAsActive(args: { sessionId: string; role: "current" | "target"; clearIncompatible: boolean }): Promise<unknown>;
    saveAsTargetAndOpenSwap(args: { sessionId: string; clearIncompatible: boolean }): Promise<unknown>;
    exportNpk(args: { sessionId: string; outputDirectory: string }): Promise<{ output_path: string; entry_count: number }>;
    close(args: { sessionId: string }): Promise<void>;
  };
  getGameDir(): string;
  getWorkspace(): { current: DyeSnapshotSummary | null; target: DyeSnapshotSummary | null };
  getWardrobeGroups(): Array<{ id: string; name: string }>;
  getWardrobeSourceIdentity(entryId: string): DyeWardrobeSourceIdentity | null;
  canUseWardrobe(): boolean;
  chooseExportDirectory(): Promise<string | null>;
  onWardrobeState(value: unknown): void;
  onStatus(message: string): void;
  onReturn(source: "studio" | "wardrobe"): void;
  onEnterSwap(): void | Promise<void>;
  closeWindow(): Promise<void>;
};

export type AvatarDyePageController = {
  wire(): void;
  render(): void;
  isOpen(): boolean;
  requestWindowClose: () => boolean;
  openFromStudio(snapshot: unknown, preferredLayer?: string): Promise<void>;
  openFromWardrobe(entryId: string): Promise<void>;
};

type DyeDialogMode = "wardrobe" | "workspace" | "swap" | "export" | "leave" | "replace_draft" | null;
type PreviewScope = "outfit" | "part" | "target";
type DyeWardrobeDialogDraft = {
  name: string;
  groupId: string;
  note: string;
  favorite: boolean;
};
type PendingDyeOpen = {
  source: "studio" | "wardrobe";
  identity: string;
  preferredLayer: string;
  open: () => Promise<void>;
};
type DyePresetDialogMode = "create" | "edit" | "delete" | null;
type DyePresetDialogDraft = {
  presetId: string;
  name: string;
  rgbHex: string;
};

const PLAY_FRAME_MS = 80;
const PREVIEW_DEBOUNCE_MS = 120;

export function normalizedDyePreviewScope(
  objectMode: DyeObjectMode,
  previewScope: PreviewScope,
  selectedPartCount: number,
): PreviewScope {
  if (objectMode === "all") return "outfit";
  if (objectMode === "part") {
    if (selectedPartCount < 1) return "outfit";
    return previewScope === "target" ? "part" : previewScope;
  }
  return previewScope;
}

export function dyeResourcePolicy(status: string) {
  const normalized = status.trim().toLowerCase().replace(/[\s-]+/g, "_");
  const repairable = normalized === "changed" || normalized === "resource_changed";
  const incomplete = normalized === "incomplete";
  const segments = normalized.split("_").filter(Boolean);
  const editingBlocked = repairable || segments.some((segment) => [
    "invalid",
    "conflict",
    "missing",
    "blocked",
    "unsupported",
  ].includes(segment));
  return {
    repairable,
    editingBlocked,
    persistenceBlocked: repairable || editingBlocked || incomplete,
  };
}

export function dyeTargetPreviewAvailable(target: { preview_available?: boolean }) {
  return target.preview_available !== false;
}

export function dyeTargetStateDetail(status: string, changedColorCount: number) {
  const normalized = status.trim().toLowerCase().replace(/[\s-]+/g, "_");
  if (normalized === "changed") return "待修复";
  if (normalized === "missing" || normalized === "conflict") return "需重置";
  if (normalized === "invalid") return "不可用";
  return changedColorCount > 0 ? `${changedColorCount} 色` : "原色";
}

export function dyeIncompleteResourceLabel(targetStatuses: string[]) {
  const statuses = new Set(targetStatuses.map((status) => (
    status.trim().toLowerCase().replace(/[\s-]+/g, "_")
  )));
  const missing = statuses.has("missing");
  const invalid = statuses.has("invalid");
  if (missing && invalid) return "部分资源缺失或解析失败";
  if (invalid) return "部分资源解析失败";
  if (missing) return "部分资源缺失";
  return "部分资源不可用";
}

export function dyeIncompleteResourceNotice(targetStatuses: string[]) {
  const label = dyeIncompleteResourceLabel(targetStatuses).replace("部分资源", "部分工作台资源");
  return `${label}；其余有效主 IMG 与 Mask 仍可编辑和预览，但禁止保存或导出不完整结果。`;
}

export function dyeSessionBlockingPolicy(view: {
  resource_status?: string;
  editing_blocked?: boolean;
  persistence_blocked?: boolean;
}) {
  const fallback = dyeResourcePolicy(view.resource_status ?? "");
  return {
    editingBlocked: view.editing_blocked ?? fallback.editingBlocked,
    persistenceBlocked: view.persistence_blocked ?? fallback.persistenceBlocked,
  };
}

export function retainedDraftOpenDecision(retainedIdentity: string, requestedIdentity: string) {
  if (!retainedIdentity) return "open_new" as const;
  return retainedIdentity === requestedIdentity ? "resume" as const : "choose" as const;
}

export function dyeDialogActionBlockedByResource(action: string | undefined) {
  return [
    "save-wardrobe",
    "save-current",
    "save-target",
    "save-swap",
    "export-folder",
    "leave-save",
  ].includes(action ?? "");
}

export function dyePersistenceErrorMessage(action: string, error: unknown) {
  if (typeof error === "string") {
    const message = error.trim();
    if (message) return message;
  } else if (error instanceof Error) {
    const message = error.message.trim();
    if (message) return `${action}失败：${message}`;
  } else if (error && typeof error === "object") {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.trim()) {
      return `${action}失败：${message.trim()}`;
    }
  }
  return `${action}失败：未知错误，请重试`;
}

export function dyeWindowCloseDecision(options: {
  hasView: boolean;
  dirty: boolean;
  loading: boolean;
  saving: boolean;
}) {
  if (options.loading || options.saving) return "busy" as const;
  if (!options.hasView) return "allow" as const;
  return options.dirty ? "confirm" as const : "allow" as const;
}

export function dyeWorkspaceCompatibilityIdentity(value: unknown): DyeWorkspaceCompatibilityIdentity | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  const professionDir = dyeIdentityText(record.professionDir ?? record.profession_dir);
  const pathFamily = dyeIdentityText(record.pathFamily ?? record.path_family);
  return professionDir && pathFamily ? { professionDir, pathFamily } : null;
}

export function dyeWorkspaceIdentitiesIncompatible(
  source: DyeWorkspaceCompatibilityIdentity | null,
  other: unknown,
) {
  const otherIdentity = dyeWorkspaceCompatibilityIdentity(other);
  if (!source || !otherIdentity) return false;
  return normalizedDyeIdentityPath(source.professionDir) !== normalizedDyeIdentityPath(otherIdentity.professionDir)
    || normalizedDyeIdentityPath(source.pathFamily) !== normalizedDyeIdentityPath(otherIdentity.pathFamily);
}

export function dyeIncompatiblePersistencePrompt(role: "current" | "target", shortcut: boolean) {
  const incoming = role === "current" ? "当前穿戴" : "目标外观";
  const existing = role === "current" ? "目标外观" : "当前穿戴";
  const next = shortcut ? "，随后前往一键换装" : "";
  return `新的${incoming}与已有${existing}不是同一职业或资源家族。继续会清空已有${existing}${next}，是否继续？`;
}

export async function persistDyeWorkspaceWithCompatibilityConfirmation<T>(options: {
  role: "current" | "target";
  shortcut: boolean;
  incompatible: boolean;
  persist(clearIncompatible: boolean): Promise<T>;
  confirm(message: string): boolean;
}): Promise<{ status: "saved"; value: T } | { status: "cancelled" }> {
  try {
    return { status: "saved", value: await options.persist(false) };
  } catch (error) {
    if (!options.incompatible) throw error;
    if (!options.confirm(dyeIncompatiblePersistencePrompt(options.role, options.shortcut))) {
      return { status: "cancelled" };
    }
    return { status: "saved", value: await options.persist(true) };
  }
}

export function normalizeDyeGameDir(gameDir: string) {
  const source = gameDir.trim().replaceAll("/", "\\");
  if (!source) return "";
  const isUnc = source.startsWith("\\\\");
  const parts = source.split(/\\+/).filter(Boolean);
  const isDrive = !isUnc && /^[a-z]:$/i.test(parts[0] ?? "");
  const rootDepth = isUnc ? Math.min(2, parts.length) : isDrive ? 1 : 0;
  const normalized: string[] = [];
  for (const part of parts) {
    if (part === ".") continue;
    if (part === "..") {
      if (normalized.length > rootDepth) normalized.pop();
      else if (!isUnc && !isDrive) normalized.push(part);
      continue;
    }
    normalized.push(part);
  }
  let result = `${isUnc ? "\\\\" : ""}${normalized.join("\\")}`;
  if (isDrive && normalized.length === 1) result += "\\";
  return result.toLowerCase();
}

export function studioDyeSourceIdentity(gameDir: string, snapshot: unknown) {
  const value = snapshot && typeof snapshot === "object" && !Array.isArray(snapshot)
    ? snapshot as Record<string, unknown>
    : {};
  const items = Array.isArray(value.items)
    ? value.items.map((item) => {
      const publicItem = item && typeof item === "object" && !Array.isArray(item)
        ? item as Record<string, unknown>
        : {};
      return [
        normalizedDyeIdentityPath(publicItem.slot),
        normalizedDyeIdentityPath(publicItem.item_ref),
        dyeIdentityText(publicItem.item_name),
        dyeIdentityText(publicItem.layer_count),
      ];
    }).sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right)))
    : [];
  return `studio:v2:${JSON.stringify([
    normalizeDyeGameDir(gameDir),
    normalizedDyeIdentityPath(value.profession_dir),
    normalizedDyeIdentityPath(value.path_family),
    optionalDyeIdentityText(value.snapshot_revision),
    optionalDyeIdentityText(value.revision),
    optionalDyeIdentityText(value.recipe_revision),
    items,
  ])}`;
}

export function wardrobeDyeSourceIdentity(
  gameDir: string,
  entryId: string,
  source: DyeWardrobeSourceIdentity | null,
) {
  return `wardrobe:v2:${JSON.stringify([
    normalizeDyeGameDir(gameDir),
    entryId.trim(),
    source ? source.snapshotRevision.trim() : null,
    source ? optionalDyeIdentityText(source.recipeRevision) : null,
  ])}`;
}

function normalizedDyeIdentityPath(value: unknown) {
  return dyeIdentityText(value).replaceAll("\\", "/").replace(/\/+/g, "/").toLowerCase();
}

function dyeIdentityText(value: unknown) {
  return String(value ?? "").trim().normalize("NFC");
}

function optionalDyeIdentityText(value: unknown) {
  return value === null || value === undefined ? null : dyeIdentityText(value);
}

export function dyeLiveResourceChangeGuidance() {
  return "源资源已变化；当前染色会话已冻结。请关闭染色工作台，确认游戏资源刷新完成并稳定后重新打开；顶部“刷新”仅同步当前会话的冻结状态。";
}

export function dyePersistedResourceChangeGuidance() {
  return "旧配方目标已变化；保存、导出和普通编辑已冻结。请修复右侧标记目标，或明确恢复全部染色。";
}

export function dyeValidationPresentation(status: string, hasRepairableTarget = false) {
  const policy = dyeResourcePolicy(status);
  if (policy.repairable) {
    return {
      tone: "warn" as const,
      title: "资源已变化",
      detail: hasRepairableTarget
        ? dyePersistedResourceChangeGuidance()
        : dyeLiveResourceChangeGuidance(),
    };
  }
  if (policy.editingBlocked) {
    return {
      tone: "error" as const,
      title: "校验失败",
      detail: "当前资源不能安全保存或生成补丁。",
    };
  }
  const normalized = status.trim().toLowerCase().replace(/[\s-]+/g, "_");
  if (normalized === "partial_read_only") {
    return {
      tone: "good" as const,
      title: "已通过（部分只读）",
      detail: "可染目标校验通过；只读 IMG 保留原色并继续参与预览和换装。",
    };
  }
  if (normalized === "valid" || normalized === "ready") {
    return {
      tone: "good" as const,
      title: "已通过",
      detail: "源 NPK、IMG 与当前会话指纹一致。",
    };
  }
  return {
    tone: "warn" as const,
    title: "待校验",
    detail: "后端会在保存或导出前再次执行严格校验。",
  };
}

export function dyePreviewHidesSelectedMask(
  scope: PreviewScope,
  selectedIsMask: boolean,
  hideRequested: boolean,
) {
  return hideRequested && selectedIsMask && scope !== "target";
}

export function dyeTargetMaskNumber(target: Pick<DyeTargetView, "kind">) {
  if (typeof target.kind === "string") {
    return target.kind.toLowerCase().includes("mask") ? 1 : null;
  }
  const descriptor = target.kind.Mask ?? target.kind.mask ?? target.kind;
  return typeof descriptor.number === "number" ? descriptor.number : null;
}

type DyeTimelineTarget = Pick<DyeTargetView, "opaque_target_id" | "frame_count" | "kind"> & {
  slot?: string;
  slot_label?: string;
};

export function dyePreviewFrameCount(
    targets: readonly DyeTimelineTarget[],
    selectedTargetId: string,
    scope: PreviewScope,
    hideSelectedMask: boolean,
    hiddenTargetIds: readonly string[] = [],
) {
    const selected = targets.find((target) => target.opaque_target_id === selectedTargetId);
    if (!selected) return 0;
    const selectedSlot = (selected.slot || selected.slot_label || "").trim().toLowerCase();
    const hidden = new Set(hiddenTargetIds);
    const scoped = targets.filter((target) => {
        if (scope !== "target" && dyeTargetMaskNumber(target) !== null) return false;
        if (hideSelectedMask && scope !== "target" && target.opaque_target_id === selectedTargetId) return false;
        if (scope === "target") return target.opaque_target_id === selectedTargetId;
        if (hidden.has(target.opaque_target_id)) return false;
        if (scope === "part") return (target.slot || target.slot_label || "").trim().toLowerCase() === selectedSlot;
    return true;
  });
  return scoped.reduce((maximum, target) => Math.max(maximum, Math.max(0, Math.trunc(target.frame_count))), 0);
}

export function validatedDyeComparisonAnimationFrameCount(
  scopedFrameCount: number,
  reportedFrameCount: number,
  receivedFrameCount: number,
) {
  const scoped = Math.max(0, Math.trunc(scopedFrameCount));
  const reported = Math.max(0, Math.trunc(reportedFrameCount));
  const received = Math.max(0, Math.trunc(receivedFrameCount));
  if (!reported || reported !== received) return null;
  if (scoped && scoped !== reported) return null;
  return reported;
}

export function groupDyeTargetsByPart(targets: readonly DyeTargetView[]) {
  const groups = new Map<string, { key: string; label: string; targets: DyeTargetView[] }>();
  for (const target of targets) {
    const slot = (target.slot || target.slot_label || "未分类").trim();
    const key = slot.toLowerCase() || "未分类";
    const label = target.slot_label?.trim() || target.slot?.trim() || "未分类";
    const group = groups.get(key) ?? { key, label, targets: [] };
    group.targets.push(target);
    groups.set(key, group);
  }
  return [...groups.values()];
}

export function dyeTargetTreeRowLabel(layerLabel: string | undefined, kindLabel: string) {
  const layer = layerLabel?.trim() || "默认层";
  const kind = kindLabel.trim();
  return layer === kind ? kind : `${layer} · ${kind}`;
}

export function buildDyeColorZones(colors: readonly DyePaletteColorView[], maximumZones = 3) {
  const used = colors
    .filter((color) => color.used_pixel_count > 0 && color.current_rgba[3] > 0)
    .map((color) => ({ color, ...rgbToHsl(color.current_rgba) }));
  if (!used.length || maximumZones < 1) return [];

  const clusters = used.map((item) => [item]);
  while (clusters.length > Math.min(maximumZones, used.length)) {
    let bestLeft = 0;
    let bestRight = 1;
    let bestDistance = Number.POSITIVE_INFINITY;
    for (let left = 0; left < clusters.length - 1; left += 1) {
      for (let right = left + 1; right < clusters.length; right += 1) {
        const distance = dyeClusterDistance(clusters[left], clusters[right]);
        if (distance < bestDistance) {
          bestDistance = distance;
          bestLeft = left;
          bestRight = right;
        }
      }
    }
    clusters[bestLeft] = [...clusters[bestLeft], ...clusters[bestRight]];
    clusters.splice(bestRight, 1);
  }

  const totalUsedPixels = used.reduce((sum, item) => sum + item.color.used_pixel_count, 0);
  return clusters
    .map((cluster) => {
      const colorsByLightness = [...cluster]
        .sort((left, right) => left.lightness - right.lightness || left.color.color_index - right.color.color_index)
        .map((item) => item.color);
      const pixels = colorsByLightness.reduce((sum, color) => sum + color.used_pixel_count, 0);
      return {
        colors: colorsByLightness,
        pixels,
        pixelShare: Number(((pixels / totalUsedPixels) * 100).toFixed(1)),
        frames: dyeAffectedFrameUnionCount(colorsByLightness),
        sortKey: dyeClusterCentroid(cluster),
      };
    })
    .sort((left, right) => left.sortKey.hue - right.sortKey.hue || left.sortKey.lightness - right.sortKey.lightness)
    .map(({ sortKey: _sortKey, ...zone }) => zone);
}

export function dyeAffectedFrameUnionCount(colors: readonly DyePaletteColorView[]) {
  const fallback = colors.reduce(
    (maximum, color) => Math.max(maximum, color.used_frame_count ?? 0),
    0,
  );
  if (!colors.length || colors.some((color) => !color.used_frame_bitmap_base64)) return fallback;
  try {
    const decoded = colors.map((color) => Uint8Array.from(
      atob(color.used_frame_bitmap_base64 ?? ""),
      (character) => character.charCodeAt(0),
    ));
    const union = new Uint8Array(decoded.reduce((maximum, bitmap) => Math.max(maximum, bitmap.length), 0));
    for (const bitmap of decoded) {
      for (let index = 0; index < bitmap.length; index += 1) union[index] |= bitmap[index];
    }
    let count = 0;
    for (const byte of union) {
      let bits = byte;
      while (bits) {
        count += bits & 1;
        bits >>>= 1;
      }
    }
    return count;
  } catch {
    return fallback;
  }
}

export function weightedDyeZoneRepresentative(
  colors: readonly DyePaletteColorView[],
  channel: "current" | "original" = "current",
) {
  const hsl = weightedDyeZoneHsl(colors, channel);
  const rgb = hslToRgb(hsl.hue, hsl.saturation, hsl.lightness);
  return { rgb, hex: dyeRgbHex(rgb) };
}

export function dyeRelativeTransformForTarget(
  colors: readonly DyePaletteColorView[],
  targetRgb: readonly number[],
) {
  const source = weightedDyeZoneHsl(colors, "current");
  const target = rgbToHsl(targetRgb);
  const rawHueDelta = target.hue - source.hue;
  const hueDelta = Math.round(((rawHueDelta + 540) % 360) - 180) || 0;
  const saturationDelta = source.saturation > 0.000_001
    ? Math.round((target.saturation / source.saturation - 1) * 100)
    : 0;
  return {
    hueDelta,
    saturationDelta: Math.max(-100, Math.min(100, saturationDelta)) || 0,
    lightnessDelta: Math.max(-100, Math.min(100, Math.round((target.lightness - source.lightness) * 100))) || 0,
  };
}

function weightedDyeZoneHsl(
  colors: readonly DyePaletteColorView[],
  channel: "current" | "original",
) {
  const features = colors
    .filter((color) => color.used_pixel_count > 0 && (channel === "current" ? color.current_rgba : color.original_rgba)[3] > 0)
    .map((color) => ({ color, ...rgbToHsl(channel === "current" ? color.current_rgba : color.original_rgba) }));
  return features.length ? dyeClusterCentroid(features) : { hue: 0, saturation: 0, lightness: 0 };
}

type DyeColorFeature = ReturnType<typeof rgbToHsl> & { color: DyePaletteColorView };

function dyeClusterDistance(left: DyeColorFeature[], right: DyeColorFeature[]) {
  const leftCenter = dyeClusterCentroid(left);
  const rightCenter = dyeClusterCentroid(right);
  const hueGap = Math.abs(leftCenter.hue - rightCenter.hue);
  const circularHueGap = Math.min(hueGap, 360 - hueGap) / 180;
  const hueReliability = Math.max(0.12, Math.min(leftCenter.saturation, rightCenter.saturation));
  const lightnessGap = Math.abs(leftCenter.lightness - rightCenter.lightness);
  const saturationGap = Math.abs(leftCenter.saturation - rightCenter.saturation);
  return circularHueGap * hueReliability * 0.68 + lightnessGap * 0.27 + saturationGap * 0.05;
}

function dyeClusterCentroid(cluster: DyeColorFeature[]) {
  let sin = 0;
  let cos = 0;
  let lightness = 0;
  let saturation = 0;
  let totalWeight = 0;
  for (const item of cluster) {
    const weight = Math.max(1, item.color.used_pixel_count);
    const hueWeight = weight * Math.max(0.08, item.saturation);
    const radians = item.hue * Math.PI / 180;
    sin += Math.sin(radians) * hueWeight;
    cos += Math.cos(radians) * hueWeight;
    lightness += item.lightness * weight;
    saturation += item.saturation * weight;
    totalWeight += weight;
  }
  const hue = (Math.atan2(sin, cos) * 180 / Math.PI + 360) % 360;
  return { hue, lightness: lightness / totalWeight, saturation: saturation / totalWeight };
}

function rgbToHsl(rgba: readonly number[]) {
  const [red, green, blue] = rgba.slice(0, 3).map((value) => value / 255);
  const maximum = Math.max(red, green, blue);
  const minimum = Math.min(red, green, blue);
  const difference = maximum - minimum;
  const lightness = (maximum + minimum) / 2;
  let hue = 0;
  if (difference > 0) {
    if (maximum === red) hue = 60 * (((green - blue) / difference) % 6);
    else if (maximum === green) hue = 60 * ((blue - red) / difference + 2);
    else hue = 60 * ((red - green) / difference + 4);
  }
  if (hue < 0) hue += 360;
  const saturation = difference === 0 ? 0 : difference / (1 - Math.abs(2 * lightness - 1));
  return { hue, saturation, lightness };
}

function hslToRgb(hue: number, saturation: number, lightness: number): [number, number, number] {
  const chroma = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const sector = ((hue % 360) + 360) % 360 / 60;
  const secondary = chroma * (1 - Math.abs((sector % 2) - 1));
  let red = 0;
  let green = 0;
  let blue = 0;
  if (sector < 1) [red, green] = [chroma, secondary];
  else if (sector < 2) [red, green] = [secondary, chroma];
  else if (sector < 3) [green, blue] = [chroma, secondary];
  else if (sector < 4) [green, blue] = [secondary, chroma];
  else if (sector < 5) [red, blue] = [secondary, chroma];
  else [red, blue] = [chroma, secondary];
  const match = lightness - chroma / 2;
  return [red, green, blue].map((value) => Math.round((value + match) * 255)) as [number, number, number];
}

function dyeRgbHex(rgb: readonly number[]) {
  return `#${rgb.slice(0, 3).map((value) => Math.max(0, Math.min(255, Math.round(value))).toString(16).padStart(2, "0")).join("")}`.toUpperCase();
}

export function createAvatarDyePageController(host: AvatarDyePageHost): AvatarDyePageController {
  const state = {
    open: false,
    retained: false,
    source: "studio" as "studio" | "wardrobe",
    sourceIdentity: "",
    sourceCompatibility: null as DyeWorkspaceCompatibilityIdentity | null,
    pendingOpen: null as PendingDyeOpen | null,
    view: null as AvatarDyeSessionView | null,
    logic: createAvatarDyeState() as AvatarDyeState,
    interaction: createDyeInteractionState() as DyeInteractionState,
    gestureBefore: null as DyeInteractionState | null,
    selectedTargetId: "",
    selectedPalette: 0,
    collapsedTargetGroups: new Set<string>(),
    editorMode: "swatches" as "swatches" | "transform",
    presetCatalog: emptyDyeColorPresetCatalog() as DyeColorPresetCatalog,
    presetTab: "builtin" as "builtin" | "custom",
    presetLoaded: false,
    presetLoading: false,
    presetLoadError: "",
    presetSaving: false,
    presetDialog: null as DyePresetDialogMode,
    presetDialogDraft: null as DyePresetDialogDraft | null,
    presetDialogError: "",
    previewScope: "outfit" as PreviewScope,
    hideSelectedMask: false,
    hiddenTargetIds: new Set<string>(),
    frame: 0,
    animationFrameCount: 0,
    playing: false,
    playTimer: 0,
    previewDebounceTimer: 0,
    transientPreviewTimer: 0,
    animationFrames: [] as Array<DyePreviewPair<PreviewImage>>,
    animationImageData: [] as Array<DyePreviewPair<ImageData>>,
    animationIndex: 0,
    loading: false,
    rendering: false,
    saving: false,
    error: "",
    dialog: null as DyeDialogMode,
    dialogError: "",
    wardrobeDialogDraft: null as DyeWardrobeDialogDraft | null,
    preferredLayer: "",
    leaveAfterSave: false,
    windowClosePending: false,
  };

  function wire() {
    query("#avatarDyeBack")?.addEventListener("click", requestLeave);
    query("#avatarDyeRefresh")?.addEventListener("click", () => void refreshSession());
    query("#avatarDyeUndo")?.addEventListener("click", () => void undo());
    query("#avatarDyeRedo")?.addEventListener("click", () => void redo());
    query("#avatarDyeResetAll")?.addEventListener("click", () => void reset("all"));
    query("#avatarDyeHideSelectedMask")?.addEventListener("click", () => {
      const target = selectedTarget();
      if (!target || !isMask(target) || state.previewScope === "target") return;
      if (state.hiddenTargetIds.has(target.opaque_target_id)) {
        state.hiddenTargetIds.delete(target.opaque_target_id);
      } else {
        state.hiddenTargetIds.add(target.opaque_target_id);
      }
      state.hideSelectedMask = state.hiddenTargetIds.has(target.opaque_target_id);
      resetAnimation();
      render();
      void renderFrame();
    });
    query("#avatarDyePlay")?.addEventListener("click", () => void togglePlayback());
    query("#avatarDyeFitCanvas")?.addEventListener("click", () => {
      fitCanvasToViewport();
      host.onStatus("染色预览已适应当前画布区域");
    });
    query<HTMLInputElement>("#avatarDyeFrame")?.addEventListener("input", (event) => {
      state.frame = Number((event.currentTarget as HTMLInputElement).value);
      updateTimeline();
      if (state.playing) drawAnimationFrame(state.frame);
      else scheduleFrameRender();
    });
    query("#avatarDyeSaveWardrobe")?.addEventListener("click", () => openDialog("wardrobe"));
    query("#avatarDyeSaveWorkspace")?.addEventListener("click", () => openDialog("workspace"));
    query("#avatarDyeGoSwap")?.addEventListener("click", () => openDialog("swap"));
    query("#avatarDyeExport")?.addEventListener("click", () => openDialog("export"));
    query("#avatarDyeDialogClose")?.addEventListener("click", requestDialogClose);
    query("#avatarDyePage")?.addEventListener("click", handlePageClick);
    query("#avatarDyeDialog")?.addEventListener("click", handleDialogClick);
    document.addEventListener("keydown", handleKeyDown);
    window.addEventListener("beforeunload", warnBeforeUnload);
  }

  function dyePresetErrorMessage(error: unknown) {
    if (typeof error === "string" && error.trim()) return error.trim();
    if (error instanceof Error && error.message.trim()) return error.message.trim();
    if (error && typeof error === "object" && "message" in error) {
      const message = String((error as { message?: unknown }).message ?? "").trim();
      if (message) return message;
    }
    return "染色剂预设操作失败，请重试";
  }

  async function loadDyeColorPresets() {
    if (state.presetLoading) return;
    state.presetLoading = true;
    state.presetLoadError = "";
    if (state.open) renderEditor();
    try {
      state.presetCatalog = normalizeDyeColorPresetCatalog(await host.commands.loadDyeColorPresets());
      state.presetLoaded = true;
    } catch (error) {
      state.presetLoadError = dyePresetErrorMessage(error);
    } finally {
      state.presetLoading = false;
      if (state.open) renderEditor();
    }
  }

  async function openFromStudio(snapshot: unknown, preferredLayer = "") {
    if (state.loading || state.saving) return;
    const gameDir = host.getGameDir();
    const identity = studioDyeSourceIdentity(gameDir, snapshot);
    const compatibility = dyeWorkspaceCompatibilityIdentity(snapshot);
    if (handleRetainedOpen({
      source: "studio",
      identity,
      preferredLayer,
      open: () => openFreshStudio(snapshot, preferredLayer, gameDir, identity, compatibility),
    })) return;
    await openFreshStudio(snapshot, preferredLayer, gameDir, identity, compatibility);
  }

  async function openFreshStudio(
    snapshot: unknown,
    preferredLayer: string,
    gameDir: string,
    identity: string,
    compatibility: DyeWorkspaceCompatibilityIdentity | null,
  ) {
    state.loading = true;
    state.error = "";
    state.preferredLayer = preferredLayer;
    host.onStatus("正在创建时装染色会话");
    try {
      beginSession(await host.commands.openFromStudio({ gameDir, snapshot }), "studio", identity, compatibility);
    } catch {
      state.error = actionFailure("打开时装染色会话");
      host.onStatus(state.error);
    } finally {
      state.loading = false;
      render();
    }
  }

  async function openFromWardrobe(entryId: string) {
    if (!entryId || state.loading || state.saving) return;
    const gameDir = host.getGameDir();
    const sourceIdentity = host.getWardrobeSourceIdentity(entryId);
    if (!sourceIdentity) {
      host.onStatus("橱窗条目状态已变化，请刷新橱窗后重试");
      return;
    }
    const identity = wardrobeDyeSourceIdentity(gameDir, entryId, sourceIdentity);
    const compatibility = dyeWorkspaceCompatibilityIdentity(sourceIdentity);
    if (handleRetainedOpen({
      source: "wardrobe",
      identity,
      preferredLayer: "",
      open: () => openFreshWardrobe(entryId, gameDir, identity, compatibility),
    })) return;
    await openFreshWardrobe(entryId, gameDir, identity, compatibility);
  }

  async function openFreshWardrobe(
    entryId: string,
    gameDir: string,
    identity: string,
    compatibility: DyeWorkspaceCompatibilityIdentity | null,
  ) {
    state.loading = true;
    state.error = "";
    host.onStatus("正在从时装橱窗加载染色会话");
    try {
      beginSession(await host.commands.openFromWardrobe({ gameDir, entryId }), "wardrobe", identity, compatibility);
    } catch {
      state.error = actionFailure("从时装橱窗打开染色会话");
      host.onStatus(state.error);
    } finally {
      state.loading = false;
      render();
    }
  }

  function beginSession(
    view: AvatarDyeSessionView,
    source: "studio" | "wardrobe",
    identity: string,
    compatibility: DyeWorkspaceCompatibilityIdentity | null,
  ) {
    state.source = source;
    state.sourceIdentity = identity;
    state.sourceCompatibility = compatibility;
    state.pendingOpen = null;
    state.open = true;
    state.retained = false;
    state.windowClosePending = false;
    state.interaction = createDyeInteractionState();
    state.gestureBefore = null;
    state.selectedTargetId = "";
    state.collapsedTargetGroups.clear();
    state.editorMode = "swatches";
    state.presetDialog = null;
    state.presetDialogDraft = null;
    state.presetDialogError = "";
    state.previewScope = "outfit";
    state.hideSelectedMask = false;
    state.hiddenTargetIds.clear();
    state.error = "";
    stopPlayback();
    syncView(view, "open");
    resetPreviewFrame();
    host.onStatus("时装染色会话已就绪");
    render();
    void renderFrame();
    if (!state.presetLoaded && !state.presetLoading) void loadDyeColorPresets();
  }

  function handleRetainedOpen(pending: PendingDyeOpen) {
    const decision = retainedDraftOpenDecision(state.retained ? state.sourceIdentity : "", pending.identity);
    if (decision === "open_new") return false;
    if (decision === "resume") {
      resumeRetainedDraft(pending.source, pending.preferredLayer);
      return true;
    }
    state.pendingOpen = pending;
    state.retained = false;
    state.open = true;
    state.dialog = "replace_draft";
    host.onStatus("另一个染色草稿仍保留在本次运行中，请选择如何处理");
    render();
    return true;
  }

  function resumeRetainedDraft(source: "studio" | "wardrobe", preferredLayer = "") {
    if (!state.retained || !state.view) return false;
    state.source = source;
    state.open = true;
    state.retained = false;
    state.preferredLayer = preferredLayer;
    state.error = "";
    state.dialog = null;
    if (preferredLayer) {
      const preferred = state.view.targets.find((target) => target.layer_label === preferredLayer);
      if (preferred) state.selectedTargetId = preferred.opaque_target_id;
    }
    state.interaction = selectDyeImage(state.interaction, state.selectedTargetId);
    host.onStatus("已恢复本次运行保留的染色草稿");
    render();
    void renderFrame();
    return true;
  }

  function syncView(view: AvatarDyeSessionView, mode: "open" | "mutation" | "persist" = "mutation") {
    state.gestureBefore = null;
    const edits = editsFromView(view);
    if (mode === "open") {
      state.logic = createAvatarDyeState({ edits });
    } else {
      state.logic = commitDyeEdits(state.logic, edits);
      if (mode === "persist") state.logic = markDyePersisted(state.logic);
    }
    state.view = view;
    const rawSemantic = view.semantic_state && typeof view.semantic_state === "object"
      ? view.semantic_state as Record<string, unknown>
      : {};
    const semanticState = {
      ...rawSemantic,
      baked_legacy: typeof view.baked_legacy === "boolean"
        ? view.baked_legacy
        : rawSemantic.baked_legacy ?? rawSemantic.bakedLegacy,
    };
    state.interaction = restoreDyeSemanticState(state.interaction, semanticState);
    const previewableIds = new Set(
      view.targets.filter(dyeTargetPreviewAvailable).map((target) => target.opaque_target_id),
    );
    for (const targetId of state.hiddenTargetIds) {
      if (!previewableIds.has(targetId)) state.hiddenTargetIds.delete(targetId);
    }
    let selected = view.targets.find((target) => target.opaque_target_id === state.selectedTargetId);
    if (!selected && state.preferredLayer) {
      selected = view.targets.find((target) => target.layer_label === state.preferredLayer);
      state.preferredLayer = "";
    }
    selected ??= view.targets.find(dyeTargetPreviewAvailable) ?? view.targets[0];
    state.selectedTargetId = selected?.opaque_target_id ?? "";
    state.interaction = selectDyeImage(state.interaction, state.selectedTargetId);
    if (!selected) return;
    state.selectedPalette = selected.active_palette;
    ensurePaletteSelection();
    updatePreviewScopeAvailability();
    state.frame = Math.min(state.frame, Math.max(0, previewFrameCount() - 1));
  }

  function editsFromView(view: AvatarDyeSessionView): AvatarDyeEdit[] {
    const edits: AvatarDyeEdit[] = [];
    for (const target of view.targets) {
      const palette = target.palettes.find((candidate) => candidate.palette_index === target.active_palette);
      if (!palette) continue;
      for (const color of palette.colors) {
        if (sameRgb(color.current_rgba, color.original_rgba)) continue;
        edits.push({
          targetId: target.opaque_target_id,
          paletteIndex: target.active_palette,
          colorIndex: color.color_index,
          replacementRgb: [color.current_rgba[0], color.current_rgba[1], color.current_rgba[2]],
        });
      }
    }
    return edits;
  }

  async function refreshSession() {
    const sessionId = state.view?.session_id;
    if (!sessionId || state.loading || state.saving) return;
    state.loading = true;
    render();
    try {
      const view = await host.commands.getView({ sessionId });
      if (!isCurrentSession(sessionId)) return;
      syncView(view);
      resetAnimation();
      state.error = "";
    } catch {
      if (isCurrentSession(sessionId)) state.error = actionFailure("刷新染色会话");
    } finally {
      if (isCurrentSession(sessionId)) {
        state.loading = false;
        render();
        void renderFrame();
      }
    }
  }

  async function updateColors(operation: DyeColorOperation) {
    const sessionId = state.view?.session_id;
    const targetId = state.selectedTargetId;
    if (
      !sessionId
      || !targetId
      || !dyeTargetPreviewAvailable(selectedTarget() ?? {})
      || state.loading
      || state.saving
      || resourceEditingBlocked()
    ) return;
    cancelTransientBatchPreview();
    state.saving = true;
    state.error = "";
    stopPlayback();
    render();
    try {
      const view = await host.commands.updateColors({ sessionId, targetId, operation });
      if (!isCurrentSession(sessionId)) return;
      syncView(view);
      resetAnimation();
      host.onStatus("染色调整已应用到当前会话");
    } catch {
      if (isCurrentSession(sessionId)) {
        state.error = actionFailure("应用染色调整");
        host.onStatus(state.error);
      }
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
        void renderFrame();
      }
    }
  }

  function dyeAdjustmentTargets(): DyeAdjustmentTarget[] {
    return state.view?.targets ?? [];
  }

  function currentBatchMutation(): DyeBatchMutation {
    return buildDyeBatchMutation(state.interaction, dyeAdjustmentTargets()) as DyeBatchMutation;
  }

  async function commitInteractionState(previous = state.gestureBefore) {
    const sessionId = state.view?.session_id;
    const next = state.interaction;
    state.gestureBefore = null;
    if (!sessionId || state.loading || state.saving || resourceEditingBlocked()) {
      if (previous) state.interaction = previous;
      return;
    }
    if (previous && JSON.stringify(previous) === JSON.stringify(next)) {
      scheduleFrameRender();
      return;
    }
    cancelTransientBatchPreview();
    state.saving = true;
    state.error = "";
    stopPlayback();
    window.setTimeout(() => {
      if (isCurrentSession(sessionId) && state.saving) render();
    }, 0);
    try {
      const view = await host.commands.updateBatch({
        sessionId,
        mutation: currentBatchMutation(),
      });
      if (!isCurrentSession(sessionId)) return;
      syncView(view);
      resetAnimation();
      host.onStatus("染色变换已保存到当前会话");
    } catch {
      if (isCurrentSession(sessionId)) {
        if (previous) state.interaction = previous;
        state.error = actionFailure("应用染色变换");
        host.onStatus(state.error);
      }
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
        void renderFrame();
      }
    }
  }

  async function repairTarget() {
    const sessionId = state.view?.session_id;
    const target = selectedTarget();
    if (!sessionId || !target || targetResourceStatus(target) !== "changed" || state.loading || state.saving) return;
    state.saving = true;
    state.error = "";
    stopPlayback();
    render();
    try {
      const view = await host.commands.repairTarget({
        sessionId,
        targetId: target.opaque_target_id,
      });
      if (!isCurrentSession(sessionId)) return;
      syncView(view);
      resetAnimation();
      host.onStatus("该目标的旧染色已安全重新绑定；请检查预览后再保存");
    } catch {
      if (isCurrentSession(sessionId)) {
        state.error = actionFailure("修复染色目标");
        host.onStatus(state.error);
      }
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
        void renderFrame();
      }
    }
  }

  async function reset(
    scope: "all" | "target" | "part" | "colors",
    requestedColorIndices?: number[],
  ) {
    const sessionId = state.view?.session_id;
    if (!sessionId || state.loading || state.saving || (scope !== "all" && resourceEditingBlocked())) return;
    const targetId = state.selectedTargetId || undefined;
    const colorIndices = scope === "colors"
      ? [...(requestedColorIndices ?? state.logic.selectedPaletteIndices)]
      : undefined;
    state.saving = true;
    state.error = "";
    stopPlayback();
    render();
    try {
      const view = await host.commands.resetScope({ sessionId, scope, targetId, colorIndices });
      if (!isCurrentSession(sessionId)) return;
      syncView(view);
      resetAnimation();
      host.onStatus(scope === "all" ? "全部染色已恢复" : "选定范围已恢复");
    } catch {
      if (isCurrentSession(sessionId)) state.error = actionFailure("恢复染色范围");
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
        void renderFrame();
      }
    }
  }

  async function undo() {
    const sessionId = state.view?.session_id;
    if (
      !sessionId
      || state.loading
      || state.saving
      || (resourceEditingBlocked() && !currentConflictUndoAvailable())
      || !state.view?.can_undo
    ) return;
    const previous = state.logic;
    state.logic = undoDyeEdit(previous);
    state.saving = true;
    stopPlayback();
    render();
    try {
      const view = await host.commands.undo({ sessionId });
      if (!isCurrentSession(sessionId)) return;
      syncView(view);
      resetAnimation();
      host.onStatus("已撤销染色操作");
    } catch {
      if (isCurrentSession(sessionId)) {
        state.logic = previous;
        state.error = actionFailure("撤销染色操作");
      }
    } finally {
      if (isCurrentSession(sessionId)) state.saving = false;
    }
    if (!isCurrentSession(sessionId)) return;
    render();
    void renderFrame();
  }

  async function redo() {
    const sessionId = state.view?.session_id;
    if (!sessionId || state.loading || state.saving || !state.view?.can_redo) return;
    const previous = state.logic;
    state.logic = redoDyeEdit(previous);
    state.saving = true;
    stopPlayback();
    render();
    try {
      const view = await host.commands.redo({ sessionId });
      if (!isCurrentSession(sessionId)) return;
      syncView(view);
      resetAnimation();
      host.onStatus("已重做染色操作");
    } catch {
      if (isCurrentSession(sessionId)) {
        state.logic = previous;
        state.error = actionFailure("重做染色操作");
      }
    } finally {
      if (isCurrentSession(sessionId)) state.saving = false;
    }
    if (!isCurrentSession(sessionId)) return;
    render();
    void renderFrame();
  }

  async function renderFrame() {
    cancelScheduledFrameRender();
    const view = state.view;
    const target = previewAnchorTarget();
    if (!state.open || !view || !target) return;
    if (!dyeTargetPreviewAvailable(target)) {
      updateCanvasStatus("目标缺失，保留上一张有效预览");
      return;
    }
    const sessionId = view.session_id;
    const issued = issueDyeRequest(state.logic);
    state.logic = issued.state;
    state.rendering = true;
    updateCanvasStatus("正在同步渲染原图与改色后帧…");
    try {
      const comparison = await host.commands.renderComparisonFrame({
        sessionId,
        frameIndex: state.frame,
        previewScope: previewRequestScope(),
        targetId: target.opaque_target_id,
        hideSelectedMask: previewHidesSelectedMask(target),
        hiddenTargetIds: previewHiddenTargetIds(),
        requestId: issued.requestId,
      });
      if (!isCurrentSession(sessionId)) return;
      const responseId = comparison.request_id ?? comparison.requestId;
      if (responseId !== issued.requestId || !alignedDyePreviewPair(comparison)) {
        throw new Error("双画布预览响应不一致");
      }
      const acceptance = acceptDyeResponse(state.logic, responseId);
      state.logic = acceptance.state;
      if (!acceptance.accepted || !state.open) return;
      drawDyePreviewPair(comparison);
      updateCanvasStatus(previewStatusText(dyePreviewPairMissingCount(comparison)));
    } catch {
      if (isCurrentSession(sessionId) && issued.requestId === state.logic.latestRequestId) {
        state.error = actionFailure("渲染预览");
        updateCanvasStatus(state.error);
      }
    } finally {
      if (isCurrentSession(sessionId) && issued.requestId === state.logic.latestRequestId) {
        state.rendering = false;
        updateTimeline();
      }
    }
  }

  async function renderTransientBatch() {
    const view = state.view;
    const target = previewAnchorTarget();
    if (!state.open || !view || !target || !dyeTargetPreviewAvailable(target)) return;
    const sessionId = view.session_id;
    const issued = issueDyeRequest(state.logic);
    state.logic = issued.state;
    state.rendering = true;
    updateCanvasStatus("正在同步渲染原图与临时改色帧…");
    try {
      const comparison = await host.commands.previewBatchComparison({
        sessionId,
        frameIndex: state.frame,
        previewScope: previewRequestScope(),
        targetId: target.opaque_target_id,
        mutation: currentBatchMutation(),
        hideSelectedMask: previewHidesSelectedMask(target),
        hiddenTargetIds: previewHiddenTargetIds(),
        requestId: issued.requestId,
      });
      if (!isCurrentSession(sessionId)) return;
      const responseId = comparison.request_id ?? comparison.requestId;
      if (responseId !== issued.requestId || !alignedDyePreviewPair(comparison)) {
        throw new Error("双画布临时预览响应不一致");
      }
      const acceptance = acceptDyeResponse(state.logic, responseId);
      state.logic = acceptance.state;
      if (!acceptance.accepted || !state.open) return;
      drawDyePreviewPair(comparison);
      updateCanvasStatus(`${previewStatusText(dyePreviewPairMissingCount(comparison))} · 临时预览`);
    } catch {
      if (isCurrentSession(sessionId) && issued.requestId === state.logic.latestRequestId) {
        updateCanvasStatus(actionFailure("渲染临时染色预览"));
      }
    } finally {
      if (isCurrentSession(sessionId) && issued.requestId === state.logic.latestRequestId) {
        state.rendering = false;
        updateTimeline();
      }
    }
  }

  async function loadAnimation() {
    const view = state.view;
    const target = previewAnchorTarget();
    if (!state.open || !view || !target || !dyeTargetPreviewAvailable(target)) return false;
    const sessionId = view.session_id;
    const issued = issueDyeRequest(state.logic);
    state.logic = issued.state;
    state.rendering = true;
    updateCanvasStatus("正在同步加载原图与改色后动画…");
    let animation: DyeComparisonAnimationResponse | null = null;
    try {
      animation = await host.commands.renderComparisonAnimation({
        sessionId,
        previewScope: previewRequestScope(),
        targetId: target.opaque_target_id,
        hideSelectedMask: previewHidesSelectedMask(target),
        hiddenTargetIds: previewHiddenTargetIds(),
        requestId: issued.requestId,
      });
      if (!isCurrentSession(sessionId)) return false;
      if (issued.requestId !== state.logic.latestRequestId) return false;
      const responseId = animation.request_id ?? animation.requestId;
      if (responseId !== issued.requestId) throw new Error("双画布动画响应已经过期");
      const frameCount = validatedDyeComparisonAnimationFrameCount(
        scopedPreviewFrameCount(),
        animation.frame_count,
        animation.frames.length,
      );
      if (frameCount === null) throw new Error("双画布动画帧数不一致");
      if (animation.frames.some((frame) => !alignedDyePreviewPair(frame))) {
        throw new Error("双画布动画帧尺寸不一致");
      }
      const animationImageData: Array<DyePreviewPair<ImageData>> = [];
      const animationFrames: Array<DyePreviewPair<PreviewImage>> = [];
      for (const frame of animation.frames) {
        try {
          animationImageData.push({
            original: toImageData(frame.original),
            dyed: toImageData(frame.dyed),
          });
          animationFrames.push({
            original: retainedDyeAnimationFrame(frame.original),
            dyed: retainedDyeAnimationFrame(frame.dyed),
          });
        } finally {
          releaseDyePreviewPairPayload(frame);
        }
      }
      animation.frames.length = 0;
      const acceptance = acceptDyeResponse(state.logic, responseId);
      state.logic = acceptance.state;
      if (!acceptance.accepted || !state.open) return false;
      state.animationFrameCount = frameCount;
      state.animationFrames = animationFrames;
      state.animationImageData = animationImageData;
      state.frame = Math.min(state.frame, Math.max(0, state.animationFrameCount - 1));
      state.animationIndex = Math.min(state.frame, Math.max(0, state.animationFrames.length - 1));
      return state.animationFrames.length > 0;
    } catch {
      if (isCurrentSession(sessionId) && issued.requestId === state.logic.latestRequestId) {
        state.error = actionFailure("加载动画");
        updateCanvasStatus(state.error);
      }
      return false;
    } finally {
      if (animation) {
        animation.frames.forEach(releaseDyePreviewPairPayload);
        animation.frames.length = 0;
      }
      if (isCurrentSession(sessionId) && issued.requestId === state.logic.latestRequestId) {
        state.rendering = false;
        updateTimeline();
      }
    }
  }

  async function togglePlayback() {
    if (state.rendering || state.loading || state.saving) return;
    if (state.playing) {
      stopPlayback();
      render();
      return;
    }
    if (!state.animationFrames.length && !(await loadAnimation())) return;
    state.playing = true;
    state.animationIndex = Math.min(state.frame, state.animationFrames.length - 1);
    drawAnimationFrame(state.animationIndex);
    state.playTimer = window.setInterval(() => {
      if (!state.playing || !state.animationFrames.length) return;
      state.animationIndex = (state.animationIndex + 1) % state.animationFrames.length;
      state.frame = state.animationIndex;
      drawAnimationFrame(state.animationIndex);
      updateTimeline();
    }, PLAY_FRAME_MS);
    render();
  }

  function stopPlayback() {
    cancelScheduledFrameRender();
    if (state.playTimer) window.clearInterval(state.playTimer);
    state.playTimer = 0;
    state.playing = false;
  }

  function resetPreviewFrame() {
    state.frame = previewFrameIndexForProfession(
      state.view?.playable_role_key,
      scopedPreviewFrameCount(),
    );
    state.animationIndex = state.frame;
  }

  function resetAnimation() {
    cancelScheduledFrameRender();
    cancelTransientBatchPreview();
    stopPlayback();
    state.animationFrames = [];
    state.animationImageData = [];
    state.animationIndex = state.frame;
    state.animationFrameCount = 0;
    invalidatePendingDyePreview();
  }

  function scheduleFrameRender() {
    cancelScheduledFrameRender();
    invalidatePendingDyePreview();
    state.previewDebounceTimer = window.setTimeout(() => {
      state.previewDebounceTimer = 0;
      void renderFrame();
    }, PREVIEW_DEBOUNCE_MS);
  }

  function cancelScheduledFrameRender() {
    if (state.previewDebounceTimer) window.clearTimeout(state.previewDebounceTimer);
    state.previewDebounceTimer = 0;
  }

  function scheduleTransientBatchPreview() {
    cancelTransientBatchPreview();
    invalidatePendingDyePreview();
    stopPlayback();
    state.animationFrames = [];
    state.animationImageData = [];
    state.animationIndex = state.frame;
    state.animationFrameCount = 0;
    state.transientPreviewTimer = window.setTimeout(() => {
      state.transientPreviewTimer = 0;
      void renderTransientBatch();
    }, PREVIEW_DEBOUNCE_MS);
  }

  function cancelTransientBatchPreview() {
    if (state.transientPreviewTimer) window.clearTimeout(state.transientPreviewTimer);
    state.transientPreviewTimer = 0;
  }

  function invalidatePendingDyePreview() {
    state.logic = issueDyeRequest(state.logic).state;
    state.rendering = false;
    updateTimeline();
  }

  function dyePreviewPairMissingCount(pair: DyePreviewPair<PreviewImage>) {
    return new Set([
      ...(pair.original.missing_imgs ?? []),
      ...(pair.dyed.missing_imgs ?? []),
    ]).size;
  }

  function drawAnimationFrame(index: number) {
    const image = state.animationFrames[index];
    const imageData = state.animationImageData[index];
    if (!image || !imageData) return;
    drawDyePreviewPairData(image, imageData);
    updateCanvasStatus(previewStatusText(dyePreviewPairMissingCount(image)));
  }

  function drawDyePreviewPair(pair: DyePreviewPair<PreviewImage>) {
    const imageData = {
      original: toImageData(pair.original),
      dyed: toImageData(pair.dyed),
    };
    drawDyePreviewPairData(pair, imageData);
  }

  function drawDyePreviewPairData(
    image: DyePreviewPair<Pick<PreviewImage, "width" | "height">>,
    imageData: DyePreviewPair<ImageData>,
  ) {
    const originalCanvas = query<HTMLCanvasElement>("#avatarDyeOriginalCanvas");
    const dyedCanvas = query<HTMLCanvasElement>("#avatarDyeDyedCanvas");
    const originalContext = originalCanvas?.getContext("2d");
    const dyedContext = dyedCanvas?.getContext("2d");
    if (!originalCanvas || !dyedCanvas || !originalContext || !dyedContext) return;
    drawDyeCanvasImageData(
      originalCanvas,
      originalContext,
      image.original.width,
      image.original.height,
      imageData.original,
    );
    drawDyeCanvasImageData(
      dyedCanvas,
      dyedContext,
      image.dyed.width,
      image.dyed.height,
      imageData.dyed,
    );
    window.requestAnimationFrame(fitCanvasToViewport);
  }

  function drawDyeCanvasImageData(
    canvas: HTMLCanvasElement,
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    imageData: ImageData,
  ) {
    canvas.width = Math.max(1, width);
    canvas.height = Math.max(1, height);
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.putImageData(imageData, 0, 0);
  }

  function fitCanvasToViewport() {
    const canvases = [
      query<HTMLCanvasElement>("#avatarDyeOriginalCanvas"),
      query<HTMLCanvasElement>("#avatarDyeDyedCanvas"),
    ];
    if (canvases.some((canvas) => !canvas || canvas.width < 1 || canvas.height < 1)) return;
    const pairs = canvases.map((canvas) => ({
      canvas: canvas!,
      viewport: canvas!.closest<HTMLElement>(".avatar-dye-preview-side"),
    }));
    if (pairs.some(({ viewport }) => !viewport)) return;
    const scale = Math.min(...pairs.flatMap(({ canvas, viewport }) => [
      Math.max(1, viewport!.clientWidth - 16) / canvas.width,
      Math.max(1, viewport!.clientHeight - 16) / canvas.height,
    ]));
    pairs.forEach(({ canvas }) => {
      canvas.style.width = `${Math.max(1, Math.floor(canvas.width * scale))}px`;
      canvas.style.height = `${Math.max(1, Math.floor(canvas.height * scale))}px`;
    });
  }

  function render() {
    const shell = query<HTMLElement>(".shell");
    const page = query<HTMLElement>("#avatarDyePage");
    if (!shell || !page) return;
    shell.classList.toggle("dye-view", state.open);
    page.hidden = !state.open;
    if (!state.open) {
      closeDialog();
      return;
    }
    const studio = query<HTMLElement>("#studioWorkspace");
    const wardrobe = query<HTMLElement>("#wardrobePage");
    const swap = query<HTMLElement>("#outfitSwapPage");
    if (studio) studio.hidden = true;
    if (wardrobe) wardrobe.hidden = true;
    if (swap) swap.hidden = true;

    const view = state.view;
    setText("#avatarDyeTitle", `时装染色 · ${state.source === "wardrobe" ? "橱窗装扮" : "当前搭配"}`);
    setText("#avatarDyeDraftStatus", draftStatusText());
    query("#avatarDyeDraftStatus")?.classList.toggle("error", resourceBlocked() || Boolean(state.error));
    setText("#avatarDyeTargetSummary", `${view?.targets.length ?? 0} 个目标 · ${modifiedTargets().length} 个已修改`);

    const undoButton = query<HTMLButtonElement>("#avatarDyeUndo");
    const redoButton = query<HTMLButtonElement>("#avatarDyeRedo");
    const resetButton = query<HTMLButtonElement>("#avatarDyeResetAll");
    const hideMaskButton = query<HTMLButtonElement>("#avatarDyeHideSelectedMask");
    const backButton = query<HTMLButtonElement>("#avatarDyeBack");
    const refreshButton = query<HTMLButtonElement>("#avatarDyeRefresh");
    if (backButton) backButton.disabled = state.loading || state.saving;
    if (refreshButton) refreshButton.disabled = state.loading || state.saving;
    if (undoButton) undoButton.disabled = state.loading
      || state.saving
      || (resourceEditingBlocked() && !currentConflictUndoAvailable())
      || !view?.can_undo;
    if (redoButton) redoButton.disabled = state.loading || state.saving || !view?.can_redo;
    if (resetButton) {
      resetButton.disabled = state.loading
        || state.saving
        || (!hasDye() && !resourceBlocked());
    }
    if (hideMaskButton) {
      hideMaskButton.disabled = state.loading
        || state.rendering
        || state.interaction.objectMode !== "img"
        || !dyeTargetPreviewAvailable(selectedTarget() ?? {})
        || state.previewScope === "target"
        || !isMask(selectedTarget());
      hideMaskButton.classList.toggle("active", previewHidesSelectedMask(selectedTarget()));
      hideMaskButton.title = state.previewScope === "target"
        ? "当前 IMG 预览必须显示自身；请切换到整套或当前部位"
        : "仅影响预览，不写入染色配方或导出结果";
    }

    const notice = query<HTMLElement>("#avatarDyeResourceNotice");
    if (notice) {
      notice.hidden = !resourceBlocked() && !state.error;
      notice.textContent = state.error || (resourceRepairable()
        ? currentResourceChangeGuidance()
        : currentConflictUndoAvailable()
          ? "同一 IMG 与 Palette 的当前编辑发生冲突；保存和导出已冻结。请撤销刚才的操作，或明确恢复全部染色。"
        : resourceBlocked() && !resourceEditingBlocked()
          ? dyeIncompleteResourceNotice(
            state.view?.targets.map((target) => targetResourceStatus(target)) ?? [],
          )
          : "旧配方存在缺失或冲突目标；当前会话只允许查看，或明确恢复全部染色。");
    }
    document.querySelectorAll<HTMLButtonElement>("[data-dye-object-mode]").forEach((button) => {
      button.classList.toggle("active", button.dataset.dyeObjectMode === state.interaction.objectMode);
    });
    updatePreviewScopeAvailability();

    renderTargetTree();
    renderEditor();
    renderWorkspaceBar();
    updateTimeline();
    renderDialog();
  }

  function renderTargetTree() {
    const root = query<HTMLElement>("#avatarDyeTargetTree");
    if (!root) return;
    const targets = state.view?.targets ?? [];
    const groups = groupDyeTargetsByPart(targets);
    if (state.interaction.objectMode === "part") {
      root.innerHTML = `<div class="avatar-dye-part-grid">${groups.map(({ key, label, targets: members }) => {
        const active = state.interaction.selectedParts.includes(key);
        const editableCount = new Set(
          members.filter(isEditable).map((target) => canonicalDyeTargetId(target)),
        ).size;
        const changed = members.reduce((sum, target) => sum + changedColorCount(target), 0);
        return `<button class="avatar-dye-part-card${active ? " active" : ""}" data-dye-part="${escapeHtml(encodeURIComponent(key))}" aria-pressed="${active}"${editableCount ? "" : " disabled"}>
          <span class="avatar-dye-part-check">${active ? "✓" : ""}</span>
          <span><strong>${escapeHtml(label)}</strong><em>${editableCount} 个可染 IMG${changed ? ` · ${changed} 色已修改` : ""}</em></span>
        </button>`;
      }).join("")}</div>` || `<div class="avatar-dye-empty"><strong>没有可选择的部位</strong></div>`;
      return;
    }
    root.innerHTML = groups.map(({ key, label, targets: members }) => {
      const changed = members.reduce((sum, target) => sum + changedColorCount(target), 0);
      const collapsed = state.interaction.objectMode === "img" && state.collapsedTargetGroups.has(key);
      const encodedKey = encodeURIComponent(key);
      return `
        <section class="avatar-dye-target-group">
          <button class="avatar-dye-target-group-head avatar-dye-target-group-head-button"${state.interaction.objectMode === "img" ? ` data-dye-group-toggle="${escapeHtml(encodedKey)}"` : " disabled"} aria-expanded="${!collapsed}"><span class="avatar-dye-target-group-caret">${collapsed ? "▶" : "▼"}</span><div><strong>${escapeHtml(label)}</strong><span>${members.length} 个 IMG</span></div><em>${changed ? `${changed} 色已修改` : "原色"}</em></button>
          ${collapsed ? "" : `<div class="avatar-dye-target-list">${members.map((target) => targetRowHtml(target)).join("")}</div>`}
        </section>
      `;
    }).join("") || `<div class="avatar-dye-empty"><strong>当前搭配没有 IMG</strong></div>`;
  }

  function targetRowHtml(target: DyeTargetView) {
    const changed = changedColorCount(target);
    const issue = targetResourceStatus(target);
    const selected = state.interaction.objectMode === "all"
      ? isEditable(target)
      : target.opaque_target_id === state.interaction.selectedTargetId;
    const active = selected ? " active" : "";
    const mask = isMask(target) ? " mask" : "";
    const status = capabilityLabel(target);
    const paletteText = target.palettes.length ? `P${target.active_palette}` : "无 Palette";
    const rowLabel = dyeTargetTreeRowLabel(target.layer_label, targetKindLongLabel(target));
    const hidden = state.hiddenTargetIds.has(target.opaque_target_id);
    const visibilityLabel = hidden ? "显示此 IMG" : "隐藏此 IMG";
    return `
      <div class="avatar-dye-target-row-wrap${hidden ? " preview-hidden" : ""}">
        <button class="avatar-dye-target-row${active}${isEditable(target) ? "" : " readonly"}"${state.interaction.objectMode === "img" ? ` data-dye-target="${escapeHtml(target.opaque_target_id)}"` : " disabled"}>
          <span class="avatar-dye-target-kind${mask}">${escapeHtml(targetKindLabel(target))}</span>
          <span class="avatar-dye-target-copy"><strong title="${escapeHtml(rowLabel)}">${escapeHtml(rowLabel)}</strong><span>V${target.img_version} · 生效 ${paletteText} · ${target.frame_count} 帧${target.is_shared ? ` · ${target.reference_count ?? 2} 处共用` : ""}</span></span>
          <span class="avatar-dye-target-state ${capabilityClass(target)}"><strong>${escapeHtml(status)}</strong><em>${dyeTargetStateDetail(issue, changed)}</em></span>
        </button>
        <button class="avatar-dye-target-visibility${hidden ? " off" : ""}" data-dye-visibility="${escapeHtml(target.opaque_target_id)}" title="${visibilityLabel}（仅影响预览）" aria-label="${visibilityLabel}"${dyeTargetPreviewAvailable(target) ? "" : " disabled"}>${hidden ? "○" : "◉"}</button>
      </div>
    `;
  }

  function renderEditor() {
    const target = selectedTarget();
    const editor = query<HTMLElement>("#avatarDyeEditorScroll");
    if (!editor) return;
    const selected = selectedDyeTargets(state.interaction, dyeAdjustmentTargets());
    const changed = selected.reduce((sum, item) => sum + changedColorCount(item as DyeTargetView), 0);
    const tabs = query<HTMLElement>("#avatarDyeEditorTabs");
    const bakedNotice = state.interaction.bakedLegacy
      ? `<section class="avatar-dye-legacy-notice"><strong>已烘焙结果，参数不可还原</strong><span>现有颜色会作为固定基线；之后的变换仍可继续调整和撤销。</span></section>`
      : "";

    if (state.interaction.objectMode === "all") {
      setText("#avatarDyeEditorName", "全部可染 IMG");
      setText("#avatarDyeEditorMeta", `${selected.length} 个物理 IMG · 统一颜色变换 · Alpha 锁定`);
    } else if (state.interaction.objectMode === "part") {
      const labels = groupDyeTargetsByPart(state.view?.targets ?? [])
        .filter((group) => state.interaction.selectedParts.includes(group.key))
        .map((group) => group.label);
      setText("#avatarDyeEditorName", labels.length ? `部位 · ${labels.join("、")}` : "请选择染色部位");
      setText("#avatarDyeEditorMeta", `${selected.length} 个物理 IMG · 可多选部位统一变换`);
    } else if (target) {
      setText("#avatarDyeEditorName", `${targetSlotLabel(target) || "部位"} / ${targetKindLongLabel(target)}`);
      setText("#avatarDyeEditorMeta", `V${target.img_version} · 生效 P${target.active_palette} · ${target.frame_count} 帧 · ${capabilityLabel(target)} · Alpha 锁定`);
    } else {
      setText("#avatarDyeEditorName", "未选择 IMG");
      setText("#avatarDyeEditorMeta", "在左侧部位树中选择一个 IMG");
    }
    setText("#avatarDyeEditorChanged", changed ? `${changed} 色已修改` : "原色");
    query("#avatarDyeEditorChanged")?.classList.toggle("changed", changed > 0);

    if (tabs) {
      tabs.hidden = state.interaction.objectMode !== "img" || !target;
      tabs.innerHTML = tabs.hidden ? "" : `
        <button class="avatar-dye-editor-tab${state.editorMode === "swatches" ? " active" : ""}" data-dye-editor-mode="swatches">逐色修改</button>
        <button class="avatar-dye-editor-tab${state.editorMode === "transform" ? " active" : ""}" data-dye-editor-mode="transform">整表变换</button>`;
    }

    if (state.interaction.objectMode === "part" && !selected.length) {
      editor.innerHTML = `${bakedNotice}<div class="avatar-dye-empty"><strong>选择一个或多个部位</strong><span>普通单击即可多选，再在这里做统一颜色变换。</span></div>`;
      return;
    }
    if (state.interaction.objectMode !== "img") {
      editor.innerHTML = `${bakedNotice}${transformEditorHtml(selected)}`;
      bindEditorInputs(editor);
      return;
    }
    if (!target) {
      editor.innerHTML = `${bakedNotice}<div class="avatar-dye-empty"><strong>请选择主 IMG 或 Mask</strong><span>右侧会排列当前 IMG 生效色表的全部色块。</span></div>`;
      return;
    }
    const palette = activePalette(target);
    const issueHtml = targetResourceIssueHtml(target);
    if (!palette || !isEditable(target)) {
      editor.innerHTML = `${bakedNotice}${issueHtml || `<div class="avatar-dye-empty"><strong>只读目标</strong><span>此资源仍参与原色预览，但没有可编辑调色板。</span></div>`}`;
      return;
    }
    state.selectedPalette = target.active_palette;
    editor.innerHTML = `${bakedNotice}${issueHtml}${state.editorMode === "swatches" ? swatchEditorHtml(target, palette) : transformEditorHtml(selected)}`;
    bindEditorInputs(editor);
  }

  function targetResourceIssueHtml(target: DyeTargetView) {
    const status = targetResourceStatus(target);
    if (status === "changed") {
      return `<section class="avatar-dye-repair-notice warn"><div><strong>这个 IMG 的源资源已变化</strong><span>普通编辑已冻结。修复会把旧配方的全部颜色索引重新绑定到当前 Palette；索引缺失或变为透明时后端会拒绝。</span></div><button class="avatar-dye-button primary" data-dye-repair-target>修复此目标</button></section>`;
    }
    if (status === "missing") {
      return `<section class="avatar-dye-repair-notice error"><div><strong>旧配方目标已缺失</strong><span>无法自动修复。保留当前草稿并返回检查资源，或使用顶部“恢复全部”明确丢弃整套旧配方。</span></div></section>`;
    }
    if (status === "invalid") {
      return `<section class="avatar-dye-repair-notice error"><div><strong>IMG 解析失败</strong><span>资源文件存在，但其帧或调色板数据未通过安全校验；该目标不会参与预览、保存或导出。</span></div></section>`;
    }
    if (status === "conflict") {
      if (currentConflictUndoAvailable()) {
        return `<section class="avatar-dye-repair-notice error"><div><strong>当前编辑发生 Palette 冲突</strong><span>同一 IMG 与 Palette 不能保存两种不同配方；请撤销刚才的操作，或明确恢复全部染色。</span></div></section>`;
      }
      return `<section class="avatar-dye-repair-notice error"><div><strong>旧配方目标存在冲突</strong><span>为防止静默选错颜色，冲突不能自动合并；只能返回检查记录，或明确恢复全部染色。</span></div></section>`;
    }
    return "";
  }

  function transformEditorHtml(targets: readonly DyeAdjustmentTarget[]) {
    const adjustment = dyeAdjustmentForSelection(state.interaction, dyeAdjustmentTargets());
    const mixed = new Set(mixedDyeAdjustmentKeys(state.interaction, dyeAdjustmentTargets()));
    const disabled = !targets.length || resourceEditingBlocked() || state.loading || state.saving;
    const disabledAttribute = disabled ? " disabled" : "";
    const mixedLabel = (key: keyof DyeAdjustment) => mixed.has(key) ? `<em class="avatar-dye-mixed">混合</em>` : "";
    return `
      <section class="avatar-dye-editor-section avatar-dye-transform-editor">
        <div class="avatar-dye-section-head"><div><strong>颜色变换</strong><span>参数保持为绝对值；拖动后不会归零</span></div><em>${targets.length} 个 IMG</em></div>
        <label class="avatar-dye-transform-toggle"><input type="checkbox" data-dye-adjustment="enabled"${adjustment.enabled ? " checked" : ""}${disabledAttribute} /><span><strong>换成指定颜色</strong><em>保留色表内部的明暗层次</em></span>${mixedLabel("enabled")}</label>
        ${dyeColorPresetPickerHtml({
          inputMode: "transform",
          rgb: adjustment.targetColor,
          mixed: mixed.has("targetColor"),
          disabled,
        })}
        <div class="avatar-dye-adjustment-ranges">
          ${adjustmentRangeHtml("strength", "染色程度", 0, 100, adjustment.strength, "%", mixed, disabled)}
          ${adjustmentRangeHtml("brightness", "亮度", -100, 100, adjustment.brightness, "%", mixed, disabled)}
          ${adjustmentRangeHtml("hue", "偏色", -180, 180, adjustment.hue, "°", mixed, disabled)}
          ${adjustmentRangeHtml("saturation", "鲜艳度", -100, 100, adjustment.saturation, "%", mixed, disabled)}
          ${adjustmentRangeHtml("lightness", "明暗", -100, 100, adjustment.lightness, "%", mixed, disabled)}
          ${adjustmentRangeHtml("contrast", "层次", -100, 100, adjustment.contrast, "%", mixed, disabled)}
        </div>
        <label class="avatar-dye-transform-toggle"><input type="checkbox" data-dye-adjustment="preserveLuminance"${adjustment.preserveLuminance ? " checked" : ""}${disabledAttribute} /><span><strong>保留原有阴影和高光</strong><em>换色时沿用每个色块原本的亮度</em></span>${mixedLabel("preserveLuminance")}</label>
        <div class="avatar-dye-section-actions"><button class="avatar-dye-button" data-dye-reset-adjustment${disabledAttribute}>恢复本层变换</button></div>
      </section>`;
  }

  function adjustmentRangeHtml(
    key: Extract<keyof DyeAdjustment, "strength" | "brightness" | "hue" | "saturation" | "lightness" | "contrast">,
    label: string,
    minimum: number,
    maximum: number,
    value: number,
    suffix: string,
    mixed: Set<string>,
    disabled: boolean,
  ) {
    const id = `avatarDyeAdjustment${key[0].toUpperCase()}${key.slice(1)}`;
    return `<label class="avatar-dye-adjustment-range"><span>${label}${mixed.has(key) ? `<em class="avatar-dye-mixed">混合</em>` : ""}</span><div><input id="${id}" type="range" min="${minimum}" max="${maximum}" value="${value}" data-dye-adjustment="${key}"${disabled ? " disabled" : ""} /><output for="${id}" data-suffix="${suffix}">${value > 0 ? "+" : ""}${value}${suffix}</output></div></label>`;
  }

  function dyeColorPresetPickerHtml(options: {
    inputMode: "transform" | "exact";
    rgb: readonly number[];
    mixed: boolean;
    disabled: boolean;
    colorIndex?: number;
    originalRgb?: readonly number[];
    clearExactDisabled?: boolean;
    rgbEncoding?: "argb1555" | "argb4444" | "argb8888" | null;
  }) {
    const catalog = state.presetCatalog;
    const rgbHex = dyeRgbHex(options.rgb);
    const match = options.mixed
      ? null
      : findMatchingDyeColorPreset(catalog, options.rgb, options.rgbEncoding);
    const sourceLabel = dyeColorPresetMatchLabel(
      catalog,
      options.rgb,
      options.mixed,
      options.rgbEncoding,
    );
    const controlsDisabled = options.disabled || state.presetSaving;
    const presetUnavailable = controlsDisabled || state.presetLoading || !state.presetLoaded;
    const customReadOnly = Boolean(catalog.storageError || state.presetLoadError);
    const colorIndex = options.colorIndex ?? 0;
    const colorInputs = options.inputMode === "transform"
      ? `<input type="color" aria-label="选择染色颜色" data-dye-adjustment="targetColor" value="${rgbHex}"${controlsDisabled ? " disabled" : ""} /><input aria-label="染色颜色 HEX" data-dye-adjustment-hex value="${rgbHex}" maxlength="7" spellcheck="false"${controlsDisabled ? " disabled" : ""} />`
      : `<input type="color" aria-label="选择染色颜色" data-dye-exact-color data-dye-color-index="${colorIndex}" value="${rgbHex}"${controlsDisabled ? " disabled" : ""} /><input aria-label="染色颜色 HEX" data-dye-exact-hex data-dye-color-index="${colorIndex}" value="${rgbHex}" maxlength="7" spellcheck="false"${controlsDisabled ? " disabled" : ""} />`;
    const original = options.originalRgb
      ? `<div class="avatar-dye-preset-original"><span>原始颜色（只读）</span><i style="--dye-preset-color:${dyeRgbHex(options.originalRgb)}"></i><code>${dyeRgbHex(options.originalRgb)}</code></div>`
      : "";
    const builtinHtml = state.presetTab === "builtin"
      ? `<div class="avatar-dye-preset-grid" aria-label="基础染色剂色表">${catalog.builtinPresets.map((preset) => {
          const selected = match?.source === "builtin" && match.preset.id === preset.id;
          return `<button class="avatar-dye-preset-swatch${selected ? " selected" : ""}" style="--dye-preset-color:${dyeRgbHex(preset.rgb)}" data-dye-preset-id="${escapeHtml(encodeURIComponent(preset.id))}" title="${escapeHtml(`${preset.name} · ${dyeRgbHex(preset.rgb)}`)}" aria-label="${escapeHtml(`${preset.name} ${dyeRgbHex(preset.rgb)}`)}" aria-pressed="${selected}"${presetUnavailable ? " disabled" : ""}><span>${escapeHtml(preset.name)}</span></button>`;
        }).join("")}</div>`
      : "";
    const customHtml = state.presetTab === "custom"
      ? `<div class="avatar-dye-custom-presets" aria-label="我的染色剂预设">${catalog.customPresets.length
          ? catalog.customPresets.map((preset) => {
              const selected = match?.source === "custom" && match.preset.id === preset.id;
              const encodedId = escapeHtml(encodeURIComponent(preset.id));
              return `<article class="avatar-dye-custom-preset${selected ? " selected" : ""}"><button class="avatar-dye-custom-preset-apply" data-dye-preset-id="${encodedId}" aria-pressed="${selected}"${presetUnavailable ? " disabled" : ""}><i style="--dye-preset-color:${dyeRgbHex(preset.rgb)}"></i><span><strong>${escapeHtml(preset.name)}</strong><em>${dyeRgbHex(preset.rgb)}</em></span></button><button class="avatar-dye-custom-preset-edit" data-dye-preset-edit="${encodedId}" title="编辑 ${escapeHtml(preset.name)}" aria-label="编辑 ${escapeHtml(preset.name)}"${controlsDisabled || customReadOnly ? " disabled" : ""}>•••</button></article>`;
            }).join("")
          : `<div class="avatar-dye-preset-empty"><strong>还没有自定义预设</strong><span>将当前染色颜色保存后会显示在这里。</span></div>`}</div>`
      : "";
    const loadNotice = state.presetLoadError
      ? `<div class="avatar-dye-preset-notice error" role="alert"><span>${escapeHtml(state.presetLoadError)}</span><button data-dye-preset-reload>重新加载</button></div>`
      : state.presetLoading
        ? `<div class="avatar-dye-preset-notice"><span>正在加载染色剂色表…</span></div>`
        : catalog.storageError
          ? `<div class="avatar-dye-preset-notice error" role="alert"><span>${escapeHtml(catalog.storageError)}</span><em>基础色仍可使用；修复配置前不会覆盖“我的预设”。</em></div>`
          : "";
    return `
      <section class="avatar-dye-preset-panel">
        <div class="avatar-dye-preset-heading"><div><strong>染色颜色</strong><span>手动调色或选择预设，修改的是同一个颜色</span></div><em data-dye-preset-match>${escapeHtml(sourceLabel)}</em></div>
        ${original}
        <div class="avatar-dye-preset-color-row">${colorInputs}<button class="avatar-dye-button" data-dye-preset-create="${rgbHex}"${options.mixed || controlsDisabled || customReadOnly || !state.presetLoaded ? " disabled" : ""}>保存为我的预设</button>${options.inputMode === "exact" ? `<button class="avatar-dye-button" data-dye-clear-exact data-dye-color-index="${colorIndex}"${options.clearExactDisabled || controlsDisabled ? " disabled" : ""}>恢复此色块</button>` : ""}</div>
        <div class="avatar-dye-preset-tabs" role="tablist" aria-label="染色剂预设来源"><button role="tab" class="${state.presetTab === "builtin" ? "active" : ""}" data-dye-preset-tab="builtin" aria-selected="${state.presetTab === "builtin"}">基础 ${catalog.builtinPresets.length}</button><button role="tab" class="${state.presetTab === "custom" ? "active" : ""}" data-dye-preset-tab="custom" aria-selected="${state.presetTab === "custom"}">我的预设 ${catalog.customPresets.length}</button></div>
        ${loadNotice}
        ${builtinHtml}${customHtml}
      </section>
      ${dyeColorPresetDialogHtml()}`;
  }

  function dyeColorPresetDialogHtml() {
    const mode = state.presetDialog;
    const draft = state.presetDialogDraft;
    if (!mode || !draft) return "";
    const error = state.presetDialogError
      ? `<div class="avatar-dye-preset-dialog-error" role="alert"><strong>无法完成操作</strong><span>${escapeHtml(state.presetDialogError)}</span></div>`
      : "";
    if (mode === "delete") {
      return `<div class="avatar-dye-preset-dialog-backdrop"><section class="avatar-dye-preset-dialog" role="dialog" aria-modal="true" aria-labelledby="avatarDyePresetDialogTitle"><div class="avatar-dye-preset-dialog-head"><div><span>Dye Preset</span><strong id="avatarDyePresetDialogTitle">删除自定义预设</strong></div></div><div class="avatar-dye-preset-dialog-body"><p>确定删除“${escapeHtml(draft.name)}”吗？已应用到装扮的颜色不会改变。</p>${error}</div><div class="avatar-dye-preset-dialog-actions"><button class="avatar-dye-button" data-dye-preset-dialog-action="cancel"${state.presetSaving ? " disabled" : ""}>取消</button><button class="avatar-dye-button danger" data-dye-preset-dialog-action="delete-confirm"${state.presetSaving ? " disabled" : ""}>${state.presetSaving ? "正在删除…" : "删除预设"}</button></div></section></div>`;
    }
    const title = mode === "create" ? "保存自定义染色剂" : "编辑自定义染色剂";
    return `<div class="avatar-dye-preset-dialog-backdrop"><section class="avatar-dye-preset-dialog" role="dialog" aria-modal="true" aria-labelledby="avatarDyePresetDialogTitle"><div class="avatar-dye-preset-dialog-head"><div><span>Dye Preset</span><strong id="avatarDyePresetDialogTitle">${title}</strong></div></div><div class="avatar-dye-preset-dialog-body"><label><span>预设名称</span><input data-dye-preset-draft-name maxlength="40" value="${escapeHtml(draft.name)}" spellcheck="false"${state.presetSaving ? " disabled" : ""} /></label><label><span>染色颜色</span><div class="avatar-dye-preset-dialog-color"><input type="color" data-dye-preset-draft-color value="${draft.rgbHex}"${state.presetSaving ? " disabled" : ""} /><input data-dye-preset-draft-hex maxlength="7" value="${draft.rgbHex}" spellcheck="false"${state.presetSaving ? " disabled" : ""} /></div></label>${error}</div><div class="avatar-dye-preset-dialog-actions">${mode === "edit" ? `<button class="avatar-dye-button danger" data-dye-preset-dialog-action="delete-open"${state.presetSaving ? " disabled" : ""}>删除</button>` : ""}<span></span><button class="avatar-dye-button" data-dye-preset-dialog-action="cancel"${state.presetSaving ? " disabled" : ""}>取消</button><button class="avatar-dye-button primary" data-dye-preset-dialog-action="save"${state.presetSaving ? " disabled" : ""}>${state.presetSaving ? "正在保存…" : "保存预设"}</button></div></section></div>`;
  }

  function swatchEditorHtml(target: DyeTargetView, palette: DyePaletteView) {
    const colors = palette.colors.slice().sort((left, right) => left.color_index - right.color_index);
    const selectedIndex = state.logic.selectedPaletteIndices[0] ?? initialColorIndex(palette);
    const selectedColor = colors.find((color) => color.color_index === selectedIndex) ?? colors[0];
    const canonicalId = canonicalDyeTargetId(target);
    const override = selectedColor
      ? state.interaction.exactOverrides[canonicalId]?.[String(selectedColor.color_index)]
      : undefined;
    const exactRgb = override ?? selectedColor?.current_rgba.slice(0, 3) ?? [0, 0, 0];
    const disabled = !selectedColor
      || selectedColor.current_rgba[3] === 0
      || resourceEditingBlocked()
      || state.loading
      || state.saving;
    return `
      <section class="avatar-dye-editor-section avatar-dye-direct-palette">
        <div class="avatar-dye-section-head"><div><strong>当前 IMG 全部色块</strong><span>按 ColorBoard 索引排列；单击一个色块直接修改</span></div><em>${colors.length} 色</em></div>
        <div class="avatar-dye-palette-legend"><span><i class="used"></i>已使用</span><span><i class="modified"></i>已修改</span><span><i class="transparent"></i>透明 / Alpha 锁定</span></div>
        <div class="avatar-dye-palette-grid">${colors.map((color) => {
          const classes = ["avatar-dye-swatch"];
          if (color.used_pixel_count === 0) classes.push("unused");
          if (color.current_rgba[3] === 0) classes.push("transparent");
          if (!sameRgb(color.current_rgba, color.original_rgba)) classes.push("modified");
          if (color.color_index === selectedColor?.color_index) classes.push("selected");
          return `<button class="${classes.join(" ")}" style="--dye-color:${rgbaCss(color.current_rgba)}" data-dye-swatch="${color.color_index}" title="索引 ${color.color_index} · ${formatPixels(color.used_pixel_count)} 像素 · Alpha ${color.current_rgba[3]}"${color.current_rgba[3] === 0 ? " disabled" : ""}><span>${String(color.color_index).padStart(3, "0")}</span></button>`;
        }).join("")}</div>
      </section>
      <section class="avatar-dye-editor-section avatar-dye-exact-card">
        <div class="avatar-dye-section-head"><div><strong>色块 ${String(selectedColor?.color_index ?? 0).padStart(3, "0")}</strong><span>${selectedColor ? `${formatPixels(selectedColor.used_pixel_count)} 像素 · Alpha ${selectedColor.current_rgba[3]}` : "无颜色"}</span></div><em>${override ? "已单独修改" : "跟随整表变换"}</em></div>
        ${dyeColorPresetPickerHtml({
          inputMode: "exact",
          rgb: exactRgb,
          mixed: false,
          disabled,
          colorIndex: selectedColor?.color_index ?? 0,
          originalRgb: selectedColor?.original_rgba.slice(0, 3),
          clearExactDisabled: !override,
          rgbEncoding: palette.rgb_encoding,
        })}
      </section>`;
  }

  function renderWorkspaceBar() {
    const workspace = host.getWorkspace();
    const current = state.view?.current ?? null;
    const target = state.view?.target ?? null;
    const currentRoot = query<HTMLElement>("#avatarDyeCurrentSummary");
    const targetRoot = query<HTMLElement>("#avatarDyeTargetWorkspaceSummary");
    if (currentRoot) currentRoot.innerHTML = workspaceSummaryHtml("CURRENT", "当前穿戴", current, workspace.current);
    if (targetRoot) targetRoot.innerHTML = workspaceSummaryHtml("TARGET", "目标外观", target, workspace.target);
    const blocked = (!hasDye() && !isDirty()) || resourceBlocked() || state.loading || state.saving;
    const wardrobeButton = query<HTMLButtonElement>("#avatarDyeSaveWardrobe");
    const workspaceButton = query<HTMLButtonElement>("#avatarDyeSaveWorkspace");
    const swapButton = query<HTMLButtonElement>("#avatarDyeGoSwap");
    const exportButton = query<HTMLButtonElement>("#avatarDyeExport");
    if (wardrobeButton) wardrobeButton.disabled = blocked || !host.canUseWardrobe();
    if (workspaceButton) workspaceButton.disabled = blocked || !host.canUseWardrobe();
    if (swapButton) swapButton.disabled = blocked || !host.canUseWardrobe();
    if (exportButton) exportButton.disabled = !hasDye() || resourceBlocked() || state.loading || state.saving;
  }

  function workspaceSummaryHtml(
    role: string,
    label: string,
    view: DyeWorkspaceSummaryView | null,
    fallback: DyeSnapshotSummary | null,
  ) {
    const hasDyeValue = view?.has_dye ?? fallback?.hasDye ?? false;
    const slots = view?.occupied_slot_count ?? fallback?.occupiedSlotCount ?? 0;
    const dyedSlots = view?.dyed_slot_count ?? fallback?.dyedSlotCount ?? 0;
    const dyedMasks = view?.dyed_mask_count ?? fallback?.dyedMaskCount ?? 0;
    const dyeStatus = view?.dye_status ?? fallback?.dyeStatus ?? (hasDyeValue ? "unchecked" : "no_dye");
    const status = dyeWorkspaceStatusPresentation(dyeStatus, hasDyeValue);
    const name = view?.label || fallback?.label || "尚未保存";
    return `<span class="avatar-dye-summary-role">${role}</span><div><small>${label}</small><strong>${escapeHtml(name)}</strong><em>${slots} 个部位 · ${hasDyeValue ? `染色 ${dyedSlots} 部位 / Mask ${dyedMasks}` : "原色"}</em></div>${hasDyeValue ? `<b class="wardrobe-dye-badge ${status.tone}">${status.label}</b>` : ""}`;
  }

  function updateTimeline() {
    const target = previewAnchorTarget();
    const previewAvailable = Boolean(target && dyeTargetPreviewAvailable(target));
    const input = query<HTMLInputElement>("#avatarDyeFrame");
    const button = query<HTMLButtonElement>("#avatarDyePlay");
    const frameCount = previewFrameCount();
    const max = Math.max(0, frameCount - 1);
    state.frame = Math.min(state.frame, max);
    if (input) {
      input.max = String(max);
      input.value = String(state.frame);
      input.disabled = !previewAvailable || frameCount === 0 || state.rendering;
    }
    if (button) {
      button.textContent = state.playing ? "Ⅱ" : "▶";
      button.disabled = !previewAvailable || frameCount === 0 || state.rendering;
    }
    setText("#avatarDyeFrameLabel", previewAvailable && frameCount ? `帧 ${state.frame + 1} / ${frameCount}` : "帧 0 / 0");
    setText("#avatarDyePreviewMeta", target ? `${state.playing ? "动画" : "静态帧"} · ${targetKindLongLabel(target)} · V${target.img_version}` : "静态帧");
  }

  function scheduleGestureRollback() {
    const previous = state.gestureBefore;
    if (!previous) return;
    window.setTimeout(() => {
      if (state.gestureBefore !== previous) return;
      state.interaction = previous;
      state.gestureBefore = null;
      resetAnimation();
      render();
      void renderFrame();
    }, 0);
  }

  function bindEditorInputs(root: HTMLElement) {
    root.querySelectorAll<HTMLInputElement>("input[data-dye-adjustment]").forEach((input) => {
      const output = root.querySelector<HTMLOutputElement>(`output[for="${input.id}"]`);
      const update = () => { if (output) output.textContent = `${Number(input.value) > 0 ? "+" : ""}${input.value}${output.dataset.suffix ?? ""}`; };
      const updateState = () => {
        state.gestureBefore ??= state.interaction;
        const key = input.dataset.dyeAdjustment as keyof DyeAdjustment;
        let patch: Partial<DyeAdjustment>;
        if (key === "enabled" || key === "preserveLuminance") patch = { [key]: input.checked };
        else if (key === "targetColor") {
          const rgb = parseRgbHex(input.value);
          if (!rgb) return false;
          patch = { targetColor: rgb, enabled: true };
          const hex = root.querySelector<HTMLInputElement>("[data-dye-adjustment-hex]");
          if (hex) hex.value = dyeRgbHex(rgb);
          updateDyePresetMatchLabel(root, rgb);
        } else patch = { [key]: Number(input.value) } as Partial<DyeAdjustment>;
        state.interaction = setDyeAdjustmentForSelection(state.interaction, dyeAdjustmentTargets(), patch);
        return true;
      };
      input.addEventListener("input", () => {
        if (!updateState()) return;
        update();
        scheduleTransientBatchPreview();
      });
      input.addEventListener("change", () => {
        if (!updateState()) return;
        void commitInteractionState();
      });
      input.addEventListener("blur", () => scheduleGestureRollback());
      update();
    });

    const adjustmentHex = root.querySelector<HTMLInputElement>("[data-dye-adjustment-hex]");
    const updateAdjustmentHex = () => {
      if (!adjustmentHex) return false;
      const rgb = parseRgbHex(adjustmentHex.value);
      if (!rgb) return false;
      state.gestureBefore ??= state.interaction;
      state.interaction = setDyeAdjustmentForSelection(
        state.interaction,
        dyeAdjustmentTargets(),
        { targetColor: rgb, enabled: true },
      );
      const color = root.querySelector<HTMLInputElement>('[data-dye-adjustment="targetColor"]');
      if (color) color.value = dyeRgbHex(rgb);
      updateDyePresetMatchLabel(root, rgb);
      return true;
    };
    adjustmentHex?.addEventListener("input", () => {
      if (updateAdjustmentHex()) scheduleTransientBatchPreview();
    });
    adjustmentHex?.addEventListener("change", () => {
      if (updateAdjustmentHex()) void commitInteractionState();
      else host.onStatus("请输入 #RRGGBB 格式的染色颜色");
    });
    adjustmentHex?.addEventListener("blur", () => scheduleGestureRollback());

    const exactColor = root.querySelector<HTMLInputElement>("[data-dye-exact-color]");
    const exactHex = root.querySelector<HTMLInputElement>("[data-dye-exact-hex]");
    const updateExact = (value: string) => {
      const rgb = parseRgbHex(value);
      const target = selectedTarget();
      const palette = activePalette(target);
      const colorIndex = Number(exactColor?.dataset.dyeColorIndex ?? exactHex?.dataset.dyeColorIndex);
      if (!rgb || !target || !Number.isInteger(colorIndex)) return false;
      const encodedRgb = quantizeDyeRgbForEncoding(rgb, palette?.rgb_encoding);
      state.gestureBefore ??= state.interaction;
      state.interaction = setExactDyeOverride(
        state.interaction,
        canonicalDyeTargetId(target),
        colorIndex,
        encodedRgb,
      );
      const normalized = dyeRgbHex(encodedRgb);
      if (exactColor) exactColor.value = normalized;
      if (exactHex) exactHex.value = normalized;
      updateDyePresetMatchLabel(root, encodedRgb, palette?.rgb_encoding);
      return true;
    };
    exactColor?.addEventListener("input", () => {
      if (updateExact(exactColor.value)) scheduleTransientBatchPreview();
    });
    exactColor?.addEventListener("change", () => {
      if (updateExact(exactColor.value)) void commitInteractionState();
    });
    exactColor?.addEventListener("blur", () => scheduleGestureRollback());
    exactHex?.addEventListener("input", () => {
      if (updateExact(exactHex.value)) scheduleTransientBatchPreview();
    });
    exactHex?.addEventListener("change", () => {
      if (updateExact(exactHex.value)) void commitInteractionState();
      else host.onStatus("请输入 #RRGGBB 格式的染色颜色");
    });
    exactHex?.addEventListener("blur", () => scheduleGestureRollback());

    const presetName = root.querySelector<HTMLInputElement>("[data-dye-preset-draft-name]");
    presetName?.addEventListener("input", () => {
      if (state.presetDialogDraft) state.presetDialogDraft.name = presetName.value;
    });
    const presetColor = root.querySelector<HTMLInputElement>("[data-dye-preset-draft-color]");
    const presetHex = root.querySelector<HTMLInputElement>("[data-dye-preset-draft-hex]");
    const updatePresetDraftColor = (value: string) => {
      const rgb = parseRgbHex(value);
      if (!rgb || !state.presetDialogDraft) return false;
      const normalized = dyeRgbHex(rgb);
      state.presetDialogDraft.rgbHex = normalized;
      if (presetColor) presetColor.value = normalized;
      if (presetHex) presetHex.value = normalized;
      return true;
    };
    presetColor?.addEventListener("input", () => updatePresetDraftColor(presetColor.value));
    presetHex?.addEventListener("input", () => updatePresetDraftColor(presetHex.value));
    presetHex?.addEventListener("change", () => {
      if (!updatePresetDraftColor(presetHex.value)) {
        state.presetDialogError = "请输入 #RRGGBB 格式的染色颜色";
        renderEditor();
      }
    });
  }

  function updateDyePresetMatchLabel(
    root: HTMLElement,
    rgb: readonly number[],
    rgbEncoding: DyePaletteView["rgb_encoding"] = null,
  ) {
    const label = root.querySelector<HTMLElement>("[data-dye-preset-match]");
    if (label) {
      label.textContent = dyeColorPresetMatchLabel(state.presetCatalog, rgb, false, rgbEncoding);
    }
    const createButton = root.querySelector<HTMLButtonElement>("[data-dye-preset-create]");
    if (createButton) createButton.dataset.dyePresetCreate = dyeRgbHex(rgb);
  }

  function currentExactPresetSelection() {
    if (state.interaction.objectMode !== "img" || state.editorMode !== "swatches") return null;
    const target = selectedTarget();
    const palette = activePalette(target);
    const colorIndex = state.logic.selectedPaletteIndices[0] ?? (palette ? initialColorIndex(palette) : -1);
    if (!target || !Number.isInteger(colorIndex) || colorIndex < 0) return null;
    const canonicalTargetId = canonicalDyeTargetId(target);
    const color = palette?.colors.find((candidate) => candidate.color_index === colorIndex);
    const currentRgb = state.interaction.exactOverrides[canonicalTargetId]?.[String(colorIndex)]
      ?? color?.current_rgba.slice(0, 3);
    return {
      canonicalTargetId,
      colorIndex,
      currentRgb,
      rgbEncoding: palette?.rgb_encoding,
    };
  }

  function applySelectedDyeColorPreset(preset: DyeColorPreset) {
    if (state.loading || state.saving || state.presetSaving || resourceEditingBlocked()) return;
    const previous = state.interaction;
    const next = applyDyeColorPreset(
      previous,
      dyeAdjustmentTargets(),
      preset.rgb,
      currentExactPresetSelection(),
    );
    if (next === previous) return;
    state.interaction = next;
    void commitInteractionState(previous);
  }

  function openDyePresetDialog(mode: Exclude<DyePresetDialogMode, null>, preset?: DyeColorPreset, rgbHex = "") {
    state.presetDialog = mode;
    state.presetDialogError = "";
    state.presetDialogDraft = {
      presetId: preset?.id ?? "",
      name: preset?.name ?? "",
      rgbHex: preset ? dyeRgbHex(preset.rgb) : rgbHex,
    };
    renderEditor();
  }

  function closeDyePresetDialog() {
    if (state.presetSaving) return;
    state.presetDialog = null;
    state.presetDialogDraft = null;
    state.presetDialogError = "";
    renderEditor();
  }

  async function refreshDyeColorPresetsAfterMutationFailure(operationError: unknown) {
    const operationMessage = dyePresetErrorMessage(operationError);
    try {
      state.presetCatalog = normalizeDyeColorPresetCatalog(
        await host.commands.loadDyeColorPresets(),
      );
      state.presetLoaded = true;
      state.presetLoadError = "";
      state.presetDialogError = `${operationMessage}；预设列表已刷新，请直接重试`;
    } catch (refreshError) {
      const refreshMessage = dyePresetErrorMessage(refreshError);
      state.presetLoadError = refreshMessage;
      state.presetDialogError = `${operationMessage}；重新加载预设失败：${refreshMessage}`;
    }
  }

  async function saveDyeColorPreset() {
    const mode = state.presetDialog;
    const draft = state.presetDialogDraft;
    if ((mode !== "create" && mode !== "edit") || !draft || state.presetSaving) return;
    const name = draft.name.trim();
    const rgb = parseRgbHex(draft.rgbHex);
    if (!name) {
      state.presetDialogError = "请输入预设名称";
      renderEditor();
      return;
    }
    if (!rgb) {
      state.presetDialogError = "请输入 #RRGGBB 格式的染色颜色";
      renderEditor();
      return;
    }
    state.presetSaving = true;
    state.presetDialogError = "";
    renderEditor();
    try {
      const catalog = mode === "create"
        ? await host.commands.createDyeColorPreset({
            expectedRevision: state.presetCatalog.revision,
            name,
            rgb,
          })
        : await host.commands.updateDyeColorPreset({
            expectedRevision: state.presetCatalog.revision,
            id: draft.presetId,
            name,
            rgb,
          });
      state.presetCatalog = normalizeDyeColorPresetCatalog(catalog);
      state.presetLoaded = true;
      state.presetDialog = null;
      state.presetDialogDraft = null;
      host.onStatus(mode === "create" ? "自定义染色剂预设已保存" : "自定义染色剂预设已更新");
    } catch (error) {
      await refreshDyeColorPresetsAfterMutationFailure(error);
    } finally {
      state.presetSaving = false;
      renderEditor();
    }
  }

  async function deleteDyeColorPreset() {
    const draft = state.presetDialogDraft;
    if (state.presetDialog !== "delete" || !draft?.presetId || state.presetSaving) return;
    state.presetSaving = true;
    state.presetDialogError = "";
    renderEditor();
    try {
      state.presetCatalog = normalizeDyeColorPresetCatalog(await host.commands.deleteDyeColorPreset({
        expectedRevision: state.presetCatalog.revision,
        id: draft.presetId,
      }));
      state.presetLoaded = true;
      state.presetDialog = null;
      state.presetDialogDraft = null;
      host.onStatus("自定义染色剂预设已删除；当前染色保持不变");
    } catch (error) {
      await refreshDyeColorPresetsAfterMutationFailure(error);
    } finally {
      state.presetSaving = false;
      renderEditor();
    }
  }

  function handlePageClick(event: Event) {
    const element = event.target instanceof Element ? event.target : null;
    if (!element) return;
    if (state.gestureBefore) {
      state.interaction = state.gestureBefore;
      state.gestureBefore = null;
      cancelTransientBatchPreview();
      scheduleFrameRender();
    }
    const objectModeButton = element.closest<HTMLButtonElement>("[data-dye-object-mode]");
    const partButton = element.closest<HTMLButtonElement>("[data-dye-part]");
    const targetButton = element.closest<HTMLButtonElement>("[data-dye-target]");
    const scopeButton = element.closest<HTMLButtonElement>("[data-dye-preview-scope]");
    const editorButton = element.closest<HTMLButtonElement>("[data-dye-editor-mode]");
    const swatchButton = element.closest<HTMLButtonElement>("[data-dye-swatch]");
    const groupButton = element.closest<HTMLButtonElement>("[data-dye-group-toggle]");
    const visibilityButton = element.closest<HTMLButtonElement>("[data-dye-visibility]");
    const presetTabButton = element.closest<HTMLButtonElement>("[data-dye-preset-tab]");
    const presetButton = element.closest<HTMLButtonElement>("[data-dye-preset-id]");
    const presetCreateButton = element.closest<HTMLButtonElement>("[data-dye-preset-create]");
    const presetEditButton = element.closest<HTMLButtonElement>("[data-dye-preset-edit]");
    const presetDialogButton = element.closest<HTMLButtonElement>("[data-dye-preset-dialog-action]");

    if (presetDialogButton) {
      const action = presetDialogButton.dataset.dyePresetDialogAction;
      if (action === "cancel") closeDyePresetDialog();
      else if (action === "save") void saveDyeColorPreset();
      else if (action === "delete-open" && state.presetDialogDraft) {
        state.presetDialog = "delete";
        state.presetDialogError = "";
        renderEditor();
      } else if (action === "delete-confirm") void deleteDyeColorPreset();
      return;
    }
    if (presetTabButton) {
      state.presetTab = presetTabButton.dataset.dyePresetTab === "custom" ? "custom" : "builtin";
      renderEditor();
      return;
    }
    if (element.closest("[data-dye-preset-reload]")) {
      void loadDyeColorPresets();
      return;
    }
    if (presetButton) {
      const encodedId = presetButton.dataset.dyePresetId;
      if (!encodedId) return;
      const match = findDyeColorPresetById(state.presetCatalog, decodeURIComponent(encodedId));
      if (match) applySelectedDyeColorPreset(match.preset);
      return;
    }
    if (presetCreateButton) {
      const rgbHex = presetCreateButton.dataset.dyePresetCreate;
      if (rgbHex && parseRgbHex(rgbHex)) openDyePresetDialog("create", undefined, rgbHex);
      return;
    }
    if (presetEditButton) {
      const encodedId = presetEditButton.dataset.dyePresetEdit;
      if (!encodedId) return;
      const match = findDyeColorPresetById(state.presetCatalog, decodeURIComponent(encodedId));
      if (match?.source === "custom") openDyePresetDialog("edit", match.preset);
      return;
    }

    if (objectModeButton) {
      const objectMode = objectModeButton.dataset.dyeObjectMode as DyeObjectMode;
      state.interaction = setDyeObjectMode(state.interaction, objectMode);
      if (objectMode === "all") state.previewScope = "outfit";
      if (objectMode === "part") {
        state.previewScope = "part";
        if (!state.interaction.selectedParts.length) {
          const slot = selectedTarget()?.slot;
          if (slot) state.interaction = toggleDyePart(state.interaction, slot);
        }
      }
      if (objectMode === "img") state.previewScope = "target";
      resetPreviewFrame();
      resetAnimation();
      render();
      void renderFrame();
      return;
    }
    if (partButton) {
      const encoded = partButton.dataset.dyePart;
      if (!encoded) return;
      const wasEmpty = state.interaction.selectedParts.length === 0;
      state.interaction = toggleDyePart(state.interaction, decodeURIComponent(encoded));
      if (wasEmpty && state.interaction.selectedParts.length) state.previewScope = "part";
      resetPreviewFrame();
      resetAnimation();
      render();
      void renderFrame();
      return;
    }
    if (groupButton) {
      const encoded = groupButton.dataset.dyeGroupToggle;
      if (!encoded) return;
      const key = decodeURIComponent(encoded);
      if (state.collapsedTargetGroups.has(key)) state.collapsedTargetGroups.delete(key);
      else state.collapsedTargetGroups.add(key);
      renderTargetTree();
      return;
    }
    if (visibilityButton) {
      const targetId = visibilityButton.dataset.dyeVisibility;
      const target = state.view?.targets.find((candidate) => candidate.opaque_target_id === targetId);
      if (!target || !dyeTargetPreviewAvailable(target)) return;
      if (state.hiddenTargetIds.has(target.opaque_target_id)) {
        state.hiddenTargetIds.delete(target.opaque_target_id);
      } else {
        state.hiddenTargetIds.add(target.opaque_target_id);
      }
      if (target.opaque_target_id === state.selectedTargetId && isMask(target)) {
        state.hideSelectedMask = state.hiddenTargetIds.has(target.opaque_target_id);
      }
      resetAnimation();
      render();
      void renderFrame();
      return;
    }
    if (targetButton) {
      const target = state.view?.targets.find((candidate) => candidate.opaque_target_id === targetButton.dataset.dyeTarget);
      if (!target) return;
      state.selectedTargetId = target.opaque_target_id;
      state.interaction = selectDyeImage(state.interaction, target.opaque_target_id);
      state.hideSelectedMask = state.hiddenTargetIds.has(target.opaque_target_id);
      state.selectedPalette = target.active_palette;
      resetPreviewFrame();
      ensurePaletteSelection(true);
      resetAnimation();
      render();
      if (dyeTargetPreviewAvailable(target)) void renderFrame();
      else updateCanvasStatus("目标缺失，保留上一张有效预览");
      return;
    }
    if (scopeButton) {
      if (scopeButton.disabled) return;
      state.previewScope = scopeButton.dataset.dyePreviewScope as PreviewScope;
      resetAnimation();
      render();
      void renderFrame();
      return;
    }
    if (editorButton) {
      state.editorMode = editorButton.dataset.dyeEditorMode as typeof state.editorMode;
      render();
      return;
    }
    if (swatchButton) {
      const palette = activePalette(selectedTarget());
      if (!palette?.colors.length) return;
      state.logic = selectPaletteIndex(state.logic, Number(swatchButton.dataset.dyeSwatch), {
        ctrlKey: false,
        shiftKey: false,
        maxIndex: palette.colors.length - 1,
      });
      render();
      return;
    }
    if (element.closest("[data-dye-repair-target]")) {
      void repairTarget();
      return;
    }
    if (element.closest("[data-dye-reset-adjustment]")) {
      const previous = state.interaction;
      state.interaction = setDyeAdjustmentForSelection(
        state.interaction,
        dyeAdjustmentTargets(),
        defaultDyeAdjustment(),
      );
      void commitInteractionState(previous);
      return;
    }
    const clearExactButton = element.closest<HTMLButtonElement>("[data-dye-clear-exact]");
    if (clearExactButton) {
      const target = selectedTarget();
      const colorIndex = Number(clearExactButton.dataset.dyeColorIndex);
      if (!target || !Number.isInteger(colorIndex)) return;
      const previous = state.interaction;
      state.interaction = clearExactDyeOverride(
        state.interaction,
        canonicalDyeTargetId(target),
        colorIndex,
      );
      void commitInteractionState(previous);
      return;
    }
  }

  function handleKeyDown(event: KeyboardEvent) {
    if (!state.open) return;
    if (event.key === "Escape" && state.presetDialog) {
      event.preventDefault();
      closeDyePresetDialog();
      return;
    }
    if (isTypingTarget(event.target)) return;
    if (event.ctrlKey && event.key.toLowerCase() === "z") {
      event.preventDefault();
      if (event.shiftKey) void redo();
      else void undo();
    } else if (event.ctrlKey && event.key.toLowerCase() === "y") {
      event.preventDefault();
      void redo();
    } else if (event.key === "Escape") {
      if (state.dialog) requestDialogClose();
      else requestLeave();
    }
  }

  function warnBeforeUnload(event: BeforeUnloadEvent) {
    if (!state.view || !isDirty()) return;
    event.preventDefault();
    event.returnValue = "";
  }

  function requestWindowClose() {
    const decision = dyeWindowCloseDecision({
      hasView: Boolean(state.view),
      dirty: isDirty(),
      loading: state.loading,
      saving: state.saving,
    });
    if (decision === "allow") return false;
    if (decision === "busy") {
      host.onStatus("染色操作仍在进行，完成后请再次关闭程序");
      return true;
    }
    state.windowClosePending = true;
    state.pendingOpen = null;
    state.retained = false;
    state.open = true;
    state.dialog = "leave";
    host.onStatus("关闭程序前请处理未保存的染色草稿");
    render();
    return true;
  }

  function requestLeave() {
    if (state.loading || state.saving) {
      host.onStatus("请等待当前染色操作完成");
      return;
    }
    if (isDirty()) openDialog("leave");
    else void closeSession(true);
  }

  function openDialog(mode: Exclude<DyeDialogMode, null>) {
    if (mode === "wardrobe" && state.dialog !== "wardrobe") {
      state.dialogError = "";
      state.wardrobeDialogDraft = {
        name: `染色装扮 ${new Date().toLocaleDateString()}`,
        groupId: "",
        note: "",
        favorite: false,
      };
    } else if (mode !== "wardrobe" && state.dialog === "wardrobe") {
      state.dialogError = "";
      state.wardrobeDialogDraft = null;
    }
    state.dialog = mode;
    renderDialog();
  }

  function closeDialog() {
    if (state.dialog === "wardrobe") {
      state.dialogError = "";
      state.wardrobeDialogDraft = null;
    }
    state.dialog = null;
    const backdrop = query<HTMLElement>("#avatarDyeDialog");
    if (backdrop) backdrop.hidden = true;
  }

  function requestDialogClose() {
    if (state.dialog === "replace_draft") {
      cancelPendingOpen();
      return;
    }
    if (!state.windowClosePending) {
      state.leaveAfterSave = false;
      closeDialog();
      return;
    }
    if (state.dialog !== "leave") {
      openDialog("leave");
      return;
    }
    state.windowClosePending = false;
    state.leaveAfterSave = false;
    closeDialog();
    host.onStatus("已取消关闭，染色草稿仍保留在内存中");
  }

  function renderDialog() {
    const backdrop = query<HTMLElement>("#avatarDyeDialog");
    const body = query<HTMLElement>("#avatarDyeDialogBody");
    const actions = query<HTMLElement>("#avatarDyeDialogActions");
    if (!backdrop || !body || !actions) return;
    backdrop.hidden = !state.dialog;
    if (!state.dialog) return;
    const metrics = dyeMetrics();
    const validation = dialogValidationHtml();
    if (state.dialog === "wardrobe") {
      setText("#avatarDyeDialogTitle", "保存染色装扮到时装橱窗");
      const groups = host.getWardrobeGroups();
      const draft = state.wardrobeDialogDraft ?? {
        name: `染色装扮 ${new Date().toLocaleDateString()}`,
        groupId: "",
        note: "",
        favorite: false,
      };
      body.innerHTML = `
        <p>保存九部位快照、主 IMG 与 Mask 的加密染色配方和染色预览；不会修改游戏 NPK。</p>
        ${state.dialogError ? `<div class="avatar-dye-dialog-error" role="alert" aria-live="polite"><strong>保存失败</strong><span>${escapeHtml(state.dialogError)}</span><em>填写内容已保留。</em></div>` : ""}
        <label>装扮名称<input id="avatarDyeWardrobeName" maxlength="80" value="${escapeHtml(draft.name)}" /></label>
        <label>栏目<select id="avatarDyeWardrobeGroup"><option value=""${draft.groupId ? "" : " selected"}>未分栏</option>${groups.map((group) => `<option value="${escapeHtml(group.id)}"${group.id === draft.groupId ? " selected" : ""}>${escapeHtml(group.name)}</option>`).join("")}</select></label>
        <label>备注<textarea id="avatarDyeWardrobeNote" maxlength="500" rows="3" placeholder="记录这套染色的用途">${escapeHtml(draft.note)}</textarea></label>
        <label class="avatar-dye-dialog-check"><input id="avatarDyeWardrobeFavorite" type="checkbox"${draft.favorite ? " checked" : ""} /> 收藏此染色装扮</label>
        ${dialogMetricsHtml(metrics)}
        ${validation}
      `;
      actions.innerHTML = cancelActionHtml() + `<button class="avatar-dye-button primary" data-dye-dialog-action="save-wardrobe">加密保存</button>`;
    } else if (state.dialog === "workspace") {
      setText("#avatarDyeDialogTitle", "写入当前 / 目标工作区");
      body.innerHTML = `<p>当前穿戴只记录你声明的游戏状态，不会自动安装染色补丁。目标外观会被一键换装读取并烘焙。</p>${dialogMetricsHtml(metrics)}${validation}`;
      actions.innerHTML = cancelActionHtml()
        + `<button class="avatar-dye-button" data-dye-dialog-action="save-current">保存为当前穿戴</button>`
        + `<button class="avatar-dye-button primary" data-dye-dialog-action="save-target">保存为目标外观</button>`;
    } else if (state.dialog === "swap") {
      setText("#avatarDyeDialogTitle", "设为目标并前往一键换装？");
      body.innerHTML = `<p>主 IMG 与 Mask 的最终染色都会烘焙到一键换装补丁，不依赖额外染色补丁。</p>${dialogMetricsHtml(metrics)}${validation}`;
      actions.innerHTML = cancelActionHtml() + `<button class="avatar-dye-button primary" data-dye-dialog-action="save-swap">设为目标并前往</button>`;
    } else if (state.dialog === "export") {
      setText("#avatarDyeDialogTitle", "导出独立染色补丁");
      body.innerHTML = `<p>仅写入新建补丁文件；原始 NPK/IMG、帧数据、LINK、坐标、压缩数据和 Alpha 均不会被原地修改。</p>${dialogMetricsHtml(metrics)}${validation}`;
      actions.innerHTML = cancelActionHtml()
        + `<button class="avatar-dye-button primary" data-dye-dialog-action="export-folder">选择输出目录</button>`;
    } else if (state.dialog === "replace_draft") {
      setText("#avatarDyeDialogTitle", "另一个染色草稿尚未处理");
      body.innerHTML = `<p>当前内存草稿与准备打开的搭配不同。请选择继续旧草稿、明确放弃旧草稿并打开新搭配，或取消本次打开。</p>${dialogMetricsHtml(metrics)}`;
      actions.innerHTML = `<button class="avatar-dye-button" data-dye-dialog-action="replace-cancel">取消</button>`
        + `<button class="avatar-dye-button" data-dye-dialog-action="replace-continue">继续旧草稿</button>`
        + `<button class="avatar-dye-button danger" data-dye-dialog-action="replace-discard">放弃旧草稿并打开新搭配</button>`;
    } else {
      setText("#avatarDyeDialogTitle", state.windowClosePending ? "关闭程序前处理染色草稿" : "染色草稿尚未保存");
      body.innerHTML = `<p>${state.windowClosePending
        ? "关闭程序前请选择保存到橱窗、放弃草稿或取消关闭；本次会话不会自动写入磁盘。"
        : "可以保存到时装橱窗、保留本次运行草稿、直接放弃，或返回继续编辑。"}</p>${dialogMetricsHtml(metrics)}`;
      actions.innerHTML = `<button class="avatar-dye-button" data-dye-dialog-action="cancel">${state.windowClosePending ? "取消关闭" : "继续编辑"}</button>`
        + (state.windowClosePending ? "" : `<button class="avatar-dye-button" data-dye-dialog-action="retain">保留本次运行草稿</button>`)
        + `<button class="avatar-dye-button" data-dye-dialog-action="leave-save">保存到橱窗</button>`
        + `<button class="avatar-dye-button danger" data-dye-dialog-action="discard">放弃草稿</button>`;
    }
    const busy = state.loading || state.saving;
    actions.querySelectorAll<HTMLButtonElement>("[data-dye-dialog-action]").forEach((button) => {
      const action = button.dataset.dyeDialogAction;
      button.disabled = busy || (resourceBlocked() && dyeDialogActionBlockedByResource(action));
    });
    const closeButton = query<HTMLButtonElement>("#avatarDyeDialogClose");
    if (closeButton) closeButton.disabled = busy;
  }

  function handleDialogClick(event: Event) {
    const backdrop = query<HTMLElement>("#avatarDyeDialog");
    if (event.target === backdrop && state.dialog !== "leave") {
      requestDialogClose();
      return;
    }
    const element = event.target instanceof Element ? event.target : null;
    const button = element?.closest<HTMLButtonElement>("[data-dye-dialog-action]");
    if (!button || button.disabled || state.loading || state.saving) return;
    const action = button.dataset.dyeDialogAction;
    if (action === "cancel") requestDialogClose();
    if (action === "save-wardrobe") void saveToWardrobe();
    if (action === "save-current") void saveAsActive("current");
    if (action === "save-target") void saveAsActive("target");
    if (action === "save-swap") void saveAsTargetAndOpenSwap();
    if (action === "export-folder") void exportWithChosenDirectory();
    if (action === "discard") void closeSession(!state.windowClosePending, state.windowClosePending);
    if (action === "retain") retainSessionDraft();
    if (action === "replace-cancel") cancelPendingOpen();
    if (action === "replace-continue") continuePendingDraft();
    if (action === "replace-discard") void discardDraftAndOpenPending();
    if (action === "leave-save") {
      state.leaveAfterSave = true;
      openDialog("wardrobe");
    }
  }

  function cancelPendingOpen() {
    const pending = state.pendingOpen;
    state.pendingOpen = null;
    state.open = false;
    state.retained = Boolean(state.view);
    state.dialog = null;
    render();
    if (pending) host.onReturn(pending.source);
    host.onStatus("已取消打开新搭配，旧染色草稿仍保留在本次运行中");
  }

  function continuePendingDraft() {
    const pending = state.pendingOpen;
    if (!pending || !state.view) return;
    state.pendingOpen = null;
    state.retained = true;
    resumeRetainedDraft(pending.source, pending.preferredLayer);
  }

  async function discardDraftAndOpenPending() {
    const pending = state.pendingOpen;
    if (!pending) return;
    state.pendingOpen = null;
    await closeSession(false);
    await pending.open();
  }

  async function refreshPersistedSessionView(sessionId: string) {
    if (!isCurrentSession(sessionId)) return;
    markPersisted();
    state.error = "";
    if (state.view) {
      state.view = {
        ...state.view,
        current: null,
        target: null,
      };
      render();
    }
    try {
      const view = await host.commands.getView({ sessionId });
      if (!isCurrentSession(sessionId)) return;
      syncView(view, "persist");
    } catch {
      // onWardrobeState already refreshed the safe current/target fallbacks.
    }
  }

  async function refreshFailedPersistenceSessionView(sessionId: string) {
    try {
      const view = await host.commands.getView({ sessionId });
      if (!isCurrentSession(sessionId)) return false;
      syncView(view);
      return true;
    } catch {
      return false;
    }
  }

  async function handlePersistenceFailure(sessionId: string, action: string, error?: unknown) {
    if (!isCurrentSession(sessionId)) return;
    await refreshFailedPersistenceSessionView(sessionId);
    if (!isCurrentSession(sessionId)) return;
    const wardrobeDialogActive = state.dialog === "wardrobe";
    if (wardrobeDialogActive) {
      state.dialogError = dyePersistenceErrorMessage(action, error);
      state.error = "";
    }
    if (resourceRepairable()) {
      resetAnimation();
      state.error = "";
      const guidance = dyeLiveResourceChangeGuidance();
      if (wardrobeDialogActive) state.dialogError = guidance;
      host.onStatus(guidance);
      return;
    }
    if (wardrobeDialogActive) {
      host.onStatus(state.dialogError);
      return;
    }
    state.error = dyePersistenceErrorMessage(action, error);
    host.onStatus(state.error);
  }

  async function saveToWardrobe() {
    const sessionId = state.view?.session_id;
    const name = query<HTMLInputElement>("#avatarDyeWardrobeName")?.value.trim() ?? "";
    if (state.loading || state.saving || !sessionId || resourceBlocked()) return;
    const groupId = query<HTMLSelectElement>("#avatarDyeWardrobeGroup")?.value ?? "";
    const note = query<HTMLTextAreaElement>("#avatarDyeWardrobeNote")?.value.trim() ?? "";
    const favorite = query<HTMLInputElement>("#avatarDyeWardrobeFavorite")?.checked ?? false;
    state.wardrobeDialogDraft = {
      name,
      groupId,
      note,
      favorite,
    };
    if (!name) {
      state.dialogError = "请填写装扮名称";
      renderDialog();
      return;
    }
    const draft = {
      name,
      group_id: groupId || null,
      note,
      favorite,
    };
    state.dialogError = "";
    state.saving = true;
    render();
    try {
      const wardrobe = await host.commands.saveToWardrobe({ sessionId, draft, allowDuplicate: false });
      if (!isCurrentSession(sessionId)) return;
      host.onWardrobeState(wardrobe);
      await refreshPersistedSessionView(sessionId);
      if (!isCurrentSession(sessionId)) return;
      host.onStatus(`已加密保存染色装扮：${name}`);
      closeDialog();
      if (state.leaveAfterSave) {
        await closeSession(!state.windowClosePending, state.windowClosePending);
      }
    } catch (error) {
      await handlePersistenceFailure(sessionId, "保存染色装扮", error);
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
      }
    }
  }

  async function saveAsActive(role: "current" | "target") {
    const sessionId = state.view?.session_id;
    if (!sessionId || state.loading || state.saving || resourceBlocked()) return;
    state.saving = true;
    render();
    try {
      const workspace = host.getWorkspace();
      const other = role === "current" ? workspace.target : workspace.current;
      const result = await persistDyeWorkspaceWithCompatibilityConfirmation({
        role,
        shortcut: false,
        incompatible: dyeWorkspaceIdentitiesIncompatible(state.sourceCompatibility, other),
        persist: (clearIncompatible) => host.commands.saveAsActive({
          sessionId,
          role,
          clearIncompatible,
        }),
        confirm: (message) => window.confirm(message),
      });
      if (result.status === "cancelled") {
        host.onStatus("已取消写入染色工作区；已有当前穿戴和目标外观均保持不变");
        return;
      }
      const wardrobe = result.value;
      if (!isCurrentSession(sessionId)) return;
      host.onWardrobeState(wardrobe);
      await refreshPersistedSessionView(sessionId);
      if (!isCurrentSession(sessionId)) return;
      host.onStatus(role === "current"
        ? "染色搭配已保存为当前穿戴；该操作不会自动安装游戏补丁"
        : "染色搭配已保存为目标外观");
      closeDialog();
    } catch (error) {
      await handlePersistenceFailure(sessionId, "写入染色工作区", error);
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
      }
    }
  }

  async function saveAsTargetAndOpenSwap() {
    const sessionId = state.view?.session_id;
    if (!sessionId || state.loading || state.saving || resourceBlocked()) return;
    state.saving = true;
    render();
    try {
      const result = await persistDyeWorkspaceWithCompatibilityConfirmation({
        role: "target",
        shortcut: true,
        incompatible: dyeWorkspaceIdentitiesIncompatible(state.sourceCompatibility, host.getWorkspace().current),
        persist: (clearIncompatible) => host.commands.saveAsTargetAndOpenSwap({ sessionId, clearIncompatible }),
        confirm: (message) => window.confirm(message),
      });
      if (result.status === "cancelled") {
        host.onStatus("已取消前往一键换装；已有当前穿戴和目标外观均保持不变");
        return;
      }
      const wardrobe = result.value;
      if (!isCurrentSession(sessionId)) return;
      host.onWardrobeState(wardrobe);
      await refreshPersistedSessionView(sessionId);
      if (!isCurrentSession(sessionId)) return;
      closeDialog();
      await closeSession(false);
      await host.onEnterSwap();
    } catch (error) {
      await handlePersistenceFailure(sessionId, "保存目标染色外观", error);
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
      }
    }
  }

  async function exportWithChosenDirectory() {
    const directory = await host.chooseExportDirectory();
    if (directory) await exportNpk(directory);
  }

  async function exportNpk(outputDirectory: string) {
    const sessionId = state.view?.session_id;
    if (!sessionId || state.loading || state.saving || resourceBlocked()) return;
    state.saving = true;
    render();
    try {
      const result = await host.commands.exportNpk({ sessionId, outputDirectory });
      if (!isCurrentSession(sessionId)) return;
      host.onStatus(`染色补丁已导出：${result.entry_count} 个 IMG · ${result.output_path}`);
      closeDialog();
    } catch (error) {
      await handlePersistenceFailure(sessionId, "导出染色补丁", error);
    } finally {
      if (isCurrentSession(sessionId)) {
        state.saving = false;
        render();
      }
    }
  }

  function markPersisted() {
    state.logic = markDyePersisted(state.logic);
    if (state.view) state.view = { ...state.view, is_dirty: false, has_dye: state.logic.hasDye };
  }

  function retainSessionDraft() {
    if (!state.view || state.windowClosePending) return;
    const source = state.source;
    resetAnimation();
    state.open = false;
    state.retained = true;
    state.dialog = null;
    state.leaveAfterSave = false;
    render();
    host.onReturn(source);
    host.onStatus("染色草稿已保留在本次运行内存中；再次进入染色即可继续");
  }

  async function closeSession(returnToSource: boolean, closeWindow = false) {
    const sessionId = state.view?.session_id;
    const source = state.source;
    stopPlayback();
    try {
      if (sessionId) await host.commands.close({ sessionId });
    } catch {
      host.onStatus("染色会话关闭失败；内存草稿已从界面移除");
    }
    state.open = false;
    state.retained = false;
    state.view = null;
    state.sourceIdentity = "";
    state.pendingOpen = null;
    state.logic = createAvatarDyeState();
    state.interaction = createDyeInteractionState();
    state.gestureBefore = null;
    state.selectedTargetId = "";
    state.presetDialog = null;
    state.presetDialogDraft = null;
    state.presetDialogError = "";
    state.hiddenTargetIds.clear();
    state.dialog = null;
    state.leaveAfterSave = false;
    state.windowClosePending = false;
    state.loading = false;
    state.rendering = false;
    state.saving = false;
    resetAnimation();
    render();
    if (returnToSource) host.onReturn(source);
    if (closeWindow) await host.closeWindow();
  }

  function selectedTarget() {
    return state.view?.targets.find((target) => target.opaque_target_id === state.selectedTargetId) ?? null;
  }

  function previewAnchorTarget() {
    const selected = selectedDyeTargets(state.interaction, dyeAdjustmentTargets()) as DyeTargetView[];
    return selected.find(dyeTargetPreviewAvailable)
      ?? (dyeTargetPreviewAvailable(selectedTarget() ?? {}) ? selectedTarget() : null)
      ?? state.view?.targets.find(dyeTargetPreviewAvailable)
      ?? null;
  }

  function previewRequestScope(): PreviewScope {
    return state.interaction.objectMode === "part" && state.previewScope === "part"
      ? "outfit"
      : state.previewScope;
  }

  function previewHiddenTargetIds() {
    const hidden = new Set(state.hiddenTargetIds);
    if (state.interaction.objectMode === "part" && state.previewScope === "part") {
      const selectedParts = new Set(state.interaction.selectedParts);
      for (const target of state.view?.targets ?? []) {
        const slot = (target.slot || target.slot_label || "").trim().toLowerCase();
        if (!selectedParts.has(slot)) hidden.add(target.opaque_target_id);
      }
    }
    return [...hidden];
  }

  function updatePreviewScopeAvailability() {
    const objectMode = state.interaction.objectMode;
    state.previewScope = normalizedDyePreviewScope(
      objectMode,
      state.previewScope,
      state.interaction.selectedParts.length,
    );
    const previewAvailable = Boolean(previewAnchorTarget());
    document.querySelectorAll<HTMLButtonElement>("[data-dye-preview-scope]").forEach((button) => {
      const scope = button.dataset.dyePreviewScope as PreviewScope;
      button.classList.toggle("active", scope === state.previewScope);
      button.disabled = !previewAvailable
        || (objectMode === "all" && scope !== "outfit")
        || (objectMode === "part" && scope === "target")
        || (objectMode === "part" && scope === "part" && !state.interaction.selectedParts.length);
    });
  }

  function isCurrentSession(sessionId: string) {
    return state.open && state.view?.session_id === sessionId;
  }

  function selectedPalette() {
    return selectedTarget()?.palettes.find((palette) => palette.palette_index === state.selectedPalette) ?? null;
  }

  function activePalette(target: DyeTargetView | null) {
    return target?.palettes.find((palette) => palette.palette_index === target.active_palette) ?? null;
  }

  function ensurePaletteSelection(force = false) {
    const palette = selectedPalette();
    if (!palette?.colors.length) return;
    const invalid = state.logic.selectedPaletteIndices.length === 0
      || state.logic.selectedPaletteIndices.some((index) => index >= palette.colors.length);
    if (force || invalid) {
      state.logic = selectPaletteIndex(state.logic, initialColorIndex(palette), {
        maxIndex: palette.colors.length - 1,
      });
    }
  }

  function initialColorIndex(palette: DyePaletteView) {
    return palette.colors.find((color) => color.used_pixel_count > 0 && color.current_rgba[3] > 0)?.color_index
      ?? palette.colors.find((color) => color.current_rgba[3] > 0)?.color_index
      ?? palette.colors[0]?.color_index
      ?? 0;
  }

  function modifiedTargets() {
    return (state.view?.targets ?? []).filter((target) => changedColorCount(target) > 0);
  }

  function changedColorCount(target: DyeTargetView) {
    return activePalette(target)?.colors.filter((color) => !sameRgb(color.current_rgba, color.original_rgba)).length ?? 0;
  }

  function hasDye() {
    return Boolean(state.view?.has_dye || state.logic.hasDye);
  }

  function isDirty() {
    return Boolean(state.view?.is_dirty || state.logic.isDirty);
  }

  function resourceBlocked() {
    return dyeSessionBlockingPolicy(state.view ?? {}).persistenceBlocked;
  }

  function resourceEditingBlocked() {
    return dyeSessionBlockingPolicy(state.view ?? {}).editingBlocked;
  }

  function resourceRepairable() {
    return dyeResourcePolicy(state.view?.resource_status ?? "").repairable;
  }

  function hasRepairableTargetIssue() {
    return Boolean(state.view?.targets.some((target) => targetResourceStatus(target) === "changed"));
  }

  function currentResourceChangeGuidance() {
    return hasRepairableTargetIssue()
      ? dyePersistedResourceChangeGuidance()
      : dyeLiveResourceChangeGuidance();
  }

  function currentConflictUndoAvailable() {
    const status = (state.view?.resource_status ?? "").trim().toLowerCase();
    return status === "conflict" && Boolean(state.view?.can_undo);
  }

  function isEditable(target: DyeTargetView) {
    return isTargetEditable(target);
  }

  function capabilityLabel(target: DyeTargetView) {
    const resourceStatus = targetResourceStatus(target);
    if (resourceStatus === "changed") return "已变化";
    if (resourceStatus === "missing") return "缺失";
    if (resourceStatus === "conflict") return "冲突";
    if (resourceStatus === "invalid") return "解析失败";
    const value = target.capability.trim().toLowerCase().replaceAll("_", "");
    if (value.includes("partial")) return "部分可染";
    if (value.includes("readonly") || !target.palettes.length) return "只读";
    if (value.includes("missing")) return "缺失";
    if (value.includes("conflict")) return "冲突";
    return "可染";
  }

  function capabilityClass(target: DyeTargetView) {
    const label = capabilityLabel(target);
    if (label === "可染") return changedColorCount(target) ? "changed" : "good";
    if (label === "部分可染" || label === "已变化") return "warn";
    return "error";
  }

  function targetResourceStatus(target: DyeTargetView) {
    const status = (target.resource_status ?? "valid").trim().toLowerCase().replace(/[\s-]+/g, "_");
    return ["changed", "missing", "conflict", "invalid"].includes(status) ? status : "valid";
  }

  function maskNumber(target: DyeTargetView) {
    return dyeTargetMaskNumber(target);
  }

  function isMask(target: DyeTargetView | null) {
    return Boolean(target && maskNumber(target) !== null);
  }

  function targetKindLabel(target: DyeTargetView) {
    const number = maskNumber(target);
    return number === null ? "主" : `M${number}`;
  }

  function targetKindLongLabel(target: DyeTargetView) {
    const number = maskNumber(target);
    return number === null ? "主 IMG" : `Mask ${number}`;
  }

  function targetSlotLabel(target: DyeTargetView) {
    return target.slot_label || target.slot || "";
  }

  function previewHidesSelectedMask(target: DyeTargetView | null) {
    return dyePreviewHidesSelectedMask(state.previewScope, isMask(target), state.hideSelectedMask);
  }

  function scopedPreviewFrameCount() {
    const target = previewAnchorTarget();
    return dyePreviewFrameCount(
      state.view?.targets ?? [],
      target?.opaque_target_id ?? "",
      previewRequestScope(),
      previewHidesSelectedMask(target),
      previewHiddenTargetIds(),
    );
  }

  function previewFrameCount() {
    return state.animationFrameCount || scopedPreviewFrameCount();
  }

  function dyeMetrics() {
    const targets = modifiedTargets();
    return {
      slots: new Set(targets.map((target) => targetSlotLabel(target) || "未分类")).size,
      targets: targets.length,
      bases: targets.filter((target) => !isMask(target)).length,
      masks: targets.filter(isMask).length,
      colors: targets.reduce((sum, target) => sum + changedColorCount(target), 0),
    };
  }

  function draftStatusText() {
    if (state.loading) return "正在加载染色会话…";
    if (resourceRepairable()) return currentResourceChangeGuidance();
    if (resourceBlocked() && !resourceEditingBlocked()) {
      const resourceLabel = dyeIncompleteResourceLabel(
        state.view?.targets.map((target) => targetResourceStatus(target)) ?? [],
      );
      return `${resourceLabel} · 可继续编辑有效目标，保存和导出已冻结`;
    }
    if (resourceBlocked()) return "源资源状态异常 · 会话冻结";
    if (state.error) return state.error;
    const metrics = dyeMetrics();
    const revision = state.view?.recipe_revision?.slice(0, 8);
    if (!hasDye()) return "尚未染色 · 原始资源只读";
    return `${isDirty() ? "草稿已修改" : "染色已保存"} · ${metrics.targets} IMG / ${metrics.colors} 色 · Mask ${metrics.masks}${revision ? ` · r${revision}` : ""}`;
  }

  function previewStatusText(missingCount = 0) {
    const hidden = state.previewScope === "target" ? 0 : state.hiddenTargetIds.size;
    return `左侧原图 / 右侧改色后 · ${state.previewScope === "outfit" ? "整套合成" : state.previewScope === "part" ? "当前部位" : "当前 IMG"}${hidden ? ` · 隐藏 ${hidden} 个 IMG` : ""}${missingCount ? ` · ${missingCount} 个资源未参与预览` : ""}`;
  }

  function actionFailure(action: string) {
    const target = selectedTarget();
    return `${action}失败${target ? `（当前 ${targetKindLongLabel(target)}）` : ""}，请刷新资源状态后重试`;
  }

  function updateCanvasStatus(message: string) {
    setText("#avatarDyeCanvasStatus", message);
    query("#avatarDyeCanvasStatus")?.classList.toggle("error", message.includes("失败"));
  }

  function rangeHtml(id: string, label: string, min: number, max: number, value: number, suffix: string) {
    return `<label class="avatar-dye-range"><span>${label}</span><input id="${id}" type="range" min="${min}" max="${max}" value="${value}" /><output for="${id}" data-suffix="${suffix}">${value}${suffix}</output></label>`;
  }

  function dialogMetricsHtml(metrics: ReturnType<typeof dyeMetrics>) {
    return `<div class="avatar-dye-dialog-metrics"><div><span>染色部位</span><strong>${metrics.slots}</strong></div><div><span>主 IMG / Mask</span><strong>${metrics.bases} / ${metrics.masks}</strong></div><div><span>修改颜色</span><strong>${metrics.colors}</strong></div></div>`;
  }

  function dialogValidationHtml() {
    const status = state.view?.resource_status ?? "unchecked";
    const validation = dyeValidationPresentation(status, hasRepairableTargetIssue());
    return `<div class="avatar-dye-dialog-validation ${validation.tone}"><span>资源校验</span><strong>${validation.title}</strong><em>${validation.detail}</em></div>`;
  }

  function cancelActionHtml() {
    return `<button class="avatar-dye-button" data-dye-dialog-action="cancel">取消</button>`;
  }

  function sameRgb(left: readonly number[], right: readonly number[]) {
    return left[0] === right[0] && left[1] === right[1] && left[2] === right[2];
  }

  function rgbaCss(rgba: readonly number[]) {
    return `rgba(${rgba[0]},${rgba[1]},${rgba[2]},${(rgba[3] ?? 255) / 255})`;
  }

  function rgbHex(rgb: readonly number[]) {
    return `#${rgb.slice(0, 3).map((value) => Math.max(0, Math.min(255, value)).toString(16).padStart(2, "0")).join("")}`.toUpperCase();
  }

  function parseRgbHex(value: string): [number, number, number] | null {
    const match = value.trim().match(/^#([0-9a-f]{6})$/i);
    if (!match) return null;
    return [0, 2, 4].map((offset) => Number.parseInt(match[1].slice(offset, offset + 2), 16)) as [number, number, number];
  }

  function formatPixels(value: number) {
    return value >= 10_000 ? `${(value / 10_000).toFixed(1)}万` : value.toLocaleString();
  }

  function formatPercent(value: number) {
    return `${Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1)}%`;
  }

  function formatIndexSelection(indices: number[]) {
    const sorted = [...indices].sort((left, right) => left - right);
    if (!sorted.length) return "未选择";
    const ranges: string[] = [];
    let start = sorted[0];
    let end = start;
    for (const value of sorted.slice(1)) {
      if (value === end + 1) {
        end = value;
      } else {
        ranges.push(start === end ? String(start) : `${start}–${end}`);
        start = value;
        end = value;
      }
    }
    ranges.push(start === end ? String(start) : `${start}–${end}`);
    return ranges.join("、");
  }

  function numberInput(selector: string) {
    return Number(query<HTMLInputElement>(selector)?.value ?? 0);
  }

  function isTypingTarget(target: EventTarget | null) {
    return target instanceof Element && Boolean(target.closest("input, textarea, select, [contenteditable='true']"));
  }

  function toImageData(image: PreviewImage) {
    const bytes = image.rgba_base64
      ? Uint8ClampedArray.from(atob(image.rgba_base64), (char) => char.charCodeAt(0))
      : Uint8ClampedArray.from(image.rgba ?? []);
    return new ImageData(bytes, image.width, image.height);
  }

  function query<T extends Element = HTMLElement>(selector: string) {
    return document.querySelector<T>(selector);
  }

  function setText(selector: string, value: string) {
    const element = query(selector);
    if (element) element.textContent = value;
  }

  function escapeHtml(value: string) {
    return value.replace(/[&<>"']/g, (character) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    })[character] ?? character);
  }

  return {
      wire,
      render,
      isOpen: () => state.open,
      requestWindowClose,
      openFromStudio,
    openFromWardrobe,
  };
}

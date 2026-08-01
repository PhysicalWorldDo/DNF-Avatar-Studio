import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Box,
  BookOpen,
  createElement,
  Download,
  FolderOpen,
  Layers3,
  PackageOpen,
  Palette,
  Pause,
  Play,
  RefreshCw,
  Search,
  Save as SaveIcon,
  Shirt,
  Trash2,
  WandSparkles,
  X,
  type IconNode,
} from "lucide";
import {
  centeredPreviewPlacement,
  opaquePixelBounds,
  type OpaqueBounds,
} from "./wardrobe-preview.mjs";
import {
  previewFrameIndexForProfession,
  previewStartFrameForProfession,
} from "./preview-frame.mjs";
import {
  createHiddenFeatureUnlockController,
  hasInvalidHiddenFeatureCredential,
  shouldSubmitHiddenFeatureUnlock,
  verifiedFeatureVisibility,
  type HiddenFeatureUnlockController,
} from "./hidden-feature-unlock.mjs";
import {
  createCatalogPreviewGate,
  createCatalogSelectionGate,
  createLatestCatalogQuery,
  projectSetMemberRecords,
} from "./avatar-catalog-query.mjs";
import {
  batchActionLabel,
  createGalleryMemorySnapshot,
  createGalleryRequestGate,
  createGalleryThumbnailScheduler,
  disposeGalleryMemorySnapshot,
  galleryPageWindow,
  gallerySingleGate,
  isGalleryThumbnailCancellation,
  mergeGalleryCards,
  SET_GALLERY_PAGE_SIZE,
  type GalleryCard,
  type GalleryMemorySnapshot,
  type GalleryWireCard,
  warmGallerySnapshotCards,
} from "./catalog-gallery.mjs";
import { createOutfitSwapPreviewGate } from "./outfit-swap-preview.mjs";
import {
  resolveAvatarLayerVisibility,
  visibleAvatarLayers,
} from "./avatar-visibility.mjs";
import {
  createAvatarDyePageController,
  dyeWorkspaceStatusPresentation,
  type AvatarDyePageHost,
  type AvatarDyePageController,
  type AvatarDyeSessionView,
  type DyeComparisonAnimationResponse,
  type DyeComparisonFrameResponse,
  type DyeBatchMutation,
  type DyeColorOperation,
  type DyeSnapshotSummary,
} from "./avatar-dye-page";
import type { DyeColorPresetCatalog } from "./avatar-dye-presets.mjs";
import brandIconUrl from "./assets/app-icon.png";
import "./styles.css";

type AvatarLayer = {
  item_id: string;
  item_name: string;
  slot: string;
  visual_family_key: string;
  layer_order: number;
  equipment_type_enum: number;
  hide_equipment_type_enums: number[];
  enable_dye_mask1: boolean;
  enable_dye_mask2: boolean;
  layer_sequence: number;
  layer_tag: string;
  variation: string;
  palette_index: number;
  visual_img: string;
  status: string;
};

type AvatarRole = {
  playable_role_key: string;
  visual_family_key: string;
};

type AvatarVisualFamily = {
  visual_family_key: string;
  resource_owner_key: string;
  resource_generation: "base" | "at";
  default_skin_img: string;
};

type AvatarItem = {
  item_id: string;
  item_name: string;
  slot: string;
  profession_dir: string;
  playable_roles: string[];
  path_family: string;
  resource_kind: "avatar" | "weapon_avatar" | "weapon";
  icon_img: string;
  layer_count: number;
  layer_counts_by_visual_family: Record<string, number>;
  hide_equipment_types: string[];
  hide_equipment_type_enums: number[];
  enable_dye_mask1: boolean;
  enable_dye_mask2: boolean;
  layers: AvatarLayer[];
};

type AvatarSet = {
  set_id: string;
  set_name: string;
  source: string;
  playable_roles: string[];
  member_ids: string[];
  member_names: string[];
};

type CatalogMetadataResponse = {
  stats: {
    item_count: number;
    set_count: number;
    layer_count: number;
  };
  roles: AvatarRole[];
  visual_families: AvatarVisualFamily[];
  item_result_limit: number;
  set_result_limit: number;
  schema_version: number;
  build_id: string;
};

type AvatarSetDetail = {
  set: AvatarSet;
  items: AvatarItem[];
};

type PreviewImage = {
  width: number;
  height: number;
  rgba_base64: string;
  missing_imgs: string[];
};

type PreviewAnimation = {
  frame_count: number;
  missing_imgs: string[];
  frames: PreviewImage[];
};


type PreviewLayerRequest = {
  img_path: string;
  frame_index: number;
  palette_index: number;
  enable_dye_mask1: boolean;
  enable_dye_mask2: boolean;
};

type ExportNpkResult = {
  output_path: string;
  entry_count: number;
};

type ExportWebpResult = {
  output_path: string;
  width: number;
  height: number;
  frame_count: number;
};

type SavedOutfitItem = {
  slot: WardrobeSlotKey;
  item_name: string | null;
  layer_count: number;
};

type SavedWeaponItem = {
  slot: "weapon";
  item_name: string;
  layer_count: number;
};

type WardrobeSnapshotInput = {
  playable_role_key: string;
  visual_family_key: string;
  resource_owner_key: string;
  resource_generation: "base" | "at";
  profession_dir: string;
  path_family: string;
  items: Array<SavedOutfitItem & { item_ref: string | null }>;
  weapon: (SavedWeaponItem & { item_ref: string }) | null;
  preview_data_url: string;
};

type SavedOutfitView = {
  playable_role_key: string;
  visual_family_key: string;
  resource_owner_key: string;
  resource_generation: "base" | "at";
  profession_dir: string;
  path_family: string;
  items: SavedOutfitItem[];
  weapon: SavedWeaponItem | null;
  preview_data_url: string;
  saved_at_ms: number;
  revision: string;
  has_dye: boolean;
  dyed_slot_count: number;
  dyed_base_count: number;
  dyed_mask_count: number;
  dye_status: string;
  recipe_revision: string | null;
  representative_colors: string[];
  resource_status: "unchecked" | "valid" | "changed";
  invalid_slots: WardrobeSlotKey[];
};

type WardrobeGroupView = {
  id: string;
  name: string;
  sort_order: number;
};

type WardrobeEntryView = {
  id: string;
  name: string;
  note: string;
  group_id: string | null;
  favorite: boolean;
  snapshot: SavedOutfitView;
  created_at_ms: number;
  updated_at_ms: number;
  last_used_at_ms: number | null;
  is_current: boolean;
  is_target: boolean;
};

type WardrobeStateView = {
  current: SavedOutfitView | null;
  target: SavedOutfitView | null;
  groups: WardrobeGroupView[];
  entries: WardrobeEntryView[];
};

type WardrobeShareImportResult = {
  wardrobe: WardrobeStateView;
  entry_count: number;
  group_name: string | null;
};

type OutfitSwapPartPlan = {
  slot: string;
  action: string;
  current_item_name: string;
  target_item_name: string;
  carrier_img: string;
  target_layer_count: number;
  output_frame_count: number;
  canvas_width: number;
  canvas_height: number;
  transparent_imgs: string[];
  status: string;
  has_dye?: boolean;
  dyed_base_count?: number;
  dyed_mask_count?: number;
  recipe_revision?: string | null;
};

type OutfitSwapCarrierGroupPlan = {
  carrier_slot: string;
  current_item_name: string;
  carrier_layer_order: number;
  carrier_equipment_type_enum: number;
  carrier_layer_sequence: number;
  target_slots: string[];
  target_item_names: string[];
  target_layer_count: number;
  output_frame_count: number;
  canvas_width: number;
  canvas_height: number;
  exact_order_match_count: number;
  cross_slot_layer_count: number;
};

type OutfitSwapPlan = {
  profession_dir: string;
  path_family: string;
  strategy: string;
  carrier_groups: OutfitSwapCarrierGroupPlan[];
  parts: OutfitSwapPartPlan[];
  warnings: string[];
};

type OutfitSwapPageResult = {
  plan: OutfitSwapPlan;
  current_preview: PreviewImage;
  target_preview: PreviewImage;
  current_preview_available: boolean;
  target_preview_available: boolean;
};

type OutfitSwapProgress = {
  percent: number;
  stage: string;
};

type OutfitSwapExportResult = {
  output_path: string;
  part_count: number;
  entry_count: number;
};

type ProfessionWideCarrierCandidate = {
  slot: string;
  item_name: string;
  layer_order: number;
  equipment_type_enum: number;
  layer_sequence: number;
};

type ProfessionWideSwapPlan = {
  status: string;
  strategy: string;
  profession_dir: string;
  path_family: string;
  avatar_root: string;
  carrier_slot: string;
  carrier_item_name: string;
  carrier_layer_order: number;
  carrier_equipment_type_enum: number;
  carrier_layer_sequence: number;
  source_npk_count: number;
  total_img_count: number;
  suppressed_img_count: number;
  total_frame_count: number;
  estimated_output_bytes: number;
  affected_scope: string;
  confirmation_token: string;
};

type AppConfigView = {
  game_dir: string;
  wardrobe_swap_unlocked: boolean;
  wardrobe_swap_credential_invalid: boolean;
};

type HiddenFeatureUnlockResult = {
  matched: boolean;
  feature: "wardrobe_swap" | null;
  already_unlocked: boolean;
  config: AppConfigView;
};

type Mode = "sets" | "items";
type WorkspaceMode = "current" | "gallery";

type CatalogGalleryPage = {
  cards: GalleryWireCard[];
  total: number;
  next_offset: number | null;
};

type CatalogGallerySnapshotResponse = {
  available: boolean;
  snapshot_id: string;
  cards: GalleryWireCard[];
  catalog_total: number;
  named_entries: number;
  unique_visuals: number;
};

type CatalogThumbnailResponse = {
  entity_recipe_key: string;
  visual_signature: string;
  webp_base64: string;
  cache_hit: boolean;
};

type CatalogThumbnailBatchPreflight = {
  scope: string;
  total_entities: number;
  cache_root: string;
  cache: {
    entity_count: number;
    blob_count: number;
    encrypted_bytes: number;
  };
};

type CatalogThumbnailBatchPhase =
  | "queued"
  | "running"
  | "paused"
  | "finalizing"
  | "completed"
  | "cancelled"
  | "failed";

type CatalogThumbnailBatchStatus = {
  job_id: string;
  scope: string;
  kind: "set" | "item";
  playable_role: string;
  phase: CatalogThumbnailBatchPhase;
  total_entities: number;
  resolved_entities: number;
  unique_visuals: number;
  cache_hits: number;
  generated_visuals: number;
  failed_entities: number;
  last_error: string | null;
  started_at_ms: number;
  finished_at_ms: number | null;
};

const PLAY_FRAME_MS = 80;

const wardrobeSlots = ["skin", "cap", "hair", "face", "neck", "coat", "belt", "pants", "shoes"] as const;
type WardrobeSlotKey = (typeof wardrobeSlots)[number];
const slots = [...wardrobeSlots, "weapon"] as const;
type SlotKey = (typeof slots)[number];

const slotLabels: Record<string, string> = {
  all: "全部",
  skin: "皮肤",
  cap: "帽子",
  hair: "头部",
  face: "脸部",
  neck: "胸部",
  coat: "上衣",
  belt: "腰带",
  pants: "裤子",
  shoes: "鞋子",
  weapon: "武器",
};

const professionLabels = [
  ["all", "全部职业"],
  ["swordman", "鬼剑士(男)"],
  ["at_swordman", "鬼剑士(女)"],
  ["fighter", "格斗家(女)"],
  ["at_fighter", "格斗家(男)"],
  ["gunner", "神枪手(男)"],
  ["at_gunner", "神枪手(女)"],
  ["mage", "魔法师(女)"],
  ["at_mage", "魔法师(男)"],
  ["priest", "圣职者(男)"],
  ["at_priest", "圣职者(女)"],
  ["thief", "暗夜使者"],
  ["knight", "守护者"],
  ["demoniclancer", "魔枪士"],
  ["gunblader", "枪剑士"],
  ["archer", "弓箭手"],
  ["imperialknight", "帝国骑士"],
  ["demonic_swordman", "黑暗武士"],
  ["creator_mage", "缔造者"],
] as const;
type ProfessionKey = (typeof professionLabels)[number][0];

const state = {
  gameDir: "",
  query: "",
  mode: "sets" as Mode,
  workspaceMode: "current" as WorkspaceMode,
  selectedSlot: "all",
  selectedProfession: "all",
  loading: false,
  rendering: false,
  exporting: false,
  playing: false,
  frame: 0,
  activePlayableRoleKey: "",
  status: "准备载入内置数据",
  catalogMeta: null as CatalogMetadataResponse | null,
  resultItems: [] as AvatarItem[],
  resultSets: [] as AvatarSet[],
  selectedEntities: new Map<string, AvatarItem>(),
  selectedSetItems: [] as AvatarItem[],
  catalogQueryStarted: false,
  catalogQueryLoading: false,
  catalogQueryError: "",
  galleryCards: [] as GalleryCard[],
  galleryTotal: 0,
  galleryResolved: 0,
  galleryNextOffset: null as number | null,
  galleryPage: 1,
  galleryLoading: false,
  gallerySnapshot: null as GalleryMemorySnapshot | null,
  gallerySnapshotPhase: "idle" as "idle" | "decoding" | "ready",
  gallerySnapshotDecoded: 0,
  gallerySnapshotTotal: 0,
  galleryBatchGameDir: "",
  galleryBatchBuildId: "",
  galleryStarted: false,
  galleryError: "",
  galleryBatch: null as CatalogThumbnailBatchStatus | null,
  selectedSet: null as AvatarSet | null,
  selectedItem: null as AvatarItem | null,
  outfit: Object.fromEntries(slots.map((slot) => [slot, null])) as Record<SlotKey, AvatarItem | null>,
  activeLayers: [] as AvatarLayer[],
  missingImgs: [] as string[],
  animationFrames: [] as PreviewImage[],
  animationImageData: [] as ImageData[],
  animationCacheKey: "",
  playFrameIndex: 0,
  lastFrameAt: 0,
  lastStatusAt: 0,
  playTimer: 0,
  outfitSwapLinearDodgeEnabled: false,
  wardrobeSwapEntryVisible: false,
  outfitSwapOpen: false,
  wardrobeOpen: false,
  wardrobe: { current: null, target: null, groups: [], entries: [] } as WardrobeStateView,
  wardrobeSelectedGroup: "all",
  wardrobeSelectedEntryId: "",
  wardrobeQuery: "",
  wardrobeProfession: "all",
  wardrobeSort: "updated",
  wardrobeDialogMode: null as "create" | "edit" | null,
  wardrobePendingSnapshot: null as WardrobeSnapshotInput | null,
  wardrobeEditingEntryId: "",
  savedCurrentOutfit: null as SavedOutfitView | null,
  savedTargetOutfit: null as SavedOutfitView | null,
  outfitSwapLoading: false,
  outfitSwapExporting: false,
  outfitSwapPlan: null as OutfitSwapPlan | null,
  outfitSwapError: "",
  outfitSwapErrorTitle: "",
  outfitSwapPreviewErrors: { current: "", target: "" },
  outfitSwapProgress: null as OutfitSwapProgress | null,
  singleCarrierDialogOpen: false,
  singleCarrierLoading: false,
  singleCarrierError: "",
  singleCarrierCandidates: [] as ProfessionWideCarrierCandidate[],
  singleCarrierSlot: "",
  singleCarrierPlan: null as ProfessionWideSwapPlan | null,
};

let outfitSwapPageCache: { key: string; result: OutfitSwapPageResult } | null = null;
let outfitSwapLoadRequestId = 0;
const outfitSwapPreviewGate = createOutfitSwapPreviewGate();
let singleCarrierRequestId = 0;
const catalogSelectionGate = createCatalogSelectionGate();
const catalogPreviewGate = createCatalogPreviewGate();
const galleryRequestGate = createGalleryRequestGate();
const galleryThumbnailScheduler = createGalleryThumbnailScheduler(4);
const wardrobeExactPreviewScheduler = createGalleryThumbnailScheduler(2, 64);
let gallerySearchTimer = 0;
let galleryBatchPollTimer = 0;
let galleryRefreshGeneration = 0;
let gallerySnapshotPageElements: HTMLElement[] = [];
let gallerySnapshotVisiblePageIndex = -1;
const galleryThumbnailObserver = new IntersectionObserver((entries) => {
  for (const entry of entries) {
    if (!entry.isIntersecting || !(entry.target instanceof HTMLElement)) continue;
    galleryThumbnailObserver.unobserve(entry.target);
    void loadVisibleGalleryThumbnail(entry.target);
  }
}, { rootMargin: "280px 0px" });

type CatalogQueryRequest = {
  mode: Mode;
  slot: string;
  playableRole: string;
  query: string;
};

type CatalogQueryResult =
  | { mode: "items"; items: AvatarItem[] }
  | { mode: "sets"; sets: AvatarSet[] };

const catalogQueryController = createLatestCatalogQuery<
  CatalogQueryRequest,
  CatalogQueryResult
>({
  execute: executeCatalogQuery,
  onResult(result) {
    if (result.mode === "items") {
      state.resultItems = result.items;
      state.status = `找到 ${result.items.length.toLocaleString()} 个单件`;
    } else {
      state.resultSets = result.sets;
      state.status = `找到 ${result.sets.length.toLocaleString()} 个套装`;
    }
    state.catalogQueryLoading = false;
    state.catalogQueryError = "";
    render();
  },
  onError(error) {
    state.catalogQueryLoading = false;
    state.catalogQueryError = String(error);
    state.status = `内置数据查询失败：${String(error)}`;
    render();
  },
});

const wardrobePreviewBoundsCache = new Map<string, OpaqueBounds | null>();
const wardrobeExactPreviewCache = new Map<string, Promise<HTMLCanvasElement>>();
const wardrobePreviewVisibleCanvases = new WeakSet<HTMLCanvasElement>();
let wardrobePreviewRequestSequence = 0;
const wardrobePreviewResizeObserver = new ResizeObserver((entries) => {
  for (const observed of entries) {
    if (
      observed.target instanceof HTMLCanvasElement
      && wardrobePreviewVisibleCanvases.has(observed.target)
    ) {
      void drawWardrobePreviewCanvas(observed.target);
    }
  }
});
const wardrobePreviewVisibilityObserver = new IntersectionObserver((entries) => {
  for (const observed of entries) {
    if (!(observed.target instanceof HTMLCanvasElement)) continue;
    const canvas = observed.target;
    if (observed.isIntersecting) {
      wardrobePreviewVisibleCanvases.add(canvas);
      wardrobePreviewResizeObserver.observe(canvas);
    } else {
      wardrobePreviewVisibleCanvases.delete(canvas);
      wardrobePreviewResizeObserver.unobserve(canvas);
      canvas.dataset.wardrobePreviewRequest = "";
      delete canvas.dataset.wardrobePreviewPending;
      delete canvas.dataset.wardrobePreviewPendingKey;
      delete canvas.dataset.wardrobePreviewRendered;
      canvas.width = 1;
      canvas.height = 1;
    }
  }
}, { rootMargin: "240px 0px" });

type ResizeTarget = "sidebar" | "workspace" | "outfit" | "layers";
type LayoutSizes = {
  sidebar: number;
  resultPane: number;
  outfit: number;
  layers: number;
};

const LAYOUT_STORAGE_KEY = "dnf-avatar-studio-layout-v1";
const layoutDefaults: LayoutSizes = {
  sidebar: 340,
  resultPane: 420,
  outfit: 326,
  layers: 220,
};
const layoutMinimums: LayoutSizes = {
  sidebar: 280,
  resultPane: 360,
  outfit: 120,
  layers: 140,
};
const resizeTargetKeys: Record<ResizeTarget, keyof LayoutSizes> = {
  sidebar: "sidebar",
  workspace: "resultPane",
  outfit: "outfit",
  layers: "layers",
};
const resizeTargetCursors: Record<ResizeTarget, string> = {
  sidebar: "col-resize",
  workspace: "col-resize",
  outfit: "row-resize",
  layers: "row-resize",
};
const WORKSPACE_SPLIT_WIDTH = 18;
const PREVIEW_MIN_WIDTH = 520;
const RESULT_LIST_MIN_HEIGHT = 160;
const CANVAS_MIN_HEIGHT = 260;
let layoutSizes: LayoutSizes = { ...layoutDefaults };
let layoutResizeListenerAttached = false;
let configSaveTimer = 0;
let wardrobeStateLoadPromise: Promise<void> | null = null;
let hiddenFeatureUnlockController: HiddenFeatureUnlockController<AppConfigView>;
let avatarDyePageController: AvatarDyePageController;

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) {
  throw new Error("Missing app root");
}
const appRoot = app;

const mainShellHtml = `
  <div class="shell">
    <aside class="sidebar">
      <div class="brand">
        <img class="brand-mark" src="${brandIconUrl}" alt="" aria-hidden="true" />
        <div>
          <h1>薛定谔的化裁间</h1>
          <p>Powered by 物理世界的欺骗 · 软件完全免费</p>
        </div>
      </div>

      <div class="path-card">
        <label>NPK 目录</label>
        <div class="path-row">
          <input id="gameDir" spellcheck="false" />
          <button id="pickGame" class="icon-btn" title="选择 NPK 目录"></button>
        </div>
        <button id="loadCatalog" class="primary-btn"></button>
      </div>

      <div class="workspace-mode-card">
        <label>浏览视图</label>
        <div class="workspace-mode-switch" role="group" aria-label="切换目录浏览视图">
          <button id="workspaceModeCurrent" class="workspace-mode-button active" type="button" aria-pressed="true">☷ 列表模式</button>
          <button id="workspaceModeGallery" class="workspace-mode-button" type="button" aria-pressed="false">▦ 图库模式</button>
        </div>
      </div>

      <div class="stats" id="stats"></div>

      <div class="filter-card">
        <label>职业</label>
        <div class="chip-grid professions" id="professionChips"></div>
      </div>
    </aside>

    <div class="resize-handle resize-handle-y shell-resize" data-resize="sidebar" role="separator" aria-orientation="vertical" aria-label="调整侧栏宽度" title="拖动调整侧栏宽度，双击还原"></div>

    <main class="main">
      <header class="toolbar">
        <div class="segmented">
          <button id="modeSets" class="segment active"></button>
          <button id="modeItems" class="segment"></button>
          <button id="modeWardrobe" class="segment" hidden></button>
          <button id="modeOutfitSwap" class="segment" hidden></button>
        </div>
        <div id="swapPreprocessTools" class="swap-preprocess-tools" hidden>
          <span>目标预处理</span>
          <button id="swapLinearDodge" type="button" aria-pressed="false" title="预览及生成补丁前，对目标外观的每个时装和武器图层应用 DNFAuraplay 线性减淡预处理">线性减淡</button>
          <em>影响目标预览和生成补丁</em>
        </div>
        <div class="searchbox">
          <span id="searchIcon"></span>
          <input id="query" placeholder="搜索套装、礼盒、部位或单件时装" spellcheck="false" />
        </div>
        <div class="status" id="status"></div>
      </header>

      <section class="workspace" id="studioWorkspace">
        <section class="result-pane">
          <div class="pane-title" id="resultTitle"></div>
          <div class="outfit-grid" id="outfitGrid"></div>
          <div class="resize-handle resize-handle-x outfit-resize" data-resize="outfit" role="separator" aria-orientation="horizontal" aria-label="调整搭配区高度" title="拖动调整搭配区高度，双击还原"></div>
          <div id="resultList" class="result-list"></div>
        </section>

        <div class="resize-handle resize-handle-y workspace-resize" data-resize="workspace" role="separator" aria-orientation="vertical" aria-label="调整列表和预览宽度" title="拖动调整列表和预览宽度，双击还原"></div>

        <section class="preview-pane">
          <div class="preview-head">
            <div>
              <div class="eyebrow" id="previewKind">未选择</div>
              <h2 id="previewTitle">等待载入</h2>
            </div>
            <div class="preview-actions">
              <button id="openAvatarDye" class="icon-text-btn" title="打开时装染色工作台" hidden></button>
              <button id="saveCurrentOutfit" class="icon-text-btn" title="保存为当前穿戴" hidden></button>
              <button id="saveTargetOutfit" class="icon-text-btn" title="保存为目标外观" hidden></button>
              <button id="saveToWardrobe" class="icon-text-btn" title="保存到时装橱窗" hidden></button>
              <button id="playPreview" class="icon-text-btn" title="播放预览"></button>
              <button id="rerender" class="icon-text-btn" title="重新合成"></button>
              <button id="exportComposed" class="icon-text-btn" title="导出合成 NPK"></button>
              <button id="exportOriginal" class="icon-text-btn" title="导出原始 IMG NPK"></button>
            </div>
          </div>
          <div class="canvas-wrap">
            <canvas id="previewCanvas" width="640" height="520"></canvas>
          </div>
          <div class="resize-handle resize-handle-x layer-resize" data-resize="layers" role="separator" aria-orientation="horizontal" aria-label="调整图层表高度" title="拖动调整图层表高度，双击还原"></div>
          <div class="layer-table-wrap">
            <table id="layerTable" class="layer-table">
              <thead>
                <tr id="layerTableHead">
                  <th>部位</th>
                  <th>层</th>
                  <th>IMG</th>
                  <th>状态</th>
                  <th>染色</th>
                </tr>
              </thead>
              <tbody id="layerRows"></tbody>
            </table>
          </div>
        </section>

        <section id="catalogGallery" class="catalog-gallery" aria-label="目录缩略图库">
          <header class="gallery-toolbar">
            <div>
              <span class="gallery-eyebrow">Gallery</span>
              <strong id="galleryTitle">预览图库</strong>
              <em id="galleryMeta"></em>
            </div>
            <div class="gallery-actions">
              <button id="galleryBatchButton" class="icon-text-btn" type="button"></button>
              <button id="galleryLibraryButton" class="icon-text-btn" type="button">预览库</button>
            </div>
          </header>
          <div id="galleryBatchStatus" class="gallery-batch-status" hidden></div>
          <div id="galleryGrid" class="gallery-grid"></div>
          <nav id="galleryPager" class="gallery-pager" aria-label="图库分页" hidden>
            <button id="galleryPagePrevious" type="button">上一页</button>
            <span id="galleryPageStatus"></span>
            <button id="galleryPageNext" type="button">下一页</button>
          </nav>
        </section>
      </section>

      <section id="avatarDyePage" class="avatar-dye-page" hidden>
        <header class="avatar-dye-context-bar">
          <button id="avatarDyeBack" class="avatar-dye-button">← 返回</button>
          <div class="avatar-dye-context-copy">
            <span>Dye Studio</span>
            <strong id="avatarDyeTitle">时装染色</strong>
          </div>
          <div id="avatarDyeDraftStatus" class="avatar-dye-draft-status">等待打开会话</div>
          <div class="avatar-dye-context-actions">
            <button id="avatarDyeRefresh" class="avatar-dye-button">刷新</button>
            <button id="avatarDyeUndo" class="avatar-dye-button">↶ 撤销</button>
            <button id="avatarDyeRedo" class="avatar-dye-button">↷ 重做</button>
            <button id="avatarDyeResetAll" class="avatar-dye-button danger">恢复全部</button>
          </div>
        </header>

        <div class="avatar-dye-workspace">
          <aside class="avatar-dye-assets-pane">
            <div class="avatar-dye-pane-head">
              <div><span>Targets</span><strong>染色对象</strong></div>
              <em id="avatarDyeTargetSummary">0 个目标</em>
            </div>
            <div class="avatar-dye-filter-row">
              <button class="avatar-dye-chip active" data-dye-object-mode="all">全部</button>
              <button class="avatar-dye-chip" data-dye-object-mode="part">部位</button>
              <button class="avatar-dye-chip" data-dye-object-mode="img">IMG</button>
            </div>
            <div id="avatarDyeResourceNotice" class="avatar-dye-resource-notice" hidden></div>
            <div id="avatarDyeTargetTree" class="avatar-dye-assets-scroll"></div>
          </aside>

          <section class="avatar-dye-preview-pane">
            <div class="avatar-dye-pane-head">
              <div><span>Live Render</span><strong>实时合成预览</strong></div>
              <em id="avatarDyePreviewMeta">静态帧</em>
            </div>
            <div class="avatar-dye-preview-controls">
              <div><span>显示</span><button class="avatar-dye-chip active" data-dye-preview-scope="outfit">整套</button><button class="avatar-dye-chip" data-dye-preview-scope="part">当前部位</button><button class="avatar-dye-chip" data-dye-preview-scope="target">当前 IMG</button><button id="avatarDyeHideSelectedMask" class="avatar-dye-chip">隐藏当前 Mask</button></div>
              <small>仅改变显示，不改变染色对象</small>
            </div>
            <div class="avatar-dye-canvas-wrap avatar-dye-preview-comparison" role="group" aria-label="原图与改色后完整对比预览">
              <div class="avatar-dye-preview-side">
                <span class="avatar-dye-preview-side-label">原图</span>
                <canvas id="avatarDyeOriginalCanvas" width="640" height="560" aria-label="完整原图预览"></canvas>
                <div id="avatarDyeCanvasStatus" class="avatar-dye-canvas-status">选择目标后加载真实帧</div>
              </div>
              <div class="avatar-dye-preview-divider" aria-hidden="true"></div>
              <div class="avatar-dye-preview-side">
                <span class="avatar-dye-preview-side-label">改色后</span>
                <canvas id="avatarDyeDyedCanvas" width="640" height="560" aria-label="完整改色后预览"></canvas>
              </div>
            </div>
            <div class="avatar-dye-timeline">
              <button id="avatarDyePlay" class="avatar-dye-button" aria-label="播放染色动画">▶</button>
              <input id="avatarDyeFrame" type="range" min="0" max="0" value="0" />
              <span id="avatarDyeFrameLabel">帧 0 / 0</span>
              <button id="avatarDyeFitCanvas" class="avatar-dye-button" title="按当前可用区域等比缩放，仅影响预览">适应画布</button>
            </div>
          </section>

          <aside class="avatar-dye-editor-pane">
            <div class="avatar-dye-editor-context">
              <div class="avatar-dye-editor-title"><div><span>Palette Editor</span><strong id="avatarDyeEditorName">未选择目标</strong></div><em id="avatarDyeEditorChanged">原色</em></div>
              <div id="avatarDyeEditorMeta" class="avatar-dye-editor-meta"></div>
              <div id="avatarDyeEditorTabs" class="avatar-dye-editor-tabs" hidden></div>
            </div>
            <div id="avatarDyeEditorScroll" class="avatar-dye-editor-scroll"></div>
          </aside>
        </div>

        <footer class="avatar-dye-workspace-bar">
          <div id="avatarDyeCurrentSummary" class="avatar-dye-snapshot-summary"></div>
          <span class="avatar-dye-workspace-arrow">→</span>
          <div id="avatarDyeTargetWorkspaceSummary" class="avatar-dye-snapshot-summary"></div>
          <div class="avatar-dye-workspace-actions">
            <button id="avatarDyeSaveWardrobe" class="avatar-dye-button">保存到橱窗</button>
            <button id="avatarDyeSaveWorkspace" class="avatar-dye-button">写入工作区</button>
            <button id="avatarDyeGoSwap" class="avatar-dye-button primary">设为目标并前往一键换装</button>
            <button id="avatarDyeExport" class="avatar-dye-button">导出染色补丁</button>
          </div>
        </footer>
      </section>

      <section id="wardrobePage" class="wardrobe-page" hidden>
        <aside class="wardrobe-groups-pane">
          <div class="wardrobe-pane-head">
            <div><span>Wardrobe</span><strong>时装橱窗</strong></div>
            <div class="wardrobe-pane-actions">
              <button id="wardrobeImportShare" class="mini-btn">导入分享</button>
              <button id="wardrobeCreateGroup" class="mini-btn">新建栏目</button>
            </div>
          </div>
          <nav id="wardrobeGroups" class="wardrobe-groups"></nav>
        </aside>

        <section class="wardrobe-library-pane">
          <div class="wardrobe-toolbar">
            <label class="wardrobe-search"><span>搜索</span><input id="wardrobeQuery" placeholder="名称或备注" spellcheck="false" /></label>
            <label><span>职业</span><select id="wardrobeProfession"></select></label>
            <label><span>排序</span><select id="wardrobeSort"><option value="updated">最近修改</option><option value="created">创建时间</option><option value="recent">最近使用</option><option value="name">名称</option></select></label>
            <button id="wardrobeShareGroup" class="icon-text-btn wardrobe-share-group" disabled>分享当前栏目</button>
          </div>
          <div id="wardrobeCards" class="wardrobe-cards"></div>
        </section>

        <aside id="wardrobeDetail" class="wardrobe-detail-pane"></aside>
      </section>

      <section id="outfitSwapPage" class="outfit-swap-page" hidden>
        <section class="swap-snapshot-pane">
          <div class="swap-pane-head">
            <div><span id="swapCurrentEyebrow">当前</span><strong>当前穿戴</strong></div>
            <em id="swapCurrentSavedAt"></em>
          </div>
          <div class="swap-canvas-wrap">
            <canvas id="swapCurrentPreviewCanvas" width="500" height="500"></canvas>
            <div id="swapCurrentPreviewError" class="swap-preview-error" hidden></div>
          </div>
          <div id="swapCurrentItems" class="swap-item-list"></div>
        </section>

        <section class="swap-preview-pane">
          <div class="swap-pane-head">
            <div><span id="swapTargetEyebrow">目标</span><strong>目标外观</strong></div>
            <em id="swapTargetSavedAt"></em>
          </div>
          <div class="swap-canvas-wrap">
            <canvas id="swapPreviewCanvas" width="500" height="500"></canvas>
            <div id="swapTargetPreviewError" class="swap-preview-error" hidden></div>
          </div>
          <div id="swapTargetItems" class="swap-target-grid"></div>
        </section>

        <section class="swap-plan-pane">
          <div class="swap-pane-head">
            <div><span id="swapPlanEyebrow">方案</span><strong>换装分析</strong></div>
            <em id="swapPlanBadge">等待快照</em>
          </div>
          <div id="swapPlanSummary" class="swap-plan-summary"></div>
          <div id="swapPartPlans" class="swap-part-plans"></div>
          <div class="swap-generate-bar">
            <div id="swapExportProgress" class="swap-export-progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0" hidden>
              <div><span id="swapExportProgressStage">准备生成</span><strong id="swapExportProgressPercent">0%</strong></div>
              <div class="swap-export-progress-track"><span id="swapExportProgressFill"></span></div>
            </div>
            <div class="swap-generate-actions">
              <button id="generateWeaponSwap" class="weapon-swap-btn" disabled>武器换装</button>
              <button id="generateSingleCarrierSwap" class="single-carrier-btn" disabled>单时装完整替代</button>
              <button id="generateOutfitSwap" class="primary-btn" disabled>生成换装补丁</button>
            </div>
          </div>
        </section>
      </section>
    </main>
    <div id="singleCarrierDialog" class="single-carrier-dialog-backdrop" hidden>
      <section class="single-carrier-dialog" role="dialog" aria-modal="true" aria-labelledby="singleCarrierTitle">
        <div class="single-carrier-dialog-head">
          <div><span>Profession-wide override</span><strong id="singleCarrierTitle">单时装完整替代</strong></div>
          <button id="singleCarrierClose" class="icon-btn" title="关闭">×</button>
        </div>
        <div class="single-carrier-dialog-body">
          <p class="single-carrier-warning">这是职业级覆盖：目标整套只写入一个载体，该职业其余所有时装 IMG 都会透明化，不只是当前穿戴。</p>
          <label><span>唯一像素载体</span><select id="singleCarrierSelect"></select></label>
          <button id="singleCarrierAnalyze" class="icon-text-btn">分析全部 NPK 影响范围</button>
          <div id="singleCarrierScope" class="single-carrier-scope"></div>
          <div id="singleCarrierError" class="single-carrier-error" role="alert" hidden></div>
        </div>
        <div class="single-carrier-dialog-actions">
          <button id="singleCarrierCancel" class="icon-text-btn">取消</button>
          <button id="singleCarrierExport" class="single-carrier-danger-btn" disabled>生成特殊补丁</button>
        </div>
      </section>
    </div>
    <div id="wardrobeDialog" class="wardrobe-dialog-backdrop" hidden>
      <section class="wardrobe-dialog" role="dialog" aria-modal="true" aria-labelledby="wardrobeDialogTitle">
        <div class="wardrobe-dialog-head"><div><span>Local Wardrobe</span><strong id="wardrobeDialogTitle">保存到时装橱窗</strong></div><button id="wardrobeDialogClose" class="icon-btn" title="关闭">×</button></div>
        <label><span>装扮名称</span><input id="wardrobeName" maxlength="80" spellcheck="false" /></label>
        <label><span>保存栏目</span><select id="wardrobeGroup"></select></label>
        <label><span>备注</span><textarea id="wardrobeNote" maxlength="500" rows="4"></textarea></label>
        <label class="wardrobe-favorite-check"><input id="wardrobeFavorite" type="checkbox" /><span>收藏此装扮</span></label>
        <div class="wardrobe-dialog-actions"><button id="wardrobeDialogCancel" class="icon-text-btn">取消</button><button id="wardrobeDialogSave" class="primary-btn">保存</button></div>
      </section>
    </div>
    <div id="avatarDyeDialog" class="avatar-dye-dialog-backdrop" hidden>
      <section class="avatar-dye-dialog" role="dialog" aria-modal="true" aria-labelledby="avatarDyeDialogTitle">
        <div class="avatar-dye-dialog-head"><div><span>Dye Session</span><strong id="avatarDyeDialogTitle">时装染色</strong></div><button id="avatarDyeDialogClose" class="avatar-dye-button" title="关闭">×</button></div>
        <div id="avatarDyeDialogBody" class="avatar-dye-dialog-body"></div>
        <div id="avatarDyeDialogActions" class="avatar-dye-dialog-actions"></div>
      </section>
    </div>
  </div>
`;

function mountIcon(nodeId: string, iconNode: IconNode, size = 18) {
  const node = document.querySelector(`#${nodeId}`);
  if (!node) return;
  const svg = createElement(iconNode);
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("stroke-width", "2");
  node.replaceChildren(svg);
}

function setupResizableLayout() {
  layoutSizes = { ...layoutDefaults, ...readStoredLayoutSizes() };
  clampAndApplyLayoutSizes();
  document.querySelectorAll<HTMLElement>(".resize-handle[data-resize]").forEach((handle) => {
    handle.addEventListener("pointerdown", startPaneResize);
    handle.addEventListener("dblclick", () => {
      const target = resizeTargetFromHandle(handle);
      if (target) resetLayoutTarget(target);
    });
  });
  if (!layoutResizeListenerAttached) {
    window.addEventListener("resize", clampAndApplyLayoutSizes);
    layoutResizeListenerAttached = true;
  }
}

function readStoredLayoutSizes() {
  try {
    const parsed = JSON.parse(localStorage.getItem(LAYOUT_STORAGE_KEY) ?? "{}") as Partial<Record<keyof LayoutSizes, unknown>>;
    const sizes: Partial<LayoutSizes> = {};
    for (const key of Object.keys(layoutDefaults) as Array<keyof LayoutSizes>) {
      const value = Number(parsed[key]);
      if (Number.isFinite(value)) sizes[key] = value;
    }
    return sizes;
  } catch {
    return {};
  }
}

function persistLayoutSizes() {
  try {
    localStorage.setItem(LAYOUT_STORAGE_KEY, JSON.stringify(layoutSizes));
  } catch {
    // Layout persistence is a convenience; dragging should keep working if storage is unavailable.
  }
}

function resizeTargetFromHandle(handle: HTMLElement): ResizeTarget | null {
  const target = handle.dataset.resize;
  if (target === "sidebar" || target === "workspace" || target === "outfit" || target === "layers") {
    return target;
  }
  return null;
}

function startPaneResize(event: PointerEvent) {
  if (event.button !== 0) return;
  const handle = event.currentTarget as HTMLElement;
  const target = resizeTargetFromHandle(handle);
  if (!target) return;
  event.preventDefault();

  const key = resizeTargetKeys[target];
  const startValue = layoutSizes[key];
  const startX = event.clientX;
  const startY = event.clientY;
  const cursor = resizeTargetCursors[target];

  handle.classList.add("active");
  handle.setPointerCapture(event.pointerId);
  document.body.classList.add("is-resizing");
  document.body.style.cursor = cursor;

  const onPointerMove = (moveEvent: PointerEvent) => {
    const deltaX = moveEvent.clientX - startX;
    const deltaY = moveEvent.clientY - startY;
    let next = startValue;
    if (target === "sidebar" || target === "workspace") next += deltaX;
    if (target === "outfit") next += deltaY;
    if (target === "layers") next -= deltaY;
    layoutSizes[key] = clampLayoutSize(target, next);
    applyLayoutSizes();
  };

  const stopResize = (endEvent: PointerEvent) => {
    handle.removeEventListener("pointermove", onPointerMove);
    handle.classList.remove("active");
    document.body.classList.remove("is-resizing");
    document.body.style.cursor = "";
    if (handle.hasPointerCapture(endEvent.pointerId)) {
      handle.releasePointerCapture(endEvent.pointerId);
    }
    persistLayoutSizes();
  };

  handle.addEventListener("pointermove", onPointerMove);
  handle.addEventListener("pointerup", stopResize, { once: true });
  handle.addEventListener("pointercancel", stopResize, { once: true });
}

function resetLayoutTarget(target: ResizeTarget) {
  const key = resizeTargetKeys[target];
  layoutSizes[key] = layoutDefaults[key];
  clampAndApplyLayoutSizes();
  persistLayoutSizes();
}

function clampAndApplyLayoutSizes() {
  (Object.keys(resizeTargetKeys) as ResizeTarget[]).forEach((target) => {
    const key = resizeTargetKeys[target];
    layoutSizes[key] = clampLayoutSize(target, layoutSizes[key]);
  });
  applyLayoutSizes();
}

function clampLayoutSize(target: ResizeTarget, value: number) {
  const key = resizeTargetKeys[target];
  const min = layoutMinimums[key];
  const max = Math.max(min, maxLayoutSize(target));
  return Math.min(Math.max(Math.round(value), min), max);
}

function maxLayoutSize(target: ResizeTarget) {
  if (target === "sidebar") {
    return Math.min(560, Math.max(layoutMinimums.sidebar, window.innerWidth - 900));
  }
  if (target === "workspace") {
    const workspace = document.querySelector<HTMLElement>(".workspace");
    if (!workspace) return layoutDefaults.resultPane;
    return contentBoxWidth(workspace) - WORKSPACE_SPLIT_WIDTH - PREVIEW_MIN_WIDTH;
  }
  if (target === "outfit") {
    const pane = document.querySelector<HTMLElement>(".result-pane");
    const title = document.querySelector<HTMLElement>(".pane-title");
    const handle = document.querySelector<HTMLElement>(".outfit-resize");
    if (!pane || !title || !handle) return layoutDefaults.outfit;
    return pane.clientHeight - title.offsetHeight - handle.offsetHeight - RESULT_LIST_MIN_HEIGHT;
  }
  const pane = document.querySelector<HTMLElement>(".preview-pane");
  const head = document.querySelector<HTMLElement>(".preview-head");
  const handle = document.querySelector<HTMLElement>(".layer-resize");
  if (!pane || !head || !handle) return layoutDefaults.layers;
  return pane.clientHeight - head.offsetHeight - handle.offsetHeight - CANVAS_MIN_HEIGHT;
}

function applyLayoutSizes() {
  const shell = document.querySelector<HTMLElement>(".shell");
  if (!shell) return;
  shell.style.setProperty("--sidebar-width", `${layoutSizes.sidebar}px`);
  shell.style.setProperty("--result-pane-width", `${layoutSizes.resultPane}px`);
  shell.style.setProperty("--outfit-grid-height", `${layoutSizes.outfit}px`);
  shell.style.setProperty("--layer-table-height", `${layoutSizes.layers}px`);
  updateResizeHandleValues();
}

function updateResizeHandleValues() {
  document.querySelectorAll<HTMLElement>(".resize-handle[data-resize]").forEach((handle) => {
    const target = resizeTargetFromHandle(handle);
    if (!target) return;
    const key = resizeTargetKeys[target];
    handle.setAttribute("aria-valuemin", String(layoutMinimums[key]));
    handle.setAttribute("aria-valuemax", String(Math.max(layoutMinimums[key], maxLayoutSize(target))));
    handle.setAttribute("aria-valuenow", String(layoutSizes[key]));
  });
}

function contentBoxWidth(element: HTMLElement) {
  const style = getComputedStyle(element);
  return element.clientWidth - cssPixels(style.paddingLeft) - cssPixels(style.paddingRight);
}

function cssPixels(value: string) {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

let gameDirInput: HTMLInputElement;
let queryInput: HTMLInputElement;

function mountMainShell() {
  appRoot.innerHTML = mainShellHtml;
  setupResizableLayout();
  mountIcon("pickGame", FolderOpen);
mountIcon("searchIcon", Search, 17);
mountIcon("modeSets", Layers3, 17);
mountIcon("modeItems", Shirt, 17);
mountIcon("modeWardrobe", BookOpen, 17);
mountIcon("modeOutfitSwap", PackageOpen, 17);
mountIcon("openAvatarDye", Palette, 16);
mountIcon("saveCurrentOutfit", Box, 16);
mountIcon("saveTargetOutfit", WandSparkles, 16);
mountIcon("saveToWardrobe", SaveIcon, 16);
mountIcon("rerender", RefreshCw, 16);
mountIcon("loadCatalog", WandSparkles, 17);
mountIcon("playPreview", Play, 16);
mountIcon("exportComposed", Download, 16);
mountIcon("exportOriginal", PackageOpen, 16);
document.querySelector("#loadCatalog")?.append("载入内置数据");
document.querySelector("#modeSets")?.append("套装");
document.querySelector("#modeItems")?.append("单件");
document.querySelector("#modeWardrobe")?.append("时装橱窗");
document.querySelector("#modeOutfitSwap")?.append("一键换装");
document.querySelector("#openAvatarDye")?.append("染色");
document.querySelector("#saveCurrentOutfit")?.append("保存为当前穿戴");
document.querySelector("#saveTargetOutfit")?.append("保存为目标外观");
document.querySelector("#saveToWardrobe")?.append("存入橱窗");
document.querySelector("#rerender")?.append("重绘");
document.querySelector("#playPreview")?.append("播放");
document.querySelector("#exportComposed")?.append("合成");
document.querySelector("#exportOriginal")?.append("原图");
  gameDirInput = document.querySelector<HTMLInputElement>("#gameDir")!;
  queryInput = document.querySelector<HTMLInputElement>("#query")!;
  hiddenFeatureUnlockController = createHiddenFeatureUnlockController<AppConfigView>({
    invoke: (command, arguments_) => invoke<HiddenFeatureUnlockResult>(command, arguments_),
    getInputValue: () => queryInput.value,
    clearInput: () => {
      catalogQueryController.invalidate();
      state.query = "";
      state.catalogQueryLoading = false;
      state.catalogQueryError = "";
      queryInput.value = "";
    },
    applyConfig: applyVerifiedFeatureConfig,
    onMatched: handleHiddenFeatureMatched,
    onError: handleHiddenFeatureUnlockError,
  });
  avatarDyePageController = createAvatarDyePageController(avatarDyePageHost());
  avatarDyePageController.wire();
  void getCurrentWindow().onCloseRequested((event) => {
    if (avatarDyePageController.requestWindowClose()) {
      event.preventDefault();
      return;
    }
    disposeActiveGallerySnapshot();
  });
}

async function boot() {
  mountMainShell();
  await init();
}

function ensureWardrobeStateLoaded() {
  if (!wardrobeStateLoadPromise) {
    wardrobeStateLoadPromise = loadWardrobeStateWithRecovery();
  }
  return wardrobeStateLoadPromise;
}

async function loadWardrobeStateWithRecovery() {
  try {
    const unchecked = await invoke<WardrobeStateView>("load_wardrobe_state");
    applyWardrobeState(unchecked);
    void validateWardrobeStateInBackground(wardrobePublicIdentity(unchecked));
  } catch (error) {
    state.status = `时装橱窗读取失败：${String(error)}`;
    if (window.confirm("时装橱窗主文件和备份均无法读取。是否确认重置为空橱窗？该操作不会生成明文配置。")) {
      try {
        applyWardrobeState(await invoke<WardrobeStateView>("reset_wardrobe_after_confirmation", { confirmed: true }));
        state.status = "损坏的时装橱窗已按确认重置";
      } catch (resetError) {
        state.status = `时装橱窗重置失败：${String(resetError)}`;
      }
    }
  }
}

async function validateWardrobeStateInBackground(capturedIdentity: string) {
  try {
    const validated = await invoke<WardrobeStateView>("validate_wardrobe_state");
    if (capturedIdentity !== wardrobePublicIdentity(state.wardrobe)) return;
    if (capturedIdentity !== wardrobePublicIdentity(validated)) return;
    applyWardrobeState(validated);
  } catch {
    if (capturedIdentity !== wardrobePublicIdentity(state.wardrobe)) return;
    state.status = "时装橱窗资源后台校验失败；已保留加密缩略图，可稍后刷新重试";
    render();
  }
}

function wardrobePublicIdentity(view: WardrobeStateView) {
  return JSON.stringify([
    view.current?.revision ?? null,
    view.target?.revision ?? null,
    view.groups.map((group) => [group.id, group.name, group.sort_order]),
    view.entries.map((entry) => [
      entry.id,
      entry.snapshot.revision,
      entry.group_id,
      entry.favorite,
      entry.updated_at_ms,
      entry.last_used_at_ms,
      entry.is_current,
      entry.is_target,
    ]),
  ]);
}

async function init() {
  localStorage.removeItem("dnf-avatar-studio-outfit-snapshots-v1");
  const defaultGameDir = await invoke<string>("default_game_dir");
  let shouldAutoOpenCatalogIndex = false;
  try {
    const config = await invoke<AppConfigView>("load_app_config");
    const savedGameDir = config.game_dir.trim();
    shouldAutoOpenCatalogIndex = Boolean(savedGameDir);
    state.gameDir = savedGameDir || defaultGameDir;
    applyVerifiedFeatureConfig(config);
    if (hasInvalidHiddenFeatureCredential(config)) {
      state.status = "功能激活凭据无效，需要重新输入指令";
    }
    if (config.wardrobe_swap_unlocked) {
      await ensureWardrobeStateLoaded();
    }
  } catch (error) {
    state.gameDir = defaultGameDir;
    state.status = `配置读取失败：${String(error)}`;
  }
  gameDirInput.value = state.gameDir;
  wireEvents();
  render();
  clearCanvas();
  if (shouldAutoOpenCatalogIndex) {
    await openCatalogIndex();
  }
}

function wireEvents() {
  document.querySelector("#pickGame")?.addEventListener("click", async () => {
    const selected = await open({ directory: true, multiple: false, defaultPath: state.gameDir });
    if (typeof selected === "string") {
      stopPlayback();
      resetAnimationCache();
      invalidateCatalogSelection();
      setGameDirectory(selected);
      gameDirInput.value = selected;
      scheduleConfigSave();
      if (state.workspaceMode === "gallery" && state.catalogMeta) void refreshGallery();
    }
  });

  document.querySelector("#loadCatalog")?.addEventListener("click", openCatalogIndex);
  document.querySelector("#workspaceModeCurrent")?.addEventListener("click", () => setWorkspaceMode("current"));
  document.querySelector("#workspaceModeGallery")?.addEventListener("click", () => setWorkspaceMode("gallery"));
  document.querySelector("#modeSets")?.addEventListener("click", () => setMode("sets"));
  document.querySelector("#modeItems")?.addEventListener("click", () => setMode("items"));
  document.querySelector("#galleryBatchButton")?.addEventListener("click", () => void startGalleryBatch());
  document.querySelector("#galleryLibraryButton")?.addEventListener("click", () => void showPreviewLibrary());
  document.querySelector("#galleryPagePrevious")?.addEventListener("click", () => void changeGalleryPage(-1));
  document.querySelector("#galleryPageNext")?.addEventListener("click", () => void changeGalleryPage(1));
  document.querySelector("#galleryGrid")?.addEventListener("click", (event) => {
    const button = event.target instanceof Element
      ? event.target.closest<HTMLButtonElement>("[data-gallery-set], [data-gallery-item]")
      : null;
    if (!button) return;
    const setId = button.dataset.gallerySet;
    const itemId = button.dataset.galleryItem;
    if (setId) void selectSet(setId);
    if (itemId) void selectItem(itemId);
  });
  document.querySelector("#modeWardrobe")?.addEventListener("click", () => enterWardrobePage());
  document.querySelector("#modeOutfitSwap")?.addEventListener("click", () => enterOutfitSwapPage());
  document.querySelector("#openAvatarDye")?.addEventListener("click", () => void openAvatarDyeFromStudio());
  document.querySelector("#saveCurrentOutfit")?.addEventListener("click", () => void saveOutfitSnapshot("current"));
  document.querySelector("#saveTargetOutfit")?.addEventListener("click", () => void saveOutfitSnapshot("target"));
  document.querySelector("#saveToWardrobe")?.addEventListener("click", () => openCreateWardrobeDialog());
  document.querySelector("#generateWeaponSwap")?.addEventListener("click", () => void exportWeaponSwap());
  document.querySelector("#generateOutfitSwap")?.addEventListener("click", () => exportOutfitSwap());
  document.querySelector("#generateSingleCarrierSwap")?.addEventListener("click", () => void openSingleCarrierDialog());
  document.querySelector("#swapLinearDodge")?.addEventListener("click", () => {
    if (state.outfitSwapExporting) return;
    state.outfitSwapLinearDodgeEnabled = !state.outfitSwapLinearDodgeEnabled;
    state.singleCarrierPlan = null;
    if (state.singleCarrierDialogOpen) {
      state.singleCarrierError = "目标预处理已改变，请重新分析影响范围";
    }
    state.status = state.outfitSwapLinearDodgeEnabled
      ? "已对目标预览启用线性减淡，生成补丁时也会应用"
      : "目标线性减淡已关闭";
    render();
    refreshOutfitSwapTargetPreview();
  });
  document.querySelector("#rerender")?.addEventListener("click", () => void rerenderActivePreview());
  document.querySelector("#playPreview")?.addEventListener("click", togglePlayback);
  document.querySelector("#exportComposed")?.addEventListener("click", () => exportNpk("composed"));
  document.querySelector("#exportOriginal")?.addEventListener("click", () => exportNpk("original"));
  gameDirInput.addEventListener("change", () => {
    stopPlayback();
    resetAnimationCache();
    invalidateCatalogSelection();
    setGameDirectory(gameDirInput.value.trim());
    scheduleConfigSave();
    if (state.workspaceMode === "gallery" && state.catalogMeta) void refreshGallery();
  });
  queryInput.addEventListener("input", () => {
    state.query = queryInput.value.trim();
    if (state.workspaceMode === "gallery") invalidateGalleryPresentation();
    refreshCatalogQuery(true);
  });
  queryInput.addEventListener("keydown", (event) => {
    if (!shouldSubmitHiddenFeatureUnlock(event)) return;
    void unlockHiddenFeature(queryInput.value);
  });
  document.querySelector<HTMLInputElement>("#wardrobeQuery")?.addEventListener("input", (event) => {
    state.wardrobeQuery = (event.currentTarget as HTMLInputElement).value;
    renderWardrobePage();
  });
  document.querySelector<HTMLSelectElement>("#wardrobeProfession")?.addEventListener("change", (event) => {
    state.wardrobeProfession = (event.currentTarget as HTMLSelectElement).value;
    renderWardrobePage();
  });
  document.querySelector<HTMLSelectElement>("#wardrobeSort")?.addEventListener("change", (event) => {
    state.wardrobeSort = (event.currentTarget as HTMLSelectElement).value;
    renderWardrobePage();
  });
  document.querySelector("#wardrobeCreateGroup")?.addEventListener("click", () => void createWardrobeGroup());
  document.querySelector("#wardrobeImportShare")?.addEventListener("click", () => void importWardrobeShare());
  document.querySelector("#wardrobeShareGroup")?.addEventListener("click", () => {
    const groupId = selectedWritableGroup();
    if (groupId) void shareWardrobeGroup(groupId);
  });
  document.querySelector("#wardrobeDialogClose")?.addEventListener("click", closeWardrobeDialog);
  document.querySelector("#wardrobeDialogCancel")?.addEventListener("click", closeWardrobeDialog);
  document.querySelector("#wardrobeDialogSave")?.addEventListener("click", () => void submitWardrobeDialog());
  document.querySelector("#singleCarrierClose")?.addEventListener("click", closeSingleCarrierDialog);
  document.querySelector("#singleCarrierCancel")?.addEventListener("click", closeSingleCarrierDialog);
  document.querySelector<HTMLSelectElement>("#singleCarrierSelect")?.addEventListener("change", (event) => {
    state.singleCarrierSlot = (event.currentTarget as HTMLSelectElement).value;
    state.singleCarrierPlan = null;
    state.singleCarrierError = "";
    renderSingleCarrierDialog();
  });
  document.querySelector("#singleCarrierAnalyze")?.addEventListener("click", () => void previewSingleCarrierScope());
  document.querySelector("#singleCarrierExport")?.addEventListener("click", () => void exportSingleCarrierSwap());
}

function avatarDyePageHost(): AvatarDyePageHost {
  return {
    commands: {
      loadDyeColorPresets: () =>
        invoke<DyeColorPresetCatalog>("load_dye_color_presets", {}),
      createDyeColorPreset: (args) =>
        invoke<DyeColorPresetCatalog>("create_dye_color_preset", args),
      updateDyeColorPreset: (args) =>
        invoke<DyeColorPresetCatalog>("update_dye_color_preset", args),
      deleteDyeColorPreset: (args) =>
        invoke<DyeColorPresetCatalog>("delete_dye_color_preset", args),
      openFromStudio: ({ gameDir, snapshot }: { gameDir: string; snapshot: unknown }) =>
        invoke<AvatarDyeSessionView>("open_avatar_dye_session_from_studio", { gameDir, snapshot }),
      openFromWardrobe: ({ gameDir, entryId }: { gameDir: string; entryId: string }) =>
        invoke<AvatarDyeSessionView>("open_avatar_dye_session_from_wardrobe", { gameDir, entryId }),
      getView: ({ sessionId }: { sessionId: string }) =>
        invoke<AvatarDyeSessionView>("get_avatar_dye_session_view", { sessionId }),
      updateColors: ({ sessionId, targetId, operation }: { sessionId: string; targetId: string; operation: DyeColorOperation }) =>
        invoke<AvatarDyeSessionView>("update_avatar_dye_colors", { sessionId, targetId, operation }),
      updateBatch: ({ sessionId, mutation }: { sessionId: string; mutation: DyeBatchMutation }) =>
        invoke<AvatarDyeSessionView>("update_avatar_dye_batch", { sessionId, mutation }),
      previewComparisonColorsFrame: (args) =>
        invoke<DyeComparisonFrameResponse>("preview_avatar_dye_comparison_colors", args),
      previewBatchComparison: (args) =>
        invoke<DyeComparisonFrameResponse>("preview_avatar_dye_batch_comparison", args),
      repairTarget: ({ sessionId, targetId }: { sessionId: string; targetId: string }) =>
        invoke<AvatarDyeSessionView>("repair_avatar_dye_target", { sessionId, targetId }),
      resetScope: (args: { sessionId: string; scope: string; targetId?: string; colorIndices?: number[] }) =>
        invoke<AvatarDyeSessionView>("reset_avatar_dye_scope", args),
      undo: ({ sessionId }: { sessionId: string }) =>
        invoke<AvatarDyeSessionView>("undo_avatar_dye", { sessionId }),
      redo: ({ sessionId }: { sessionId: string }) =>
        invoke<AvatarDyeSessionView>("redo_avatar_dye", { sessionId }),
      renderComparisonFrame: (args) =>
        invoke<DyeComparisonFrameResponse>("render_avatar_dye_comparison_frame", args),
      renderComparisonAnimation: (args) =>
        invoke<DyeComparisonAnimationResponse>("render_avatar_dye_comparison_animation", args),
      saveToWardrobe: (args: { sessionId: string; draft: unknown; allowDuplicate: boolean }) =>
        invoke<WardrobeStateView>("save_avatar_dye_to_wardrobe", args),
      saveAsActive: (args: { sessionId: string; role: "current" | "target"; clearIncompatible: boolean }) =>
        invoke<WardrobeStateView>("save_avatar_dye_as_active", args),
      saveAsTargetAndOpenSwap: (args: { sessionId: string; clearIncompatible: boolean }) =>
        invoke<WardrobeStateView>("save_avatar_dye_as_target_and_open_swap", args),
      exportNpk: (args: { sessionId: string; outputDirectory: string }) =>
        invoke<ExportNpkResult>("export_avatar_dye_npk", args),
      close: ({ sessionId }: { sessionId: string }) =>
        invoke<void>("close_avatar_dye_session", { sessionId }),
    },
    getGameDir: () => state.gameDir,
    getWorkspace: () => ({
      current: dyeSnapshotSummary(state.savedCurrentOutfit),
      target: dyeSnapshotSummary(state.savedTargetOutfit),
    }),
    getWardrobeGroups: () => state.wardrobe.groups.map(({ id, name }) => ({ id, name })),
    getWardrobeSourceIdentity: (entryId: string) => {
      const entry = state.wardrobe.entries.find((candidate) => candidate.id === entryId);
      if (!entry) return null;
      return {
        snapshotRevision: entry.snapshot.revision,
        recipeRevision: entry.snapshot.recipe_revision,
        professionDir: entry.snapshot.profession_dir,
        pathFamily: entry.snapshot.path_family,
      };
    },
    canUseWardrobe: () => state.wardrobeSwapEntryVisible,
    chooseExportDirectory: async () => {
      const selected = await open({ directory: true, multiple: false, defaultPath: state.gameDir });
      return typeof selected === "string" ? selected : null;
    },
    onWardrobeState: (value: unknown) => {
      applyWardrobeState(value as WardrobeStateView);
      outfitSwapPageCache = null;
      state.outfitSwapPlan = null;
      render();
    },
    onStatus: (message: string) => {
      state.status = message;
      render();
    },
    onReturn: (source: "studio" | "wardrobe") => {
      state.outfitSwapOpen = false;
      state.wardrobeOpen = source === "wardrobe";
      render();
    },
    onEnterSwap: () => enterOutfitSwapPage(),
    closeWindow: () => getCurrentWindow().destroy(),
  };
}

async function openAvatarDyeFromStudio(preferredLayer = "") {
  if (!state.wardrobeSwapEntryVisible) return;
  const snapshot = outfitSnapshotFromStudio();
  if (!snapshot) return;
  stopPlayback();
  await avatarDyePageController.openFromStudio(snapshot, preferredLayer);
}

function dyeSnapshotSummary(snapshot: SavedOutfitView | null): DyeSnapshotSummary | null {
  if (!snapshot) return null;
  return {
    label: wardrobeSnapshotLabel(snapshot),
    occupiedSlotCount: snapshot.items.filter((item) => item.item_name !== null).length,
    hasDye: snapshot.has_dye,
    dyeStatus: snapshot.dye_status,
    dyedSlotCount: snapshot.dyed_slot_count,
    dyedBaseCount: snapshot.dyed_base_count,
    dyedMaskCount: snapshot.dyed_mask_count,
    recipeRevision: snapshot.recipe_revision,
    professionDir: snapshot.profession_dir,
    pathFamily: snapshot.path_family,
  };
}

function unlockHiddenFeature(phrase: string) {
  return hiddenFeatureUnlockController.submit(phrase);
}

function applyVerifiedFeatureConfig(config: AppConfigView) {
  const visibility = verifiedFeatureVisibility(config);
  state.wardrobeSwapEntryVisible = visibility.wardrobeSwapEntryVisible;
}

async function handleHiddenFeatureMatched(result: HiddenFeatureUnlockResult) {
  if (result.feature === "wardrobe_swap") {
    state.status = result.already_unlocked
      ? "该功能已处于激活状态"
      : "时装橱窗与一键换装已激活，下次启动将自动开启";
    render();
    await ensureWardrobeStateLoaded();
  }
  render();
}

function handleHiddenFeatureUnlockError(error: unknown) {
  state.status = `激活状态保存失败，请检查配置目录：${String(error)}`;
  render();
}

function scheduleConfigSave() {
  if (configSaveTimer) window.clearTimeout(configSaveTimer);
  configSaveTimer = window.setTimeout(() => {
    configSaveTimer = 0;
    void saveAppConfig();
  }, 250);
}

async function saveAppConfig() {
  try {
    await invoke<AppConfigView>("save_app_config", {
      config: {
        game_dir: state.gameDir.trim(),
      },
    });
  } catch (error) {
    state.status = `配置保存失败：${String(error)}`;
    render();
  }
}

async function saveOutfitSnapshot(kind: "current" | "target") {
  if (!state.wardrobeSwapEntryVisible) return;
  const snapshot = outfitSnapshotFromStudio();
  if (!snapshot) return;
  const other = kind === "current" ? state.savedTargetOutfit : state.savedCurrentOutfit;
  let clearIncompatible = false;
  if (other && !snapshotsCompatible(snapshot, other)) {
    if (kind === "target") {
      state.status = "目标外观与当前穿戴不是同一职业或资源家族";
      render();
      return;
    }
    if (!window.confirm("新的当前穿戴与已有目标外观不兼容，是否清空目标外观后继续？")) return;
    clearIncompatible = true;
  }
  try {
    applyWardrobeState(await invoke<WardrobeStateView>("save_wardrobe_active", { role: kind, snapshot, clearIncompatible }));
    state.outfitSwapPlan = null;
    state.outfitSwapError = "";
    state.outfitSwapErrorTitle = "";
    state.status = kind === "current" ? "当前穿戴已加密保存" : "目标外观已加密保存";
  } catch (error) {
    state.status = `装扮保存失败：${String(error)}`;
  }
  render();
}

function outfitSnapshotFromStudio(): WardrobeSnapshotInput | null {
  const selected = wardrobeSlots
    .map((slot) => state.outfit[slot])
    .filter((item): item is AvatarItem => Boolean(item))
    .filter((item) => item.resource_kind === "avatar");
  const selectedWeapon = state.outfit.weapon;
  const weapon = selectedWeapon && isStudioWeaponItem(selectedWeapon)
    ? selectedWeapon
    : null;
  if (!selected.length && !weapon) {
    state.status = "当前搭配没有可保存的时装或武器";
    render();
    return null;
  }
  const selectedWithWeapon = weapon ? [...selected, weapon] : selected;
  const effectivePlayableRoleKey = effectivePlayableRoleForItems(
    selectedWithWeapon,
    state.selectedProfession,
  );
  const visualFamily = effectivePlayableRoleKey
    ? visualFamilyForPlayableRole(effectivePlayableRoleKey)
    : null;
  if (!effectivePlayableRoleKey || !visualFamily) {
    state.status = "当前搭配缺少明确的玩法职业或视觉族，不能保存为换装快照";
    render();
    return null;
  }
  return {
    playable_role_key: effectivePlayableRoleKey,
    visual_family_key: visualFamily.visual_family_key,
    resource_owner_key: visualFamily.resource_owner_key,
    resource_generation: visualFamily.resource_generation,
    profession_dir: visualFamily.resource_owner_key,
    path_family: selected[0]?.path_family.trim() || weapon?.path_family.trim() || "weapon",
    items: wardrobeSlots.map((slot) => {
      const item = state.outfit[slot];
      const selectedItem = item?.resource_kind === "avatar" ? item : null;
      return {
        slot,
        item_ref: selectedItem?.item_id ?? null,
        item_name: selectedItem ? cleanDisplayName(selectedItem.item_name) : null,
        layer_count: selectedItem
          ? layerCountForPlayableRole(selectedItem, effectivePlayableRoleKey)
          : 0,
      };
    }),
    weapon: weapon ? {
      slot: "weapon",
      item_ref: weapon.item_id,
      item_name: cleanDisplayName(weapon.item_name),
      layer_count: layerCountForPlayableRole(weapon, effectivePlayableRoleKey),
    } : null,
    preview_data_url: studioPreviewDataUrl(),
  };
}

function studioPreviewDataUrl() {
  const canvas = document.querySelector<HTMLCanvasElement>("#previewCanvas");
  if (!canvas || !canvas.width || !canvas.height) return "";
  try {
    return canvas.toDataURL("image/webp", 0.88);
  } catch {
    return "";
  }
}

function applyWardrobeState(next: WardrobeStateView) {
  state.wardrobe = next;
  state.savedCurrentOutfit = next.current;
  state.savedTargetOutfit = next.target;
  if (state.wardrobeSelectedEntryId && !next.entries.some((entry) => entry.id === state.wardrobeSelectedEntryId)) {
    state.wardrobeSelectedEntryId = next.entries[0]?.id ?? "";
  }
}

function snapshotsCompatible(
  left: Pick<
    WardrobeSnapshotInput | SavedOutfitView,
    "visual_family_key" | "resource_owner_key" | "resource_generation"
  >,
  right: Pick<
    WardrobeSnapshotInput | SavedOutfitView,
    "visual_family_key" | "resource_owner_key" | "resource_generation"
  >,
) {
  return left.visual_family_key.trim().toLowerCase() === right.visual_family_key.trim().toLowerCase()
    && left.resource_owner_key.trim().toLowerCase() === right.resource_owner_key.trim().toLowerCase()
    && left.resource_generation === right.resource_generation;
}

function enterWardrobePage() {
  if (!state.wardrobeSwapEntryVisible) return;
  stopPlayback();
  invalidateCatalogSelection();
  state.outfitSwapOpen = false;
  state.wardrobeOpen = true;
  if (!state.wardrobeSelectedEntryId) state.wardrobeSelectedEntryId = filteredWardrobeEntries()[0]?.id ?? "";
  render();
}

function openCreateWardrobeDialog() {
  if (!state.wardrobeSwapEntryVisible) return;
  const snapshot = outfitSnapshotFromStudio();
  if (!snapshot) return;
  state.wardrobePendingSnapshot = snapshot;
  state.wardrobeEditingEntryId = "";
  state.wardrobeDialogMode = "create";
  const first = slots.map((slot) => state.outfit[slot]).find((item): item is AvatarItem => Boolean(item));
  const defaultName = cleanDisplayName(state.selectedSet?.set_name ?? "")
    || `${first ? professionLabel(first) : playableRoleLabel(snapshot.playable_role_key)} ${new Date().toLocaleDateString()}`;
  renderWardrobeDialog({ name: defaultName, note: "", group_id: selectedWritableGroup(), favorite: false });
}

function openEditWardrobeDialog(entryId: string) {
  const entry = state.wardrobe.entries.find((candidate) => candidate.id === entryId);
  if (!entry) return;
  state.wardrobePendingSnapshot = null;
  state.wardrobeEditingEntryId = entry.id;
  state.wardrobeDialogMode = "edit";
  renderWardrobeDialog(entry);
}

function selectedWritableGroup() {
  return state.wardrobe.groups.some((group) => group.id === state.wardrobeSelectedGroup)
    ? state.wardrobeSelectedGroup
    : null;
}

function renderWardrobeDialog(value?: { name: string; note: string; group_id: string | null; favorite: boolean }) {
  const dialog = document.querySelector<HTMLElement>("#wardrobeDialog");
  if (!dialog) return;
  dialog.hidden = !state.wardrobeDialogMode;
  if (!state.wardrobeDialogMode || !value) return;
  document.querySelector<HTMLElement>("#wardrobeDialogTitle")!.textContent = state.wardrobeDialogMode === "create" ? "保存到时装橱窗" : "编辑橱窗装扮";
  const group = document.querySelector<HTMLSelectElement>("#wardrobeGroup")!;
  group.innerHTML = `<option value="">未分栏</option>${[...state.wardrobe.groups]
    .sort((left, right) => left.sort_order - right.sort_order)
    .map((item) => `<option value="${escapeHtml(item.id)}">${escapeHtml(item.name)}</option>`)
    .join("")}`;
  document.querySelector<HTMLInputElement>("#wardrobeName")!.value = value.name;
  document.querySelector<HTMLTextAreaElement>("#wardrobeNote")!.value = value.note;
  group.value = value.group_id ?? "";
  document.querySelector<HTMLInputElement>("#wardrobeFavorite")!.checked = value.favorite;
}

function closeWardrobeDialog() {
  state.wardrobeDialogMode = null;
  state.wardrobePendingSnapshot = null;
  state.wardrobeEditingEntryId = "";
  const dialog = document.querySelector<HTMLElement>("#wardrobeDialog");
  if (dialog) dialog.hidden = true;
}

async function submitWardrobeDialog() {
  if (!state.wardrobeDialogMode) return;
  const draft = {
    name: document.querySelector<HTMLInputElement>("#wardrobeName")!.value.trim(),
    note: document.querySelector<HTMLTextAreaElement>("#wardrobeNote")!.value.trim(),
    group_id: document.querySelector<HTMLSelectElement>("#wardrobeGroup")!.value || null,
    favorite: document.querySelector<HTMLInputElement>("#wardrobeFavorite")!.checked,
  };
  if (!draft.name) {
    state.status = "请填写装扮名称";
    render();
    return;
  }
  try {
    if (state.wardrobeDialogMode === "create") {
      if (!state.wardrobePendingSnapshot) throw new Error("没有可保存的搭配");
      const duplicate = await invoke<WardrobeEntryView | null>("find_matching_wardrobe_entry", {
        snapshot: state.wardrobePendingSnapshot,
      });
      if (duplicate) {
        const updateExisting = window.confirm(`检测到相同时装与配色“${duplicate.name}”，不能重复保存。确定：更新已有记录；取消：不保存。`);
        if (updateExisting) {
          await invoke("update_wardrobe_entry", { entryId: duplicate.id, draft });
          state.wardrobeSelectedEntryId = duplicate.id;
          state.status = `已更新已有橱窗装扮：${draft.name}`;
          closeWardrobeDialog();
          await refreshWardrobeState();
          return;
        }
        state.status = "已取消保存：橱窗中已有相同时装与配色";
        render();
        return;
      }
      await invoke<WardrobeEntryView>("create_wardrobe_entry", {
        snapshot: state.wardrobePendingSnapshot,
        draft,
        allowDuplicate: false,
      });
      state.status = `已保存到时装橱窗：${draft.name}`;
    } else {
      await invoke("update_wardrobe_entry", { entryId: state.wardrobeEditingEntryId, draft });
      state.status = `已更新橱窗装扮：${draft.name}`;
    }
    closeWardrobeDialog();
    await refreshWardrobeState();
  } catch (error) {
    state.status = `时装橱窗保存失败：${String(error)}`;
    render();
  }
}

async function refreshWardrobeState() {
  const unchecked = await invoke<WardrobeStateView>("load_wardrobe_state");
  applyWardrobeState(unchecked);
  void validateWardrobeStateInBackground(wardrobePublicIdentity(unchecked));
  render();
}

async function createWardrobeGroup() {
  const name = window.prompt("新栏目名称")?.trim();
  if (!name) return;
  try {
    const group = await invoke<WardrobeGroupView>("create_wardrobe_group", { name });
    state.wardrobeSelectedGroup = group.id;
    await refreshWardrobeState();
  } catch (error) {
    state.status = `新建栏目失败：${String(error)}`;
    render();
  }
}

async function shareWardrobeEntry(entryId: string) {
  const entry = state.wardrobe.entries.find((candidate) => candidate.id === entryId);
  if (!entry) return;
  const selected = await save({
    defaultPath: "outfit_share.dnfwardrobe",
    filters: [{ name: "时装橱窗分享", extensions: ["dnfwardrobe"] }],
  });
  if (!selected) return;
  try {
    await invoke<number>("export_wardrobe_entry_share", { entryId, outputPath: selected });
    state.status = `已导出装扮分享：${entry.note || entry.name}`;
  } catch (error) {
    state.status = String(error);
  }
  render();
}

async function shareWardrobeGroup(groupId: string) {
  const group = state.wardrobe.groups.find((candidate) => candidate.id === groupId);
  if (!group) return;
  const selected = await save({
    defaultPath: "wardrobe_group_share.dnfwardrobe",
    filters: [{ name: "时装橱窗分享", extensions: ["dnfwardrobe"] }],
  });
  if (!selected) return;
  try {
    const count = await invoke<number>("export_wardrobe_group_share", { groupId, outputPath: selected });
    state.status = `已导出栏目“${group.name}”：${count} 套装扮`;
  } catch (error) {
    state.status = String(error);
  }
  render();
}

async function importWardrobeShare() {
  const selected = await open({
    multiple: false,
    filters: [{ name: "时装橱窗分享", extensions: ["dnfwardrobe"] }],
  });
  if (typeof selected !== "string") return;
  try {
    const result = await invoke<WardrobeShareImportResult>("import_wardrobe_share", { inputPath: selected });
    applyWardrobeState(result.wardrobe);
    if (result.group_name) {
      state.wardrobeSelectedGroup = result.wardrobe.groups.find((group) => group.name === result.group_name)?.id ?? "all";
    } else {
      state.wardrobeSelectedGroup = "unfiled";
    }
    state.wardrobeSelectedEntryId = "";
    state.status = result.group_name
      ? `已导入栏目“${result.group_name}”：${result.entry_count} 套装扮`
      : `已导入 ${result.entry_count} 套装扮到未分栏`;
  } catch (error) {
    state.status = String(error);
  }
  render();
}

async function renameWardrobeGroup(groupId: string) {
  const group = state.wardrobe.groups.find((candidate) => candidate.id === groupId);
  if (!group) return;
  const name = window.prompt("栏目名称", group.name)?.trim();
  if (!name || name === group.name) return;
  try {
    await invoke("rename_wardrobe_group", { groupId, name });
    await refreshWardrobeState();
  } catch (error) {
    state.status = `重命名栏目失败：${String(error)}`;
    render();
  }
}

async function deleteWardrobeGroup(groupId: string) {
  const group = state.wardrobe.groups.find((candidate) => candidate.id === groupId);
  if (!group) return;
  const count = state.wardrobe.entries.filter((entry) => entry.group_id === groupId).length;
  if (!window.confirm(`删除栏目“${group.name}”？其中 ${count} 套装扮会移动到未分栏。`)) return;
  try {
    await invoke("delete_wardrobe_group", { groupId });
    state.wardrobeSelectedGroup = "unfiled";
    await refreshWardrobeState();
  } catch (error) {
    state.status = `删除栏目失败：${String(error)}`;
    render();
  }
}

async function moveWardrobeGroup(groupId: string, delta: number) {
  const groups = [...state.wardrobe.groups].sort((left, right) => left.sort_order - right.sort_order);
  const index = groups.findIndex((group) => group.id === groupId);
  const target = index + delta;
  if (index < 0 || target < 0 || target >= groups.length) return;
  [groups[index], groups[target]] = [groups[target], groups[index]];
  try {
    await invoke("reorder_wardrobe_groups", { groupIds: groups.map((group) => group.id) });
    await refreshWardrobeState();
  } catch (error) {
    state.status = `栏目排序失败：${String(error)}`;
    render();
  }
}

async function setWardrobeFavorite(entryId: string, favorite: boolean) {
  try {
    await invoke("set_wardrobe_favorite", { entryId, favorite });
    await refreshWardrobeState();
  } catch (error) {
    state.status = `收藏操作失败：${String(error)}`;
    render();
  }
}

async function activateWardrobeEntry(entryId: string, role: "current" | "target"): Promise<boolean> {
  const entry = state.wardrobe.entries.find((candidate) => candidate.id === entryId);
  if (!entry) return false;
  const other = role === "current" ? state.savedTargetOutfit : state.savedCurrentOutfit;
  let clearIncompatible = false;
  if (other && !snapshotsCompatible(entry.snapshot, other)) {
    if (role === "target") {
      state.status = "该装扮与当前穿戴不是同一职业或资源家族";
      render();
      return false;
    }
    if (!window.confirm("该装扮与已有目标外观不兼容，是否清空目标外观后设为当前穿戴？")) return false;
    clearIncompatible = true;
  }
  try {
    applyWardrobeState(await invoke<WardrobeStateView>("activate_wardrobe_entry", { entryId, role, clearIncompatible }));
    state.outfitSwapPlan = null;
    state.outfitSwapError = "";
    state.outfitSwapErrorTitle = "";
    state.status = role === "current" ? `已设为当前穿戴：${entry.name}` : `已设为目标外观：${entry.name}`;
    return true;
  } catch (error) {
    state.status = `指定装扮失败：${String(error)}`;
    return false;
  } finally {
    render();
  }
}

async function activateWardrobeTargetAndOpenSwap(entryId: string) {
  if (await activateWardrobeEntry(entryId, "target")) {
    await enterOutfitSwapPage();
  }
}

async function deleteWardrobeEntry(entryId: string) {
  const entry = state.wardrobe.entries.find((candidate) => candidate.id === entryId);
  if (!entry || !window.confirm(`从时装橱窗删除“${entry.name}”？当前/目标工作区副本不会被清除。`)) return;
  try {
    await invoke("delete_wardrobe_entry", { entryId });
    await refreshWardrobeState();
  } catch (error) {
    state.status = `删除装扮失败：${String(error)}`;
    render();
  }
}

async function replaceWardrobeEntry(entryId: string) {
  const snapshot = outfitSnapshotFromStudio();
  const entry = state.wardrobe.entries.find((candidate) => candidate.id === entryId);
  if (!snapshot || !entry || !window.confirm(`用当前搭配覆盖“${entry.name}”的装扮内容？名称、备注、栏目和收藏状态将保留。`)) return;
  try {
    await invoke("replace_wardrobe_entry_snapshot", { entryId, snapshot });
    state.status = `已用当前搭配覆盖：${entry.name}`;
    await refreshWardrobeState();
  } catch (error) {
    state.status = `覆盖装扮失败：${String(error)}`;
    render();
  }
}

function filteredWardrobeEntries() {
  const query = state.wardrobeQuery.trim().toLowerCase();
  let entries = state.wardrobe.entries.filter((entry) => {
    if (state.wardrobeSelectedGroup === "favorites" && !entry.favorite) return false;
    if (state.wardrobeSelectedGroup === "recent" && entry.last_used_at_ms === null) return false;
    if (state.wardrobeSelectedGroup === "unfiled" && entry.group_id !== null) return false;
    if (!["all", "favorites", "recent", "unfiled"].includes(state.wardrobeSelectedGroup)
      && entry.group_id !== state.wardrobeSelectedGroup) return false;
    if (state.wardrobeProfession !== "all" && wardrobePlayableRoleKey(entry.snapshot) !== state.wardrobeProfession) return false;
    if (!query) return true;
    return entry.name.toLowerCase().includes(query)
      || entry.note.toLowerCase().includes(query)
      || entry.snapshot.items.some((item) => item.item_name?.toLowerCase().includes(query));
  });
  entries = [...entries].sort((left, right) => {
    if (state.wardrobeSort === "name") return left.name.localeCompare(right.name, "zh-CN");
    if (state.wardrobeSort === "created") return right.created_at_ms - left.created_at_ms;
    if (state.wardrobeSort === "recent") return (right.last_used_at_ms ?? 0) - (left.last_used_at_ms ?? 0);
    return right.updated_at_ms - left.updated_at_ms;
  });
  return entries;
}

function wardrobePlayableRoleKey(snapshot: Pick<SavedOutfitView, "playable_role_key">) {
  return snapshot.playable_role_key.trim().toLowerCase();
}

function wardrobeSnapshotLabel(snapshot: SavedOutfitView | null) {
  if (!snapshot) return "未保存";
  const occupied = snapshot.items.filter((item) => item.item_name !== null).length;
  const dye = snapshot.has_dye
    ? ` · 染色 ${snapshot.dyed_slot_count} 部位（主 IMG ${snapshot.dyed_base_count} / Mask ${snapshot.dyed_mask_count}）`
    : " · 原色";
  return `${playableRoleLabel(wardrobePlayableRoleKey(snapshot))} · ${occupied} 个部位${dye}`;
}

async function enterOutfitSwapPage() {
  if (!state.wardrobeSwapEntryVisible) return;
  const requestId = ++outfitSwapLoadRequestId;
  stopPlayback();
  invalidateCatalogSelection();
  state.wardrobeOpen = false;
  state.outfitSwapOpen = true;
  state.outfitSwapError = "";
  state.outfitSwapErrorTitle = "";
  state.outfitSwapPreviewErrors = { current: "", target: "" };
  state.outfitSwapProgress = null;
  const current = state.savedCurrentOutfit;
  const target = state.savedTargetOutfit;
  if (!current || !target) {
    state.outfitSwapLoading = false;
    state.outfitSwapPlan = null;
    render();
    clearOutfitSwapCanvases();
    void Promise.allSettled([
      ...(current ? [drawAvailableOutfitPreview("current", current, requestId)] : []),
      ...(target ? [drawAvailableOutfitPreview("target", target, requestId)] : []),
    ]);
    return;
  }
  const cacheKey = outfitSwapCacheKey();
  if (outfitSwapPageCache?.key === cacheKey) {
    state.outfitSwapLoading = false;
    clearOutfitSwapCanvases();
    drawOutfitSwapPageResult(outfitSwapPageCache.result);
    void Promise.allSettled([
      ...(outfitSwapPageCache.result.current_preview_available === false
        ? [drawAvailableOutfitPreview("current", current, requestId)]
        : []),
      ...(outfitSwapPageCache.result.target_preview_available === false
        ? [drawAvailableOutfitPreview("target", target, requestId)]
        : []),
    ]);
    return;
  }
  state.outfitSwapLoading = true;
  state.outfitSwapPlan = null;
  render();
  clearOutfitSwapCanvases();
  void Promise.allSettled([
    drawAvailableOutfitPreview("current", current, requestId),
    drawAvailableOutfitPreview("target", target, requestId),
  ]);
  try {
    const result = await invoke<OutfitSwapPageResult>("load_active_outfit_swap_page", {
      gameDir: state.gameDir,
    });
    if (requestId !== outfitSwapLoadRequestId || cacheKey !== outfitSwapCacheKey() || !state.outfitSwapOpen) return;
    outfitSwapPageCache = { key: cacheKey, result };
    drawOutfitSwapPageResult(result);
  } catch (error) {
    if (requestId !== outfitSwapLoadRequestId || cacheKey !== outfitSwapCacheKey() || !state.outfitSwapOpen) return;
    state.outfitSwapError = String(error);
    state.outfitSwapErrorTitle = "换装分析失败";
    state.outfitSwapPreviewErrors = {
      current: "",
      target: "",
    };
  } finally {
    if (requestId === outfitSwapLoadRequestId) {
      state.outfitSwapLoading = false;
      render();
    }
  }
}

function outfitSwapCacheKey() {
  const currentSavedAt = state.savedCurrentOutfit?.saved_at_ms ?? 0;
  const targetSavedAt = state.savedTargetOutfit?.saved_at_ms ?? 0;
  const currentRecipe = state.savedCurrentOutfit?.recipe_revision ?? "original";
  const targetRecipe = state.savedTargetOutfit?.recipe_revision ?? "original";
  const gameDir = state.gameDir.trim().replaceAll("\\", "/").toLowerCase();
  return `${gameDir}|${currentSavedAt}|${currentRecipe}|${targetSavedAt}|${targetRecipe}`;
}

function drawOutfitSwapPageResult(result: OutfitSwapPageResult) {
  state.outfitSwapPlan = result.plan;
  state.outfitSwapError = "";
  render();
  if (result.current_preview_available !== false) {
    drawOutfitSnapshotPreview({
      canvasSelector: "#swapCurrentPreviewCanvas",
      image: result.current_preview,
    });
  }
  if (result.target_preview_available !== false) {
    drawOutfitSnapshotPreview({
      canvasSelector: "#swapPreviewCanvas",
      image: result.target_preview,
    });
  }
}

async function drawAvailableOutfitPreview(
  kind: "current" | "target",
  snapshot: SavedOutfitView,
  requestId: number,
) {
  const canvasSelector = kind === "current" ? "#swapCurrentPreviewCanvas" : "#swapPreviewCanvas";
  const linearDodge = kind === "target" && state.outfitSwapLinearDodgeEnabled;
  const requestedGameDir = state.gameDir;
  const requestedGameDirKey = normalizedPreviewGameDir(requestedGameDir);
  const previewTicket = outfitSwapPreviewGate.issue(kind, requestId, linearDodge);
  const isCurrentPreview = () => state.outfitSwapOpen && outfitSwapPreviewGate.accepts(
    previewTicket,
    outfitSwapLoadRequestId,
    kind === "target" && state.outfitSwapLinearDodgeEnabled,
  ) && normalizedPreviewGameDir() === requestedGameDirKey;
  const requiresProfessionFrame = previewStartFrameForProfession(snapshot.playable_role_key) > 0;
  const savedPreviewDrawn = linearDodge || requiresProfessionFrame
    ? false
    : await drawSavedOutfitPreview(snapshot, canvasSelector, isCurrentPreview).catch(() => false);
  if (!isCurrentPreview()) return;
  if (savedPreviewDrawn) return;
  try {
    const image = await invoke<PreviewImage>("render_active_outfit", {
      role: kind,
      gameDir: requestedGameDir,
      linearDodge,
    });
    if (!isCurrentPreview()) return;
    drawOutfitSnapshotPreview({ canvasSelector, image });
  } catch (error) {
    if (!isCurrentPreview()) return;
    state.outfitSwapPreviewErrors = {
      ...state.outfitSwapPreviewErrors,
      [kind]: String(error),
    };
    render();
  }
}

function refreshOutfitSwapTargetPreview() {
  const target = state.savedTargetOutfit;
  if (!state.outfitSwapOpen || !target) return;
  state.outfitSwapPreviewErrors = {
    ...state.outfitSwapPreviewErrors,
    target: "",
  };
  const canvas = document.querySelector<HTMLCanvasElement>("#swapPreviewCanvas");
  canvas?.getContext("2d")?.clearRect(0, 0, canvas.width, canvas.height);
  void drawAvailableOutfitPreview("target", target, outfitSwapLoadRequestId);
}

async function drawSavedOutfitPreview(
  snapshot: SavedOutfitView,
  canvasSelector: string,
  isCurrentPreview: () => boolean,
): Promise<boolean> {
  if (!snapshot.preview_data_url) return false;
  const image = await loadWardrobePreviewImage(snapshot.preview_data_url);
  if (!isCurrentPreview()) return false;
  const canvas = document.querySelector<HTMLCanvasElement>(canvasSelector);
  if (!canvas) return false;
  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  const context = canvas.getContext("2d");
  context?.clearRect(0, 0, canvas.width, canvas.height);
  context?.drawImage(image, 0, 0);
  return true;
}

function clearOutfitSwapCanvases() {
  for (const selector of ["#swapCurrentPreviewCanvas", "#swapPreviewCanvas"]) {
    const canvas = document.querySelector<HTMLCanvasElement>(selector);
    canvas?.getContext("2d")?.clearRect(0, 0, canvas.width, canvas.height);
  }
}

function drawOutfitSnapshotPreview(result: { canvasSelector: string; image: PreviewImage }) {
  const { canvasSelector, image } = result;
  const canvas = document.querySelector<HTMLCanvasElement>(canvasSelector);
  if (!canvas) return;
  const imageData = toImageData(image);
  canvas.width = image.width;
  canvas.height = image.height;
  const context = canvas.getContext("2d");
  context?.clearRect(0, 0, canvas.width, canvas.height);
  context?.putImageData(imageData, 0, 0);
}

async function exportWeaponSwap() {
  const current = state.savedCurrentOutfit;
  const target = state.savedTargetOutfit;
  if (!current?.weapon || !target?.weapon || state.outfitSwapExporting) return;
  const selected = await save({
    defaultPath: "weapon_swap_patch.npk",
    filters: [{ name: "NPK", extensions: ["npk"] }],
  });
  if (!selected) return;
  state.outfitSwapExporting = true;
  state.outfitSwapProgress = { percent: 0, stage: "准备生成武器换装补丁" };
  state.outfitSwapError = "";
  state.outfitSwapErrorTitle = "";
  render();
  try {
    const onProgress = new Channel<OutfitSwapProgress>();
    onProgress.onmessage = (progress) => {
      state.outfitSwapProgress = {
        percent: Math.max(0, Math.min(100, Math.round(progress.percent))),
        stage: progress.stage || "正在生成武器换装补丁",
      };
      updateOutfitSwapProgress();
    };
    const result = await invoke<OutfitSwapExportResult>("export_active_weapon_swap_npk", {
      gameDir: state.gameDir,
      outputPath: selected,
      linearDodge: state.outfitSwapLinearDodgeEnabled,
      onProgress,
    });
    const effectLabel = state.outfitSwapLinearDodgeEnabled ? "（线性减淡已应用）" : "";
    state.outfitSwapProgress = { percent: 100, stage: `武器换装补丁生成完成${effectLabel}` };
    state.status = `武器换装补丁已生成${effectLabel}：${result.entry_count} 个 IMG`;
  } catch (error) {
    state.outfitSwapError = String(error);
    state.outfitSwapErrorTitle = "武器换装补丁生成失败";
    state.outfitSwapProgress = {
      percent: state.outfitSwapProgress?.percent ?? 0,
      stage: "武器换装补丁生成失败",
    };
  } finally {
    state.outfitSwapExporting = false;
    render();
  }
}

async function exportOutfitSwap() {
  const current = state.savedCurrentOutfit;
  const target = state.savedTargetOutfit;
  if (!current || !target || !state.outfitSwapPlan) return;
  const selected = await save({
    defaultPath: "outfit_swap_patch.npk",
    filters: [{ name: "NPK", extensions: ["npk"] }],
  });
  if (!selected) return;
  state.outfitSwapExporting = true;
  state.outfitSwapProgress = { percent: 0, stage: "准备生成换装补丁" };
  state.outfitSwapError = "";
  state.outfitSwapErrorTitle = "";
  render();
  try {
    const onProgress = new Channel<OutfitSwapProgress>();
    onProgress.onmessage = (progress) => {
      state.outfitSwapProgress = {
        percent: Math.max(0, Math.min(100, Math.round(progress.percent))),
        stage: progress.stage || "正在生成换装补丁",
      };
      updateOutfitSwapProgress();
    };
    const result = await invoke<OutfitSwapExportResult>("export_active_outfit_swap_npk", {
      gameDir: state.gameDir,
      outputPath: selected,
      linearDodge: state.outfitSwapLinearDodgeEnabled,
      onProgress,
    });
    const effectLabel = state.outfitSwapLinearDodgeEnabled ? "（线性减淡已应用）" : "";
    state.outfitSwapProgress = { percent: 100, stage: `换装补丁生成完成${effectLabel}` };
    state.status = `换装补丁已生成${effectLabel}：${result.part_count} 个部位，${result.entry_count} 个 IMG`;
  } catch (error) {
    state.outfitSwapError = String(error);
    state.outfitSwapErrorTitle = "换装补丁生成失败";
    state.outfitSwapProgress = {
      percent: state.outfitSwapProgress?.percent ?? 0,
      stage: "换装补丁生成失败",
    };
  } finally {
    state.outfitSwapExporting = false;
    render();
  }
}

function renderOutfitSwapPage() {
  const shell = document.querySelector<HTMLElement>(".shell");
  const studio = document.querySelector<HTMLElement>("#studioWorkspace");
  const page = document.querySelector<HTMLElement>("#outfitSwapPage");
  if (!shell || !studio || !page) return;
  shell.classList.toggle("swap-view", state.outfitSwapOpen);
  studio.hidden = state.outfitSwapOpen || state.wardrobeOpen;
  page.hidden = !state.outfitSwapOpen;
  const preprocessTools = document.querySelector<HTMLElement>("#swapPreprocessTools")!;
  const linearDodge = document.querySelector<HTMLButtonElement>("#swapLinearDodge")!;
  if (!state.outfitSwapOpen) {
    preprocessTools.hidden = true;
    return;
  }
  preprocessTools.hidden = false;
  linearDodge.classList.toggle("active", state.outfitSwapLinearDodgeEnabled);
  linearDodge.setAttribute("aria-pressed", String(state.outfitSwapLinearDodgeEnabled));
  linearDodge.textContent = state.outfitSwapLinearDodgeEnabled ? "线性减淡 · 已开启" : "线性减淡";
  linearDodge.disabled = state.outfitSwapExporting;

  document.querySelector<HTMLElement>("#swapCurrentEyebrow")!.textContent = "当前";
  document.querySelector<HTMLElement>("#swapTargetEyebrow")!.textContent = "目标";
  document.querySelector<HTMLElement>("#swapPlanEyebrow")!.textContent = "方案";
  const previewErrors = state.outfitSwapPreviewErrors;
  const currentPreviewError = document.querySelector<HTMLElement>("#swapCurrentPreviewError")!;
  const targetPreviewError = document.querySelector<HTMLElement>("#swapTargetPreviewError")!;
  currentPreviewError.hidden = !previewErrors.current;
  currentPreviewError.textContent = previewErrors.current ? `当前穿戴预览失败：${previewErrors.current}` : "";
  targetPreviewError.hidden = !previewErrors.target;
  targetPreviewError.textContent = previewErrors.target ? `目标外观预览失败：${previewErrors.target}` : "";

  renderSwapSnapshot("current", state.savedCurrentOutfit);
  renderSwapSnapshot("target", state.savedTargetOutfit);
  const badge = document.querySelector<HTMLElement>("#swapPlanBadge")!;
  const summary = document.querySelector<HTMLElement>("#swapPlanSummary")!;
  const plans = document.querySelector<HTMLElement>("#swapPartPlans")!;
  const weaponSwap = document.querySelector<HTMLButtonElement>("#generateWeaponSwap")!;
  const generate = document.querySelector<HTMLButtonElement>("#generateOutfitSwap")!;
  const singleCarrier = document.querySelector<HTMLButtonElement>("#generateSingleCarrierSwap")!;
  updateOutfitSwapProgress();
  const standardStrategyBlocked = ["requires_single_carrier_mode", "blocked", "noop"].includes(
    state.outfitSwapPlan?.strategy ?? "",
  );
  const unsupported = state.outfitSwapPlan?.parts.some(
    (part) => part.action === "unsupported" || part.action === "blocked" || part.action === "requires_single_carrier",
  ) ?? false;
  const actionable = state.outfitSwapPlan?.parts.some(
    (part) => part.action === "replace" || part.action === "insert" || part.action === "clear" || part.action === "recolor"
  ) ?? false;
  generate.disabled = !state.outfitSwapPlan || standardStrategyBlocked || unsupported || !actionable || state.outfitSwapLoading || state.outfitSwapExporting;
  generate.textContent = state.outfitSwapExporting ? "正在生成补丁" : "生成换装补丁";
  weaponSwap.disabled = !state.savedCurrentOutfit?.weapon || !state.savedTargetOutfit?.weapon || state.outfitSwapLoading || state.outfitSwapExporting;
  weaponSwap.textContent = state.outfitSwapExporting ? "正在生成补丁" : "武器换装";
  singleCarrier.disabled = !state.savedCurrentOutfit || !state.savedTargetOutfit || state.outfitSwapLoading || state.outfitSwapExporting;
  singleCarrier.textContent = state.outfitSwapExporting ? "正在生成补丁" : "单时装完整替代";

  if (!state.savedCurrentOutfit || !state.savedTargetOutfit) {
    badge.textContent = "缺少快照";
    summary.innerHTML = `<strong>请先在套装浏览或自由搭配页保存两份搭配</strong><span>需要“当前穿戴”和“目标外观”后才能生成换装补丁。</span>`;
    plans.innerHTML = "";
    return;
  }
  if (state.outfitSwapLoading) {
    badge.textContent = "自动分析中";
    summary.innerHTML = "<strong>正在分析换装方案</strong><span>检查当前穿戴与目标外观是否可以安全生成补丁。</span>";
    plans.innerHTML = "";
    return;
  }
  if (state.outfitSwapError) {
    const exportFailed = state.outfitSwapErrorTitle.endsWith("生成失败");
    badge.textContent = exportFailed ? "生成失败" : "不可生成";
    summary.innerHTML = `<strong>${escapeHtml(state.outfitSwapErrorTitle || "换装分析失败")}</strong><span>${escapeHtml(state.outfitSwapError)}</span>`;
    plans.innerHTML = "";
    return;
  }
  const plan = state.outfitSwapPlan;
  if (!plan) {
    badge.textContent = "等待分析";
    summary.innerHTML = "";
    plans.innerHTML = "";
    return;
  }
  const actionLabels: Record<string, string> = {
    replace: "替换",
    recolor: "染色替换",
    insert: "新增",
    clear: "移除",
    keep: "保持",
    lend: "参与替换",
    requires_single_carrier: "需特殊生成",
    unsupported: "无法新增",
    blocked: "染色失效",
  };
  const actionCounts = {
    replace: plan.parts.filter((part) => part.action === "replace").length,
    recolor: plan.parts.filter((part) => part.action === "recolor").length,
    insert: plan.parts.filter((part) => part.action === "insert").length,
    clear: plan.parts.filter((part) => part.action === "clear").length,
    keep: plan.parts.filter((part) => part.action === "keep").length,
    lend: plan.parts.filter((part) => part.action === "lend").length,
    requiresSingleCarrier: plan.parts.filter((part) => part.action === "requires_single_carrier").length,
    unsupported: plan.parts.filter((part) => part.action === "unsupported").length,
    blocked: plan.parts.filter((part) => part.action === "blocked").length,
  };
  const blockingPresentation = plan.strategy === "requires_single_carrier_mode"
      ? {
          badge: "需特殊生成",
          detail: "标准生成无法安全完成整套替换，请主动使用“单时装完整替代”",
      }
    : plan.strategy === "noop"
      ? {
          badge: "无需生成",
          detail: "当前穿戴与目标外观一致",
        }
      : actionCounts.blocked && actionCounts.unsupported
        ? {
            badge: "染色失效且无法新增",
            detail: "存在失效染色配方，并且目标外观无法完整替换，不能生成补丁",
          }
        : actionCounts.blocked
      ? {
          badge: "存在染色失效",
          detail: "存在失效染色配方，不能生成完整补丁",
        }
      : actionCounts.unsupported
        ? {
            badge: "存在无法新增部位",
            detail: "当前组合无法安全新增目标部位，不能生成完整补丁",
          }
        : {
            badge: "分析通过",
            detail: "目标外观已按当前穿戴完成换装规划",
          };
  badge.textContent = blockingPresentation.badge;
  const planSummary = `替换 ${actionCounts.replace} · 染色替换 ${actionCounts.recolor} · 新增 ${actionCounts.insert} · 参与替换 ${actionCounts.lend} · 移除 ${actionCounts.clear} · 保持 ${actionCounts.keep} · 需特殊生成 ${actionCounts.requiresSingleCarrier} · 无法新增 ${actionCounts.unsupported} · 染色失效 ${actionCounts.blocked}`;
  summary.innerHTML = `<strong>${planSummary}</strong><span>${blockingPresentation.detail}</span>`;
  const partPlansMarkup = plan.parts.map((part) => {
    const partDyeDetail = part.has_dye || part.action === "recolor"
      ? " · 已应用染色"
      : "";
    return `
      <article class="swap-part-plan action-${escapeHtml(part.action)}">
        <div><strong>${escapeHtml(slotLabels[part.slot] ?? part.slot)}</strong><span>${escapeHtml(
          part.action === "clear"
            ? `移除当前${part.current_item_name}`
            : part.action === "keep"
              ? `${part.target_item_name || part.current_item_name}（无需变化）`
              : part.action === "insert"
                ? `新增${part.target_item_name}`
              : part.action === "lend"
                ? `${part.current_item_name}参与目标外观替换`
              : part.action === "requires_single_carrier"
                ? `标准生成无法安全完成整套替换，请使用“单时装完整替代”`
              : part.action === "unsupported"
                ? `当前组合无法安全新增${part.target_item_name}`
              : part.action === "blocked"
                ? `${part.target_item_name || part.current_item_name} 的染色配方失效，已阻止生成`
              : part.action === "recolor"
                ? `${part.target_item_name || part.current_item_name} · 将目标染色固化到补丁`
                : `${part.current_item_name} → ${part.target_item_name}`
        )}${partDyeDetail}</span></div>
        <em>${escapeHtml(actionLabels[part.action] ?? part.action)}</em>
      </article>
    `;
  }).join("");
  plans.innerHTML = partPlansMarkup;
}

async function openSingleCarrierDialog() {
  if (!state.savedCurrentOutfit || !state.savedTargetOutfit || state.outfitSwapExporting) return;
  const requestId = ++singleCarrierRequestId;
  state.singleCarrierDialogOpen = true;
  state.singleCarrierLoading = true;
  state.singleCarrierError = "";
  state.singleCarrierCandidates = [];
  state.singleCarrierSlot = "";
  state.singleCarrierPlan = null;
  render();
  try {
    const candidates = await invoke<ProfessionWideCarrierCandidate[]>("list_active_profession_wide_carriers");
    if (requestId !== singleCarrierRequestId || !state.singleCarrierDialogOpen) return;
    state.singleCarrierCandidates = candidates;
    state.singleCarrierSlot = candidates[0]?.slot ?? "";
    if (!candidates.length) state.singleCarrierError = "当前穿戴没有仍会被游戏绘制的安全载体";
  } catch (error) {
    if (requestId !== singleCarrierRequestId || !state.singleCarrierDialogOpen) return;
    state.singleCarrierError = String(error);
  } finally {
    if (requestId === singleCarrierRequestId) {
      state.singleCarrierLoading = false;
      render();
    }
  }
}

function closeSingleCarrierDialog() {
  if (state.outfitSwapExporting) return;
  singleCarrierRequestId += 1;
  state.singleCarrierDialogOpen = false;
  state.singleCarrierLoading = false;
  state.singleCarrierError = "";
  state.singleCarrierCandidates = [];
  state.singleCarrierSlot = "";
  state.singleCarrierPlan = null;
  render();
}

async function previewSingleCarrierScope() {
  const carrierSlot = state.singleCarrierSlot;
  if (!carrierSlot || state.singleCarrierLoading || state.outfitSwapExporting) return;
  const requestId = ++singleCarrierRequestId;
  state.singleCarrierLoading = true;
  state.singleCarrierPlan = null;
  state.singleCarrierError = "";
  renderSingleCarrierDialog();
  try {
    const plan = await invoke<ProfessionWideSwapPlan>("preview_active_profession_wide_single_carrier", {
      gameDir: state.gameDir,
      carrierSlot,
      linearDodge: state.outfitSwapLinearDodgeEnabled,
    });
    if (
      requestId !== singleCarrierRequestId
      || !state.singleCarrierDialogOpen
      || carrierSlot !== state.singleCarrierSlot
    ) return;
    state.singleCarrierPlan = plan;
  } catch (error) {
    if (requestId !== singleCarrierRequestId || !state.singleCarrierDialogOpen) return;
    state.singleCarrierError = String(error);
  } finally {
    if (requestId === singleCarrierRequestId) {
      state.singleCarrierLoading = false;
      renderSingleCarrierDialog();
    }
  }
}

async function exportSingleCarrierSwap() {
  const plan = state.singleCarrierPlan;
  const carrierSlot = state.singleCarrierSlot;
  if (!plan || !carrierSlot || state.outfitSwapExporting) return;
  const selected = await save({
    defaultPath: "outfit_swap_single_carrier_patch.npk",
    filters: [{ name: "NPK", extensions: ["npk"] }],
  });
  if (!selected) return;
  state.singleCarrierDialogOpen = false;
  state.outfitSwapExporting = true;
  state.outfitSwapProgress = { percent: 0, stage: "准备生成单时装特殊补丁" };
  state.singleCarrierError = "";
  state.outfitSwapError = "";
  state.outfitSwapErrorTitle = "";
  render();
  try {
    const onProgress = new Channel<OutfitSwapProgress>();
    onProgress.onmessage = (progress) => {
      state.outfitSwapProgress = {
        percent: Math.max(0, Math.min(100, Math.round(progress.percent))),
        stage: progress.stage || "正在生成单时装特殊补丁",
      };
      updateOutfitSwapProgress();
    };
    const result = await invoke<OutfitSwapExportResult>("export_active_profession_wide_single_carrier_npk", {
      gameDir: state.gameDir,
      outputPath: selected,
      carrierSlot,
      confirmationToken: plan.confirmation_token,
      linearDodge: state.outfitSwapLinearDodgeEnabled,
      onProgress,
    });
    const effectLabel = state.outfitSwapLinearDodgeEnabled ? "（线性减淡已应用）" : "";
    state.outfitSwapProgress = { percent: 100, stage: `单时装特殊补丁生成完成${effectLabel}` };
    state.status = `单时装特殊补丁已生成${effectLabel}：覆盖 ${result.entry_count.toLocaleString()} 个 IMG`;
  } catch (error) {
    state.outfitSwapError = String(error);
    state.outfitSwapErrorTitle = "特殊补丁生成失败";
    state.outfitSwapProgress = {
      percent: state.outfitSwapProgress?.percent ?? 0,
      stage: "单时装特殊补丁生成失败",
    };
  } finally {
    state.outfitSwapExporting = false;
    render();
  }
}

function renderSingleCarrierDialog() {
  const dialog = document.querySelector<HTMLElement>("#singleCarrierDialog");
  if (!dialog) return;
  dialog.hidden = !state.singleCarrierDialogOpen;
  if (!state.singleCarrierDialogOpen) return;

  const select = document.querySelector<HTMLSelectElement>("#singleCarrierSelect")!;
  const analyze = document.querySelector<HTMLButtonElement>("#singleCarrierAnalyze")!;
  const scope = document.querySelector<HTMLElement>("#singleCarrierScope")!;
  const exportButton = document.querySelector<HTMLButtonElement>("#singleCarrierExport")!;
  const error = document.querySelector<HTMLElement>("#singleCarrierError")!;
  const close = document.querySelector<HTMLButtonElement>("#singleCarrierClose")!;
  const cancel = document.querySelector<HTMLButtonElement>("#singleCarrierCancel")!;
  const plan = state.singleCarrierPlan;

  select.innerHTML = state.singleCarrierCandidates
    .map((candidate) => `<option value="${escapeHtml(candidate.slot)}">${escapeHtml(slotLabels[candidate.slot] ?? candidate.slot)} · ${escapeHtml(candidate.item_name)}</option>`)
    .join("");
  select.value = state.singleCarrierSlot;
  select.disabled = state.singleCarrierLoading || state.outfitSwapExporting || !state.singleCarrierCandidates.length;
  analyze.disabled = state.singleCarrierLoading || state.outfitSwapExporting || !state.singleCarrierSlot;
  analyze.textContent = state.singleCarrierLoading ? "正在分析全部 NPK" : "分析全部 NPK 影响范围";

  if (plan) {
    scope.innerHTML = `
      <strong>职业全量覆盖 · ${escapeHtml(plan.affected_scope)}</strong>
      <span>资源根：${escapeHtml(plan.avatar_root)}</span>
      <span>唯一载体：${escapeHtml(slotLabels[plan.carrier_slot] ?? plan.carrier_slot)} · ${escapeHtml(plan.carrier_item_name)}</span>
      <span>NPK ${plan.source_npk_count.toLocaleString()} 个 · IMG ${plan.total_img_count.toLocaleString()} 个 · 透明化 ${plan.suppressed_img_count.toLocaleString()} 个</span>
      <span>预计补丁 ${(plan.estimated_output_bytes / 1024 / 1024).toFixed(1)} MiB</span>
      <em>安装期间，该职业其他时装、目标来源、默认皮肤、Mask 和变体都会不可见。</em>
    `;
  } else {
    scope.innerHTML = state.singleCarrierLoading
      ? "<span>正在逐个读取该职业的全部 Avatar NPK 与 IMG 元数据……</span>"
      : "<span>选择载体后先分析影响范围。只有完整读取全部职业时装 NPK 才能继续。</span>";
  }
  exportButton.disabled = !plan || state.singleCarrierLoading || state.outfitSwapExporting;
  exportButton.textContent = state.outfitSwapExporting ? "正在生成特殊补丁" : "生成特殊补丁";
  close.disabled = state.outfitSwapExporting;
  cancel.disabled = state.outfitSwapExporting;
  error.hidden = !state.singleCarrierError;
  error.textContent = state.singleCarrierError;
}

function updateOutfitSwapProgress() {
  const progress = state.outfitSwapProgress;
  const root = document.querySelector<HTMLElement>("#swapExportProgress");
  const stage = document.querySelector<HTMLElement>("#swapExportProgressStage");
  const percent = document.querySelector<HTMLElement>("#swapExportProgressPercent");
  const fill = document.querySelector<HTMLElement>("#swapExportProgressFill");
  if (!root || !stage || !percent || !fill) return;
  root.hidden = !progress;
  if (!progress) return;
  const value = Math.max(0, Math.min(100, Math.round(progress.percent)));
  root.setAttribute("aria-valuenow", String(value));
  stage.textContent = progress.stage;
  percent.textContent = `${value}%`;
  fill.style.width = `${value}%`;
}

function wardrobeRepresentativeColorsHtml(colors: readonly string[], variant: "card" | "detail") {
  const safeColors = colors
    .map((color) => color.trim())
    .filter((color) => /^#[0-9a-f]{6}$/i.test(color))
    .slice(0, 6);
  if (!safeColors.length) return "";
  return `<span class="wardrobe-representative-colors ${variant}" title="染色代表色">${safeColors
    .map((color) => `<i style="--wardrobe-color:${color}" aria-hidden="true"></i>`)
    .join("")}</span>`;
}

function renderWardrobePage() {
  const shell = document.querySelector<HTMLElement>(".shell");
  const studio = document.querySelector<HTMLElement>("#studioWorkspace");
  const page = document.querySelector<HTMLElement>("#wardrobePage");
  if (!shell || !studio || !page) return;
  shell.classList.toggle("wardrobe-view", state.wardrobeOpen);
  studio.hidden = state.wardrobeOpen || state.outfitSwapOpen;
  page.hidden = !state.wardrobeOpen;
  if (!state.wardrobeOpen) return;
  wardrobeExactPreviewScheduler.activateContext(normalizedPreviewGameDir());

  const profession = document.querySelector<HTMLSelectElement>("#wardrobeProfession")!;
  const professionOptions = playableRoleOptions();
  if (!professionOptions.some(([key]) => key === state.wardrobeProfession)) {
    state.wardrobeProfession = "all";
  }
  profession.innerHTML = professionOptions
    .map(([key, label]) => `<option value="${key}">${escapeHtml(label)}</option>`)
    .join("");
  profession.value = state.wardrobeProfession;
  document.querySelector<HTMLInputElement>("#wardrobeQuery")!.value = state.wardrobeQuery;
  document.querySelector<HTMLSelectElement>("#wardrobeSort")!.value = state.wardrobeSort;
  const shareGroupButton = document.querySelector<HTMLButtonElement>("#wardrobeShareGroup")!;
  const shareGroupId = selectedWritableGroup();
  shareGroupButton.disabled = !shareGroupId;
  shareGroupButton.title = shareGroupId ? "分享当前自定义栏目" : "只能分享自定义栏目";

  const systemGroups = [
    ["all", "全部", state.wardrobe.entries.length],
    ["favorites", "收藏", state.wardrobe.entries.filter((entry) => entry.favorite).length],
    ["recent", "最近使用", state.wardrobe.entries.filter((entry) => entry.last_used_at_ms !== null).length],
    ["unfiled", "未分栏", state.wardrobe.entries.filter((entry) => entry.group_id === null).length],
  ] as const;
  const groups = document.querySelector<HTMLElement>("#wardrobeGroups")!;
  groups.innerHTML = systemGroups.map(([id, name, count]) => `
    <button class="wardrobe-group${state.wardrobeSelectedGroup === id ? " active" : ""}" data-wardrobe-group="${id}"><span>${name}</span><em>${count}</em></button>
  `).join("") + `<div class="wardrobe-group-separator"><span>我的栏目</span></div>` + [...state.wardrobe.groups]
    .sort((left, right) => left.sort_order - right.sort_order)
    .map((group, index, ordered) => `
      <div class="wardrobe-group-row${state.wardrobeSelectedGroup === group.id ? " active" : ""}">
        <button class="wardrobe-group" data-wardrobe-group="${escapeHtml(group.id)}"><span>${escapeHtml(group.name)}</span><em>${state.wardrobe.entries.filter((entry) => entry.group_id === group.id).length}</em></button>
        <span class="wardrobe-group-actions">
          <button data-group-up="${escapeHtml(group.id)}" title="上移"${index === 0 ? " disabled" : ""}>↑</button>
          <button data-group-down="${escapeHtml(group.id)}" title="下移"${index === ordered.length - 1 ? " disabled" : ""}>↓</button>
          <button data-group-rename="${escapeHtml(group.id)}" title="重命名">✎</button>
          <button data-group-delete="${escapeHtml(group.id)}" title="删除">×</button>
        </span>
      </div>
    `).join("");

  groups.querySelectorAll<HTMLButtonElement>("[data-wardrobe-group]").forEach((button) => {
    button.addEventListener("click", () => {
      state.wardrobeSelectedGroup = button.dataset.wardrobeGroup ?? "all";
      state.wardrobeSelectedEntryId = "";
      renderWardrobePage();
    });
  });
  groups.querySelectorAll<HTMLButtonElement>("[data-group-up]").forEach((button) => button.addEventListener("click", () => void moveWardrobeGroup(button.dataset.groupUp ?? "", -1)));
  groups.querySelectorAll<HTMLButtonElement>("[data-group-down]").forEach((button) => button.addEventListener("click", () => void moveWardrobeGroup(button.dataset.groupDown ?? "", 1)));
  groups.querySelectorAll<HTMLButtonElement>("[data-group-rename]").forEach((button) => button.addEventListener("click", () => void renameWardrobeGroup(button.dataset.groupRename ?? "")));
  groups.querySelectorAll<HTMLButtonElement>("[data-group-delete]").forEach((button) => button.addEventListener("click", () => void deleteWardrobeGroup(button.dataset.groupDelete ?? "")));

  const entries = filteredWardrobeEntries();
  if (!entries.some((entry) => entry.id === state.wardrobeSelectedEntryId)) {
    state.wardrobeSelectedEntryId = entries[0]?.id ?? "";
  }
  const cards = document.querySelector<HTMLElement>("#wardrobeCards")!;
  cards.innerHTML = entries.map((entry) => {
    const selected = entry.id === state.wardrobeSelectedEntryId ? " active" : "";
    const resourceChanged = entry.snapshot.resource_status === "changed";
    const resourceUnchecked = entry.snapshot.resource_status === "unchecked";
    const dyeState = dyeWorkspaceStatusPresentation(entry.snapshot.dye_status, entry.snapshot.has_dye);
    const resourceBlocked = resourceChanged || resourceUnchecked || dyeState.persistenceBlocked;
    const hasPreview = Boolean(entry.snapshot.preview_data_url)
      || previewStartFrameForProfession(entry.snapshot.playable_role_key) > 0;
    const image = hasPreview
      ? `<canvas class="wardrobe-card-canvas" data-wardrobe-preview-id="${escapeHtml(entry.id)}" data-wardrobe-preview-kind="card" role="img" aria-label="${escapeHtml(entry.name)}"></canvas>`
      : `<span class="wardrobe-card-placeholder">无预览</span>`;
    const dyeBadge = entry.snapshot.has_dye
      ? `<span class="wardrobe-dye-badge ${dyeState.tone}" title="${entry.snapshot.dyed_base_count} 个主 IMG、${entry.snapshot.dyed_mask_count} 个 Mask 已染色">${dyeState.label} · ${entry.snapshot.dyed_slot_count} 部位</span>`
      : "";
    const representativeColors = wardrobeRepresentativeColorsHtml(entry.snapshot.representative_colors, "card");
    return `
      <article class="wardrobe-card${selected}${resourceBlocked ? " resource-changed" : ""}" data-wardrobe-entry="${escapeHtml(entry.id)}">
        <button class="wardrobe-card-preview" data-wardrobe-select="${escapeHtml(entry.id)}">${image}</button>
        ${dyeBadge}
        ${representativeColors}
        <button class="wardrobe-card-star${entry.favorite ? " active" : ""}" data-wardrobe-favorite="${escapeHtml(entry.id)}" aria-label="${entry.favorite ? "取消收藏" : "收藏"}">★</button>
        <button class="wardrobe-card-copy" data-wardrobe-select="${escapeHtml(entry.id)}"><strong class="wardrobe-card-note">${entry.note ? escapeHtml(entry.note) : "暂无备注"}</strong><span class="wardrobe-card-meta">${escapeHtml(entry.name)}</span><em>${resourceChanged ? "资源已变化 · 禁止使用" : resourceUnchecked ? "暂时无法校验资源 · 禁止使用" : dyeState.persistenceBlocked ? escapeHtml(wardrobeDyeBlockMessage(entry.snapshot.dye_status)) : escapeHtml(wardrobeSnapshotLabel(entry.snapshot))}</em></button>
      </article>
    `;
  }).join("") || `<div class="wardrobe-empty"><strong>当前栏目没有装扮</strong><span>请在套装或单件页面完成搭配后，点击“存入橱窗”。</span></div>`;
  cards.querySelectorAll<HTMLButtonElement>("[data-wardrobe-select]").forEach((button) => {
    button.addEventListener("click", () => {
      state.wardrobeSelectedEntryId = button.dataset.wardrobeSelect ?? "";
      renderWardrobePage();
    });
  });
  cards.querySelectorAll<HTMLButtonElement>("[data-wardrobe-favorite]").forEach((button) => {
    button.addEventListener("click", () => {
      const entry = state.wardrobe.entries.find((candidate) => candidate.id === button.dataset.wardrobeFavorite);
      if (entry) void setWardrobeFavorite(entry.id, !entry.favorite);
    });
  });

  const entry = state.wardrobe.entries.find((candidate) => candidate.id === state.wardrobeSelectedEntryId) ?? null;
  renderWardrobeDetail(entry);
  refreshWardrobePreviewCanvases(page);
}

function renderWardrobeDetail(entry: WardrobeEntryView | null) {
  const detail = document.querySelector<HTMLElement>("#wardrobeDetail")!;
  if (!entry) {
    detail.innerHTML = `
      <div class="wardrobe-workspace-summary"><span>当前穿戴</span><strong>${escapeHtml(wardrobeSnapshotLabel(state.savedCurrentOutfit))}</strong><span>目标外观</span><strong>${escapeHtml(wardrobeSnapshotLabel(state.savedTargetOutfit))}</strong></div>
      <div class="wardrobe-empty"><strong>请选择一套装扮</strong><span>在这里可以指定当前穿戴或目标外观。</span></div>
    `;
    return;
  }
  const groupName = state.wardrobe.groups.find((group) => group.id === entry.group_id)?.name ?? "未分栏";
  const current = entry.is_current;
  const target = entry.is_target;
  const resourceChanged = entry.snapshot.resource_status === "changed";
  const resourceUnchecked = entry.snapshot.resource_status === "unchecked";
  const dyeState = dyeWorkspaceStatusPresentation(entry.snapshot.dye_status, entry.snapshot.has_dye);
  const resourceBlocked = resourceChanged || resourceUnchecked || dyeState.persistenceBlocked;
  const currentDisabled = resourceBlocked ? " disabled" : "";
  const targetDisabled = resourceBlocked || (state.savedCurrentOutfit && !snapshotsCompatible(state.savedCurrentOutfit, entry.snapshot)) ? " disabled" : "";
  const dyeSummary = entry.snapshot.has_dye
    ? `<div class="wardrobe-dye-summary"><span class="wardrobe-dye-badge ${dyeState.tone}">${dyeState.label}</span><strong>${entry.snapshot.dyed_slot_count} 个部位</strong><span>主 IMG ${entry.snapshot.dyed_base_count} · Mask ${entry.snapshot.dyed_mask_count}</span><em>配方 ${escapeHtml(entry.snapshot.recipe_revision?.slice(0, 8) ?? "待校验")}</em>${wardrobeRepresentativeColorsHtml(entry.snapshot.representative_colors, "detail")}</div>`
    : `<div class="wardrobe-dye-summary original"><strong>原色装扮</strong><span>可加载到染色工坊创建独立版本</span></div>`;
  const resourceNotice = resourceChanged
    ? `<div class="wardrobe-resource-warning"><strong>资源已变化，不能指定或生成补丁</strong><span>失效部位：${entry.snapshot.invalid_slots.map((slot) => escapeHtml(slotLabels[slot] ?? slot)).join("、")}</span></div>`
    : resourceUnchecked
      ? `<div class="wardrobe-resource-warning"><strong>当前无法校验内置资源，暂时禁止使用</strong><span>重新载入数据后再指定为当前穿戴或目标外观。</span></div>`
      : dyeState.persistenceBlocked
        ? `<div class="wardrobe-resource-warning"><strong>${escapeHtml(wardrobeDyeBlockMessage(entry.snapshot.dye_status))}</strong><span>记录和加密缩略图会继续保留；可加载到染色工坊检查、修复或恢复原色。</span></div>`
        : "";
  detail.innerHTML = `
    <div class="wardrobe-workspace-summary"><span>当前穿戴</span><strong>${escapeHtml(wardrobeSnapshotLabel(state.savedCurrentOutfit))}</strong><span>目标外观</span><strong>${escapeHtml(wardrobeSnapshotLabel(state.savedTargetOutfit))}</strong><button id="wardrobeGoSwap" class="mini-btn">前往一键换装</button></div>
    <div class="wardrobe-detail-preview">${entry.snapshot.preview_data_url || previewStartFrameForProfession(entry.snapshot.playable_role_key) > 0 ? `<canvas id="wardrobeDetailCanvas" class="wardrobe-detail-canvas" data-wardrobe-preview-id="${escapeHtml(entry.id)}" data-wardrobe-preview-kind="detail" role="img" aria-label="${escapeHtml(entry.name)}"></canvas>` : `<span>无预览</span>`}</div>
    <div class="wardrobe-detail-head"><div class="wardrobe-detail-copy"><strong>${entry.note ? escapeHtml(entry.note) : "暂无备注"}</strong><span>${escapeHtml(entry.name)}</span><em>${escapeHtml(groupName)} · ${escapeHtml(wardrobeSnapshotLabel(entry.snapshot))}</em></div><button id="wardrobeDetailFavorite" class="wardrobe-detail-star${entry.favorite ? " active" : ""}">★</button></div>
    ${dyeSummary}
    ${resourceNotice}
    <div class="wardrobe-active-actions">
      <button id="wardrobeOpenDye" class="primary-btn">加载到染色工坊</button>
      <button class="primary-btn" data-wardrobe-role="current"${currentDisabled}>${current ? "已是当前穿戴" : "设为当前穿戴"}</button>
      <button class="primary-btn" data-wardrobe-role="target"${targetDisabled}>${target ? "已是目标外观" : "设为目标外观"}</button>
      <button id="wardrobeTargetAndSwap" class="primary-btn"${targetDisabled}>设为目标并前往一键换装</button>
    </div>
    <div class="wardrobe-detail-items">${entry.snapshot.items.map((item) => `<div><span>${escapeHtml(slotLabels[item.slot] ?? item.slot)}</span><strong>${item.item_name ? escapeHtml(item.item_name) : "未穿戴"}</strong><em>${item.item_name ? `${item.layer_count} layer` : "空"}</em></div>`).join("")}</div>
    <div class="wardrobe-detail-actions"><button id="wardrobeShareEntry" class="icon-text-btn">分享此装扮</button><button id="wardrobeEditEntry" class="icon-text-btn">修改名称/备注</button><button id="wardrobeReplaceEntry" class="icon-text-btn">用当前搭配覆盖</button><button id="wardrobeDeleteEntry" class="icon-text-btn danger">删除</button></div>
  `;
  detail.querySelector<HTMLButtonElement>("#wardrobeGoSwap")?.addEventListener("click", () => void enterOutfitSwapPage());
  detail.querySelector<HTMLButtonElement>("#wardrobeOpenDye")?.addEventListener("click", () => void avatarDyePageController.openFromWardrobe(entry.id));
  detail.querySelector<HTMLButtonElement>("#wardrobeTargetAndSwap")?.addEventListener("click", () => void activateWardrobeTargetAndOpenSwap(entry.id));
  detail.querySelector<HTMLButtonElement>("#wardrobeDetailFavorite")?.addEventListener("click", () => void setWardrobeFavorite(entry.id, !entry.favorite));
  detail.querySelector<HTMLButtonElement>("#wardrobeShareEntry")?.addEventListener("click", () => void shareWardrobeEntry(entry.id));
  detail.querySelectorAll<HTMLButtonElement>("[data-wardrobe-role]").forEach((button) => button.addEventListener("click", () => void activateWardrobeEntry(entry.id, button.dataset.wardrobeRole as "current" | "target")));
  detail.querySelector<HTMLButtonElement>("#wardrobeEditEntry")?.addEventListener("click", () => openEditWardrobeDialog(entry.id));
  detail.querySelector<HTMLButtonElement>("#wardrobeReplaceEntry")?.addEventListener("click", () => void replaceWardrobeEntry(entry.id));
  detail.querySelector<HTMLButtonElement>("#wardrobeDeleteEntry")?.addEventListener("click", () => void deleteWardrobeEntry(entry.id));
}

function wardrobeDyeBlockMessage(status: string) {
  switch (status.trim().toLowerCase()) {
    case "unsupported":
      return "染色配方不支持，不能指定或生成补丁";
    case "partial":
      return "染色配方部分失效，不能生成不完整结果";
    case "changed":
      return "染色配方引用的资源已变化，不能指定或生成补丁";
    default:
      return "染色配方尚未校验，暂时不能指定或生成补丁";
  }
}

function refreshWardrobePreviewCanvases(root: ParentNode) {
  wardrobePreviewResizeObserver.disconnect();
  wardrobePreviewVisibilityObserver.disconnect();
  root.querySelectorAll<HTMLCanvasElement>("[data-wardrobe-preview-id]").forEach((canvas) => {
    if (canvas.dataset.wardrobePreviewKind === "detail") {
      wardrobePreviewVisibleCanvases.add(canvas);
      wardrobePreviewResizeObserver.observe(canvas);
    } else {
      wardrobePreviewVisibilityObserver.observe(canvas);
    }
  });
}

async function drawWardrobePreviewCanvas(canvas: HTMLCanvasElement) {
  const entry = state.wardrobe.entries.find((candidate) => candidate.id === canvas.dataset.wardrobePreviewId);
  if (!entry) return;
  const professionStartFrame = previewStartFrameForProfession(entry.snapshot.playable_role_key);
  if (!entry.snapshot.preview_data_url && professionStartFrame === 0) return;
  const kind = canvas.dataset.wardrobePreviewKind === "detail" ? "detail" : "card";
  const width = Math.max(1, Math.round(canvas.clientWidth));
  const height = Math.max(1, Math.round(canvas.clientHeight));
  const requestedGameDir = state.gameDir;
  const requestedGameDirKey = normalizedPreviewGameDir(requestedGameDir);
  const requestKey = `${entry.id}:${entry.updated_at_ms}:${professionStartFrame}:${requestedGameDirKey}:${kind}:${width}x${height}`;
  if (
    canvas.dataset.wardrobePreviewRendered === requestKey
    || canvas.dataset.wardrobePreviewPendingKey === requestKey
  ) return;
  const requestToken = `${requestKey}:${++wardrobePreviewRequestSequence}`;
  canvas.dataset.wardrobePreviewPending = requestToken;
  canvas.dataset.wardrobePreviewPendingKey = requestKey;
  canvas.dataset.wardrobePreviewRequest = requestToken;
  try {
    const image = await loadWardrobePreviewSource(
      entry,
      entry.snapshot.playable_role_key,
      requestedGameDir,
      requestedGameDirKey,
    );
    if (
      !canvas.isConnected
      || canvas.dataset.wardrobePreviewRequest !== requestToken
      || normalizedPreviewGameDir() !== requestedGameDirKey
    ) return;
    const sourceWidth = wardrobePreviewSourceWidth(image);
    const sourceHeight = wardrobePreviewSourceHeight(image);
    const boundsKey = `${entry.id}:${entry.updated_at_ms}:${professionStartFrame}:${requestedGameDirKey}`;
    const bounds = wardrobePreviewBoundsCache.has(boundsKey)
      ? wardrobePreviewBoundsCache.get(boundsKey) ?? null
      : measureWardrobePreviewBounds(image, sourceWidth, sourceHeight, boundsKey);
    if (!bounds) {
      canvas.dataset.wardrobePreviewRendered = requestKey;
      return;
    }
    const placement = centeredPreviewPlacement(bounds, width, height, kind === "detail"
      ? { padding: 12, maxWidth: 300, maxHeight: 340 }
      : { padding: 16, maxWidth: Number.POSITIVE_INFINITY, maxHeight: Number.POSITIVE_INFINITY });
    const pixelRatio = Math.min(Math.max(window.devicePixelRatio || 1, 1), 2);
    canvas.width = Math.round(width * pixelRatio);
    canvas.height = Math.round(height * pixelRatio);
    const context = canvas.getContext("2d");
    if (!context) return;
    context.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
    context.clearRect(0, 0, width, height);
    context.imageSmoothingEnabled = false;
    context.drawImage(
      image,
      placement.sourceX,
      placement.sourceY,
      placement.sourceWidth,
      placement.sourceHeight,
      placement.x,
      placement.y,
      placement.width,
      placement.height,
    );
    canvas.dataset.wardrobePreviewRendered = requestKey;
  } catch {
    if (canvas.dataset.wardrobePreviewRequest === requestToken) {
      canvas.title = "预览图读取失败";
    }
  } finally {
    if (canvas.dataset.wardrobePreviewPending === requestToken) {
      delete canvas.dataset.wardrobePreviewPending;
      delete canvas.dataset.wardrobePreviewPendingKey;
    }
  }
}

function loadWardrobePreviewImage(source: string) {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    image.addEventListener("load", () => resolve(image), { once: true });
    image.addEventListener("error", () => reject(new Error("preview image failed to load")), { once: true });
    image.src = source;
  });
}

async function loadWardrobePreviewSource(
  entry: WardrobeEntryView,
  playableRoleKey: string,
  requestedGameDir: string,
  requestedGameDirKey: string,
): Promise<HTMLImageElement | HTMLCanvasElement> {
  if (previewStartFrameForProfession(playableRoleKey) === 0) {
    return loadWardrobePreviewImage(entry.snapshot.preview_data_url);
  }
  const cacheKey = `${entry.id}:${entry.updated_at_ms}:${requestedGameDirKey}`;
  let pending = wardrobeExactPreviewCache.get(cacheKey);
  if (!pending) {
    const scheduled = wardrobeExactPreviewScheduler.schedule(
      cacheKey,
      () => invoke<PreviewImage>("render_wardrobe_entry_preview", {
        entryId: entry.id,
        gameDir: requestedGameDir,
      }).then((image) => {
        const source = document.createElement("canvas");
        source.width = Math.max(1, image.width);
        source.height = Math.max(1, image.height);
        const context = source.getContext("2d");
        if (!context) throw new Error("wardrobe preview canvas is unavailable");
        context.putImageData(toImageData(image), 0, 0);
        return source;
      }),
      requestedGameDirKey,
    );
    if (!scheduled) throw new Error("wardrobe preview request was superseded");
    pending = scheduled.catch((error) => {
      wardrobeExactPreviewCache.delete(cacheKey);
      throw error;
    });
    wardrobeExactPreviewCache.set(cacheKey, pending);
    while (wardrobeExactPreviewCache.size > 64) {
      const oldestKey = wardrobeExactPreviewCache.keys().next().value;
      if (oldestKey === undefined) break;
      wardrobeExactPreviewCache.delete(oldestKey);
    }
  }
  return pending;
}

function normalizedPreviewGameDir(gameDir = state.gameDir) {
  return gameDir.trim().replaceAll("\\", "/").toLowerCase();
}

function invalidateWardrobePreviewGameDir() {
  wardrobeExactPreviewScheduler.activateContext(normalizedPreviewGameDir());
  wardrobeExactPreviewCache.clear();
  wardrobePreviewBoundsCache.clear();
}

function setGameDirectory(gameDir: string) {
  const previousGameDirKey = normalizedPreviewGameDir();
  state.gameDir = gameDir;
  if (normalizedPreviewGameDir() === previousGameDirKey) return;
  invalidateWardrobePreviewGameDir();
  outfitSwapPageCache = null;
  outfitSwapLoadRequestId += 1;
  if (state.wardrobeOpen) render();
  if (state.outfitSwapOpen) void enterOutfitSwapPage();
}

function wardrobePreviewSourceWidth(source: HTMLImageElement | HTMLCanvasElement) {
  return source instanceof HTMLImageElement ? source.naturalWidth : source.width;
}

function wardrobePreviewSourceHeight(source: HTMLImageElement | HTMLCanvasElement) {
  return source instanceof HTMLImageElement ? source.naturalHeight : source.height;
}

function measureWardrobePreviewBounds(
  image: HTMLImageElement | HTMLCanvasElement,
  width: number,
  height: number,
  cacheKey: string,
) {
  const source = document.createElement("canvas");
  source.width = width;
  source.height = height;
  const context = source.getContext("2d", { willReadFrequently: true });
  if (!context) return null;
  context.drawImage(image, 0, 0);
  const bounds = opaquePixelBounds(
    context.getImageData(0, 0, source.width, source.height).data,
    source.width,
    source.height,
  );
  wardrobePreviewBoundsCache.set(cacheKey, bounds);
  return bounds;
}

function renderSwapSnapshot(
  kind: "current" | "target",
  snapshot: SavedOutfitView | null,
) {
  const itemNode = document.querySelector<HTMLElement>(kind === "current" ? "#swapCurrentItems" : "#swapTargetItems")!;
  const savedNode = document.querySelector<HTMLElement>(kind === "current" ? "#swapCurrentSavedAt" : "#swapTargetSavedAt")!;
  if (!snapshot) {
    savedNode.textContent = "未保存";
    itemNode.innerHTML = `<div class="swap-empty">请在原搭配页面保存${kind === "current" ? "当前穿戴" : "目标外观"}</div>`;
    return;
  }
  const dyeState = dyeWorkspaceStatusPresentation(snapshot.dye_status, snapshot.has_dye);
  savedNode.innerHTML = `${escapeHtml(formatSnapshotTime(snapshot.saved_at_ms))}${snapshot.has_dye ? ` <span class="wardrobe-dye-badge ${dyeState.tone}">${dyeState.label} · ${snapshot.dyed_slot_count} 部位</span>` : ""}`;
  const outfitItems = snapshot.items.map((item) => {
    return `
      <article class="swap-saved-item">
        <span>${escapeHtml(slotLabels[item.slot] ?? item.slot)}</span>
        <strong>${item.item_name ? escapeHtml(item.item_name) : "未穿戴"}</strong>
      </article>
    `;
  }).join("");
  const weaponItem = snapshot.weapon
    ? `<article class="swap-saved-item weapon"><span>武器</span><strong>${escapeHtml(snapshot.weapon.item_name)}</strong></article>`
    : `<article class="swap-saved-item weapon empty"><span>武器</span><strong>未穿戴</strong></article>`;
  itemNode.innerHTML = `${outfitItems}${weaponItem}`;
}

function formatSnapshotTime(value: number) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "已保存" : `保存于 ${date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
}

function issueCatalogSelection() {
  stopPlayback();
  state.rendering = false;
  catalogPreviewGate.invalidate();
  return catalogSelectionGate.issue();
}

function invalidateCatalogSelection() {
  catalogSelectionGate.invalidate();
  catalogPreviewGate.invalidate();
  state.rendering = false;
}

function issueCatalogPreview(isSelectionCurrent: () => boolean = () => true) {
  const previewTicket = catalogPreviewGate.issue();
  return () => isSelectionCurrent() && catalogPreviewGate.accepts(previewTicket);
}

function finishCatalogSelection(selectionTicket: number) {
  if (catalogSelectionGate.finish(selectionTicket)) {
    render();
  }
}

function isCatalogSelectionPending() {
  return catalogSelectionGate.isPending();
}

function setMode(mode: Mode) {
  stopPlayback();
  resetAnimationCache();
  invalidateCatalogSelection();
  state.outfitSwapOpen = false;
  state.wardrobeOpen = false;
  state.mode = mode;
  state.selectedSlot = "all";
  state.selectedSet = null;
  state.selectedSetItems = [];
  state.selectedItem = null;
  state.activeLayers = [];
  state.activePlayableRoleKey = "";
  state.frame = previewStartFrameForProfession(state.selectedProfession);
  state.missingImgs = [];
  render();
  clearCanvas();
  refreshCatalogQuery(false);
}

async function openCatalogIndex() {
  stopPlayback();
  invalidateCatalogSelection();
  catalogQueryController.invalidate();
  beginGalleryRefreshGeneration();
  disposeActiveGallerySnapshot();
  state.loading = true;
  state.catalogQueryLoading = false;
  state.catalogQueryStarted = false;
  state.catalogQueryError = "";
  state.resultItems = [];
  state.resultSets = [];
  state.status = "正在打开内置数据索引";
  setGameDirectory(gameDirInput.value.trim());
  render();
  try {
    state.catalogMeta = await invoke<CatalogMetadataResponse>("open_avatar_catalog_index");
    state.status = `索引已就绪：${state.catalogMeta.stats.set_count.toLocaleString()} 个套装，${state.catalogMeta.stats.item_count.toLocaleString()} 个单件`;
  } catch (error) {
    state.catalogMeta = null;
    state.status = String(error);
  } finally {
    state.loading = false;
    render();
    if (state.catalogMeta && state.workspaceMode === "gallery") void refreshGallery();
  }
}

async function executeCatalogQuery(
  request: CatalogQueryRequest,
): Promise<CatalogQueryResult> {
  if (request.mode === "items") {
    const items = await invoke<AvatarItem[]>("query_avatar_items", {
      slot: request.slot,
      playableRole: request.playableRole,
      query: request.query,
    });
    return { mode: "items", items };
  }
  const sets = await invoke<AvatarSet[]>("query_avatar_sets", {
    playableRole: request.playableRole,
    query: request.query,
  });
  return { mode: "sets", sets };
}

function refreshCatalogQuery(debounced: boolean) {
  if (!state.catalogMeta) {
    state.status = "请先载入内置数据";
    render();
    return;
  }
  if (state.workspaceMode === "gallery") {
    window.clearTimeout(gallerySearchTimer);
    if (debounced) {
      state.galleryLoading = true;
      state.status = "等待图库搜索输入";
      render();
      gallerySearchTimer = window.setTimeout(() => void refreshGallery(), 220);
    } else {
      void refreshGallery();
    }
    return;
  }
  const request: CatalogQueryRequest = {
    mode: state.mode,
    slot: state.selectedSlot,
    playableRole: state.selectedProfession,
    query: state.query,
  };
  state.catalogQueryStarted = true;
  state.catalogQueryLoading = true;
  state.catalogQueryError = "";
  state.status = debounced ? "等待搜索输入" : "正在查询内置数据";
  render();
  if (debounced) {
    catalogQueryController.debounced(request);
  } else {
    void catalogQueryController.immediate(request);
  }
}

function setWorkspaceMode(workspaceMode: WorkspaceMode) {
  if (state.workspaceMode === workspaceMode) return;
  state.workspaceMode = workspaceMode;
  galleryRequestGate.invalidate();
  disposeActiveGallerySnapshot();
  window.clearTimeout(gallerySearchTimer);
  state.galleryLoading = false;
  if (workspaceMode === "gallery") {
    state.galleryCards = [];
    state.galleryTotal = 0;
    state.galleryResolved = 0;
    state.galleryNextOffset = null;
    state.galleryPage = 1;
    state.galleryStarted = false;
    state.galleryError = "";
    render();
    if (state.catalogMeta) void refreshGallery();
  } else {
    render();
    if (state.catalogMeta) refreshCatalogQuery(false);
  }
}

function gallerySelectionMessage() {
  if (state.selectedProfession === "all") return "请先在左侧选择具体职业";
  if (state.mode === "items") {
    const gate = gallerySingleGate({
      mode: state.mode,
      profession: state.selectedProfession,
      slot: state.selectedSlot,
    });
    if (!gate.allowed) return "单件图库需要先选择职业，并在上方搭配槽位中选择一个具体部位";
  }
  return "";
}

type GalleryCatalogSnapshot = {
  gameDir: string;
  kind: "set" | "item";
  playableRole: string;
  slot: string;
  query: string;
  selectedEntityId: string;
};

type GalleryLoadState = {
  cards: GalleryCard[];
  total: number;
  resolved: number;
  nextOffset: number | null;
};

function galleryCatalogContextKey() {
  return JSON.stringify([
    state.workspaceMode,
    state.gameDir,
    state.mode,
    state.selectedProfession,
    state.selectedSlot,
    state.query,
  ]);
}

function galleryCatalogSnapshot(): GalleryCatalogSnapshot {
  return Object.freeze({
    gameDir: state.gameDir,
    kind: state.mode === "sets" ? "set" : "item",
    playableRole: state.selectedProfession,
    slot: state.mode === "items" ? state.selectedSlot : "all",
    query: state.query,
    selectedEntityId: selectedGalleryEntityId(),
  });
}

function beginGalleryRefreshGeneration() {
  galleryRefreshGeneration += 1;
  galleryRequestGate.invalidate();
  galleryThumbnailScheduler.activateContext(galleryContextKey());
}

function invalidateGalleryPresentation() {
  beginGalleryRefreshGeneration();
  disposeActiveGallerySnapshot();
  state.galleryCards = [];
  state.galleryTotal = 0;
  state.galleryResolved = 0;
  state.galleryNextOffset = null;
  state.galleryPage = 1;
  state.galleryError = "";
}

function disposeActiveGallerySnapshot() {
  disposeGalleryMemorySnapshot(state.gallerySnapshot, (objectUrl) => URL.revokeObjectURL(objectUrl));
  state.gallerySnapshot = null;
  state.gallerySnapshotPhase = "idle";
  state.gallerySnapshotDecoded = 0;
  state.gallerySnapshotTotal = 0;
  gallerySnapshotPageElements = [];
  gallerySnapshotVisiblePageIndex = -1;
  const grid = document.querySelector<HTMLDivElement>("#galleryGrid");
  if (grid) {
    delete grid.dataset.gallerySnapshotContext;
    delete grid.dataset.gallerySnapshotVisiblePage;
    grid.replaceChildren();
  }
}

function galleryWebpObjectUrl(base64: string) {
  const binary = window.atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return URL.createObjectURL(new Blob([bytes], { type: "image/webp" }));
}

async function decodeGalleryObjectUrl(objectUrl: string) {
  const image = new Image();
  image.decoding = "async";
  image.src = objectUrl;
  await image.decode();
}

async function loadPersistentGallerySnapshot(
  snapshot: GalleryCatalogSnapshot,
  ticket: number,
  context: string,
) {
  const response = await invoke<CatalogGallerySnapshotResponse>(
    "load_catalog_gallery_snapshot",
    {
      request: {
        game_dir: snapshot.gameDir,
        kind: snapshot.kind,
        playable_role: snapshot.playableRole,
        slot: snapshot.slot,
        query: snapshot.query,
      },
    },
  );
  const isCurrent = () => (
    galleryRequestGate.accepts(ticket)
    && context === galleryCatalogContextKey()
  );
  if (!isCurrent() || !response.available) {
    if (isCurrent()) state.gallerySnapshotPhase = "idle";
    return false;
  }

  const merged = mergeGalleryCards(
    [],
    response.cards.map((card) => ({
      ...card,
      thumbnail_webp_base64: card.thumbnail_webp_base64 ?? "",
      error: card.error ?? "",
    })),
    {
      selectedEntityId: snapshot.selectedEntityId,
      query: snapshot.query,
    },
  );
  state.gallerySnapshotPhase = "decoding";
  state.gallerySnapshotTotal = merged.length;
  renderGallery();
  const warmed = await warmGallerySnapshotCards(merged, {
    concurrency: 8,
    createObjectUrl: galleryWebpObjectUrl,
    decodeObjectUrl: decodeGalleryObjectUrl,
    revokeObjectUrl: (objectUrl) => URL.revokeObjectURL(objectUrl),
    isCurrent,
    onProgress(completed, total) {
      if (!isCurrent()) return;
      state.gallerySnapshotDecoded = completed;
      state.gallerySnapshotTotal = total;
      if (completed === total || completed % 8 === 0) renderGalleryBatchStatus();
    },
  });
  if (!isCurrent()) return false;

  const memorySnapshot = createGalleryMemorySnapshot(
    `${context}:${response.snapshot_id}`,
    warmed,
    SET_GALLERY_PAGE_SIZE,
    galleryCardHtml,
  );
  state.gallerySnapshot = memorySnapshot;
  state.galleryCards = [...memorySnapshot.cards];
  state.galleryTotal = response.catalog_total;
  state.galleryResolved = response.catalog_total;
  state.galleryNextOffset = null;
  state.galleryPage = 1;
  state.gallerySnapshotDecoded = memorySnapshot.cards.length;
  state.gallerySnapshotTotal = memorySnapshot.cards.length;
  state.gallerySnapshotPhase = "ready";
  state.status = `全量图库已就绪：${response.named_entries.toLocaleString()} 个命名条目，${memorySnapshot.cards.length.toLocaleString()} 个唯一预览`;
  if (state.galleryBatch?.phase === "completed") {
    window.clearTimeout(galleryBatchPollTimer);
    state.galleryBatch = null;
  }
  return true;
}

async function loadGalleryCardsUntil(
  snapshot: GalleryCatalogSnapshot,
  initial: GalleryLoadState,
  targetUniqueCards: number,
  isCurrent: () => boolean,
): Promise<GalleryLoadState | null> {
  let cards = initial.cards;
  let total = initial.total;
  let resolved = initial.resolved;
  let nextOffset = initial.nextOffset;

  while (nextOffset !== null && cards.length < targetUniqueCards) {
    const offset = nextOffset;
    const page = await invoke<CatalogGalleryPage>("query_catalog_gallery_page", {
      request: {
        game_dir: snapshot.gameDir,
        kind: snapshot.kind,
        playable_role: snapshot.playableRole,
        slot: snapshot.slot,
        query: snapshot.query,
        offset,
        limit: 64,
      },
    });
    if (!isCurrent()) return null;
    const incoming = page.cards.map((card) => ({
      ...card,
      thumbnail_webp_base64: card.thumbnail_webp_base64 ?? "",
      error: card.error ?? "",
    }));
    cards = mergeGalleryCards(cards, incoming, {
      selectedEntityId: snapshot.selectedEntityId,
      query: snapshot.query,
    });
    total = page.total;
    resolved = Math.min(page.total, page.next_offset ?? page.total);
    nextOffset = page.next_offset;
    if (nextOffset === offset) break;
  }

  return { cards, total, resolved, nextOffset };
}

async function continueGalleryFallbackScan(
  snapshot: GalleryCatalogSnapshot,
  initial: GalleryLoadState,
  ticket: number,
  context: string,
) {
  const isCurrent = () => (
    galleryRequestGate.accepts(ticket)
    && context === galleryCatalogContextKey()
  );
  let loaded = initial;
  while (isCurrent() && !state.gallerySnapshot && loaded.nextOffset !== null) {
    const next = await loadGalleryCardsUntil(
      snapshot,
      loaded,
      loaded.cards.length + SET_GALLERY_PAGE_SIZE,
      isCurrent,
    );
    if (!next || !isCurrent() || state.gallerySnapshot) return;
    loaded = {
      ...next,
      cards: mergeGalleryCards(state.galleryCards, next.cards, {
        selectedEntityId: snapshot.selectedEntityId,
        query: snapshot.query,
      }),
    };
    state.galleryCards = loaded.cards;
    state.galleryTotal = loaded.total;
    state.galleryResolved = loaded.resolved;
    state.galleryNextOffset = loaded.nextOffset;
    state.status = loaded.nextOffset === null
      ? `图库目录后台整理完成：${loaded.resolved.toLocaleString()} / ${loaded.total.toLocaleString()} 项，整理出 ${loaded.cards.length.toLocaleString()} 个唯一预览`
      : `图库正在后台整理：${loaded.resolved.toLocaleString()} / ${loaded.total.toLocaleString()} 项，已整理 ${loaded.cards.length.toLocaleString()} 个唯一预览`;
    renderGallery();
    const status = document.querySelector("#status");
    if (status) status.textContent = state.status;
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
  }
}

function scheduleGalleryBackgroundHydration(
  snapshot: GalleryCatalogSnapshot,
  initial: GalleryLoadState,
  ticket: number,
  context: string,
) {
  window.setTimeout(() => {
    const isCurrent = () => (
      galleryRequestGate.accepts(ticket)
      && context === galleryCatalogContextKey()
    );
    if (!isCurrent()) return;
    void continueGalleryFallbackScan(snapshot, initial, ticket, context)
      .catch((error) => {
        if (!isCurrent() || state.gallerySnapshot) return;
        state.status = `图库后台整理已停止：${String(error)}`;
        const status = document.querySelector("#status");
        if (status) status.textContent = state.status;
      });
    void loadPersistentGallerySnapshot(snapshot, ticket, context)
      .then((loaded) => {
        if (!loaded || !isCurrent()) return;
        renderGallery();
        const status = document.querySelector("#status");
        if (status) status.textContent = state.status;
      })
      .catch(() => {
        if (!isCurrent()) return;
        state.gallerySnapshotPhase = "idle";
        renderGalleryBatchStatus();
      });
  }, 0);
}

async function refreshGallery() {
  const singleGate = gallerySingleGate({
    mode: state.mode,
    profession: state.selectedProfession,
    slot: state.selectedSlot,
  });
  const blocked = state.mode === "items"
    ? !singleGate.allowed
    : state.selectedProfession === "all";
  beginGalleryRefreshGeneration();
  disposeActiveGallerySnapshot();
  if (!state.catalogMeta || blocked) {
    state.galleryCards = [];
    state.galleryTotal = 0;
    state.galleryResolved = 0;
    state.galleryNextOffset = null;
    state.galleryPage = 1;
    state.galleryStarted = false;
    state.galleryLoading = false;
    state.galleryError = "";
    renderGallery();
    return;
  }
  const context = galleryCatalogContextKey();
  const snapshot = galleryCatalogSnapshot();
  const ticket = galleryRequestGate.issue();
  state.galleryCards = [];
  state.galleryTotal = 0;
  state.galleryResolved = 0;
  state.galleryNextOffset = null;
  state.galleryPage = 1;
  state.galleryStarted = true;
  state.galleryLoading = true;
  state.galleryError = "";
  renderGallery();
  try {
    const loaded = await loadGalleryCardsUntil(
      snapshot,
      { cards: [], total: 0, resolved: 0, nextOffset: 0 },
      SET_GALLERY_PAGE_SIZE,
      () => galleryRequestGate.accepts(ticket) && context === galleryCatalogContextKey(),
    );
    if (!loaded) return;
    state.galleryCards = loaded.cards;
    state.galleryTotal = loaded.total;
    state.galleryResolved = loaded.resolved;
    state.galleryNextOffset = loaded.nextOffset;
    state.galleryPage = 1;
    state.status = `图库已显示首屏：已扫描目录 ${state.galleryResolved.toLocaleString()} / ${state.galleryTotal.toLocaleString()} 项，整理出 ${state.galleryCards.length.toLocaleString()} 个唯一预览`;
    state.galleryLoading = false;
    renderGallery();
    const status = document.querySelector("#status");
    if (status) status.textContent = state.status;
    scheduleGalleryBackgroundHydration(snapshot, loaded, ticket, context);
  } catch (error) {
    if (!galleryRequestGate.accepts(ticket)) return;
    state.galleryError = String(error);
    state.status = `图库读取失败：${String(error)}`;
    state.galleryLoading = false;
    renderGallery();
    const status = document.querySelector("#status");
    if (status) status.textContent = state.status;
  }
}

function changeGalleryPage(delta: -1 | 1) {
  const dismissedBatch = dismissTerminalGalleryBatch();
  if (dismissedBatch) renderGalleryBatchStatus();
  if (state.galleryLoading || !state.galleryCards.length) return;
  const targetPage = Math.max(1, state.galleryPage + delta);
  if (targetPage === state.galleryPage) return;
  const page = galleryPageWindow(
    state.galleryCards,
    targetPage,
    SET_GALLERY_PAGE_SIZE,
    false,
  );
  state.galleryPage = page.page;
  if (state.gallerySnapshot) {
    showGallerySnapshotPage();
  } else {
    renderFallbackGalleryPage(page);
  }
  updateGalleryPager(page);
  const grid = document.getElementById("galleryGrid");
  if (grid) grid.scrollTop = 0;
}

function mountGallerySnapshotPages(
  grid: HTMLDivElement,
  snapshot: GalleryMemorySnapshot,
) {
  if (
    grid.dataset.gallerySnapshotContext === snapshot.contextKey
    && gallerySnapshotPageElements.length === snapshot.pages.length
  ) return;
  grid.innerHTML = snapshot.pages.map((page) => `
    <section class="gallery-snapshot-page"
      data-gallery-snapshot-page="${page.page}"
      ${page.page === state.galleryPage ? "" : "hidden"}>
      ${page.html}
    </section>
  `).join("");
  grid.dataset.gallerySnapshotContext = snapshot.contextKey;
  grid.dataset.gallerySnapshotVisiblePage = String(state.galleryPage);
  gallerySnapshotPageElements = Array.from(grid.children).filter(
    (element): element is HTMLElement => element instanceof HTMLElement,
  );
  gallerySnapshotVisiblePageIndex = state.galleryPage - 1;
}

function showGallerySnapshotPage() {
  const snapshot = state.gallerySnapshot;
  const grid = document.getElementById("galleryGrid");
  if (!snapshot || !grid || grid.dataset.gallerySnapshotContext !== snapshot.contextKey) return;
  const currentPageIndex = state.galleryPage - 1;
  if (
    gallerySnapshotVisiblePageIndex >= 0
    && gallerySnapshotVisiblePageIndex !== currentPageIndex
  ) {
    const previous = gallerySnapshotPageElements[gallerySnapshotVisiblePageIndex];
    if (previous) previous.hidden = true;
  }
  const current = gallerySnapshotPageElements[currentPageIndex];
  if (!current) return;
  current.hidden = false;
  gallerySnapshotVisiblePageIndex = currentPageIndex;
  grid.dataset.gallerySnapshotVisiblePage = String(state.galleryPage);
  const selectedEntityId = selectedGalleryEntityId();
  for (const child of current.children) {
    if (!(child instanceof HTMLButtonElement)) continue;
    const button = child;
    const entityId = button.dataset.gallerySet ?? button.dataset.galleryItem ?? "";
    button.classList.toggle("selected", entityId === selectedEntityId);
  }
}

function updateGalleryPager(
  page = galleryPageWindow(
    state.galleryCards,
    state.galleryPage,
    SET_GALLERY_PAGE_SIZE,
    false,
  ),
) {
  const pager = document.getElementById("galleryPager");
  const pagePrevious = document.getElementById("galleryPagePrevious") as HTMLButtonElement | null;
  const pageStatus = document.getElementById("galleryPageStatus");
  const pageNext = document.getElementById("galleryPageNext") as HTMLButtonElement | null;
  if (!pager || !pagePrevious || !pageStatus || !pageNext) return;
  const selectionMessage = gallerySelectionMessage();
  pager.hidden = !state.galleryCards.length
    || Boolean(selectionMessage)
    || Boolean(state.galleryError);
  pagePrevious.disabled = state.galleryLoading || !page.hasPrevious;
  pageNext.disabled = state.galleryLoading || !page.hasNext;
  pageNext.title = "";
  pageStatus.textContent = `第 ${page.page} / ${page.loadedPageCount} 页 · 本页 ${page.cards.length} 个`;
}

function renderFallbackGalleryPage(
  page = galleryPageWindow(
    state.galleryCards,
    state.galleryPage,
    SET_GALLERY_PAGE_SIZE,
    false,
  ),
) {
  const grid = document.getElementById("galleryGrid");
  if (!(grid instanceof HTMLDivElement)) return;
  galleryThumbnailObserver.disconnect();
  delete grid.dataset.gallerySnapshotContext;
  grid.innerHTML = page.cards.map(galleryCardHtml).join("");
  grid.querySelectorAll<HTMLElement>("[data-gallery-thumbnail]").forEach((node) => {
    galleryThumbnailObserver.observe(node);
  });
}

function renderGallery() {
  const grid = document.querySelector<HTMLDivElement>("#galleryGrid");
  const meta = document.querySelector<HTMLElement>("#galleryMeta");
  const title = document.querySelector<HTMLElement>("#galleryTitle");
  const pager = document.querySelector<HTMLElement>("#galleryPager");
  const pagePrevious = document.querySelector<HTMLButtonElement>("#galleryPagePrevious");
  const pageStatus = document.querySelector<HTMLElement>("#galleryPageStatus");
  const pageNext = document.querySelector<HTMLButtonElement>("#galleryPageNext");
  const batchButton = document.querySelector<HTMLButtonElement>("#galleryBatchButton");
  if (
    !grid
    || !meta
    || !title
    || !pager
    || !pagePrevious
    || !pageStatus
    || !pageNext
    || !batchButton
  ) return;

  const setPage = galleryPageWindow(
    state.galleryCards,
    state.galleryPage,
    SET_GALLERY_PAGE_SIZE,
    false,
  );
  state.galleryPage = setPage.page;
  galleryThumbnailScheduler.activateContext(galleryContextKey());

  title.textContent = state.mode === "sets"
    ? "套装预览图库"
    : `${slotLabels[state.selectedSlot] ?? "单件"}预览图库`;
  meta.textContent = state.galleryStarted
    ? state.gallerySnapshotPhase === "ready"
      ? `全量快照 ${state.galleryCards.length.toLocaleString()} 个唯一画面 · 全部缩略图已载入内存`
      : `已扫描目录 ${state.galleryResolved.toLocaleString()} / ${state.galleryTotal.toLocaleString()} 项 · 已整理 ${state.galleryCards.length.toLocaleString()} 个唯一画面`
    : "";
  batchButton.textContent = batchActionLabel(state.mode);
  batchButton.disabled = !state.catalogMeta
    || state.selectedProfession === "all"
    || Boolean(
      state.galleryBatch
      && ["queued", "running", "paused", "finalizing"].includes(state.galleryBatch.phase),
    );
  renderGalleryBatchStatus();

  const selectionMessage = gallerySelectionMessage();
  galleryThumbnailObserver.disconnect();
  if (!state.catalogMeta) {
    delete grid.dataset.gallerySnapshotContext;
    grid.innerHTML = `<div class="gallery-empty">请先载入内置数据索引</div>`;
  } else if (selectionMessage) {
    delete grid.dataset.gallerySnapshotContext;
    grid.innerHTML = `<div class="gallery-empty">${escapeHtml(selectionMessage)}</div>`;
  } else if (state.galleryLoading && !state.galleryCards.length) {
    delete grid.dataset.gallerySnapshotContext;
    grid.innerHTML = gallerySkeletonHtml(10);
  } else if (state.galleryError) {
    delete grid.dataset.gallerySnapshotContext;
    grid.innerHTML = `<div class="gallery-empty gallery-error">${escapeHtml(state.galleryError)}</div>`;
  } else if (!state.galleryCards.length) {
    delete grid.dataset.gallerySnapshotContext;
    grid.innerHTML = `<div class="gallery-empty">没有匹配的预览</div>`;
  } else if (state.gallerySnapshot) {
    mountGallerySnapshotPages(grid, state.gallerySnapshot);
    showGallerySnapshotPage();
  } else {
    renderFallbackGalleryPage(setPage);
  }

  updateGalleryPager(setPage);
}

function galleryCardHtml(card: GalleryCard) {
  const selected = selectedGalleryEntityId() === card.entity_id ? " selected" : "";
  const duplicate = (card.duplicate_count ?? 1) > 1
    ? `<span class="gallery-duplicate">同图 ×${card.duplicate_count}</span>`
    : "";
  const thumbnailSource = card.thumbnail_object_url
    || (card.thumbnail_webp_base64
      ? `data:image/webp;base64,${card.thumbnail_webp_base64}`
      : "");
  const ready = card.thumbnail_status === "ready" && Boolean(thumbnailSource);
  const thumbnail = ready
    ? `<img src="${escapeHtml(thumbnailSource)}" alt="" draggable="false" />`
    : `<div class="gallery-thumbnail-placeholder"><span>正在准备预览</span></div>`;
  const observe = !ready
    ? ` data-gallery-thumbnail="${escapeHtml(card.visual_signature)}"`
    : "";
  const entityAttribute = card.entity_kind === "set"
    ? `data-gallery-set="${escapeHtml(card.entity_id)}"`
    : `data-gallery-item="${escapeHtml(card.entity_id)}"`;
  const displayName = cleanDisplayName(card.name) || "未命名";
  return `
    <button class="gallery-card${selected}" type="button" ${entityAttribute} title="${escapeHtml(displayName)}">
      <span class="gallery-thumbnail"${observe}
        data-gallery-kind="${card.entity_kind}"
        data-gallery-entity="${escapeHtml(card.entity_id)}">
        ${duplicate}
        ${thumbnail}
      </span>
      <span class="gallery-card-caption">
        <strong>${escapeHtml(cleanDisplayName(card.name) || "未命名")}</strong>
      </span>
    </button>
  `;
}

function gallerySkeletonHtml(count: number) {
  return Array.from({ length: count }, () => `
    <div class="gallery-card skeleton" aria-hidden="true">
      <span class="gallery-thumbnail"><span></span></span>
      <span class="gallery-card-caption"><strong></strong></span>
    </div>
  `).join("");
}

function selectedGalleryEntityId() {
  return state.mode === "sets"
    ? state.selectedSet?.set_id ?? ""
    : state.selectedItem?.item_id ?? "";
}

function galleryContextKey() {
  return JSON.stringify([
    galleryRefreshGeneration,
    state.workspaceMode,
    state.gameDir,
    state.mode,
    state.selectedProfession,
    state.selectedSlot,
    state.query,
    state.galleryPage,
  ]);
}

async function loadVisibleGalleryThumbnail(node: HTMLElement) {
  if (state.workspaceMode !== "gallery") return;
  const signature = node.dataset.galleryThumbnail ?? "";
  const entityId = node.dataset.galleryEntity ?? "";
  const kind = node.dataset.galleryKind === "set" ? "set" : "item";
  if (!signature || !entityId) return;
  const card = state.galleryCards.find((entry) => entry.visual_signature === signature);
  if (!card || card.thumbnail_status === "ready" || card.thumbnail_status === "failed") return;
  const context = galleryContextKey();
  const requestKey = `${context}:${signature}`;
  const request = Object.freeze({
    game_dir: state.gameDir,
    kind,
    entity_id: entityId,
    playable_role: state.selectedProfession,
  });
  const wasPending = galleryThumbnailScheduler.has(requestKey);
  const scheduled = galleryThumbnailScheduler.schedule(
    requestKey,
    () => invoke<CatalogThumbnailResponse>("load_catalog_thumbnail", { request }),
    context,
  );
  if (!scheduled) return;
  if (!wasPending) {
    state.galleryCards = state.galleryCards.map((entry) => entry.visual_signature === signature
      ? { ...entry, thumbnail_status: "rendering" }
      : entry);
  }
  try {
    const response = await scheduled;
    if (context !== galleryContextKey()) return;
    state.galleryCards = state.galleryCards.map((entry) => entry.visual_signature === signature
      ? {
          ...entry,
          entity_recipe_key: response.entity_recipe_key,
          visual_signature: response.visual_signature,
          thumbnail_status: "ready",
          thumbnail_webp_base64: response.webp_base64,
          error: "",
        }
      : entry);
  } catch (error) {
    if (isGalleryThumbnailCancellation(error)) return;
    if (context !== galleryContextKey()) return;
    state.galleryCards = state.galleryCards.filter(
      (entry) => entry.visual_signature !== signature,
    );
    state.status = `已隐藏预览生成失败的条目：${String(error)}`;
  } finally {
    if (context === galleryContextKey()) {
      renderGallery();
    }
  }
}

function dismissTerminalGalleryBatch() {
  const phase = state.galleryBatch?.phase;
  if (!phase || !["completed", "cancelled", "failed"].includes(phase)) return false;
  window.clearTimeout(galleryBatchPollTimer);
  state.galleryBatch = null;
  return true;
}

function galleryBatchRequest() {
  return {
    game_dir: state.gameDir,
    kind: state.mode === "sets" ? "set" as const : "item" as const,
    playable_role: state.selectedProfession,
  };
}

async function startGalleryBatch() {
  if (!state.catalogMeta || state.selectedProfession === "all") return;
  try {
    const preflight = await invoke<CatalogThumbnailBatchPreflight>(
      "prepare_catalog_thumbnail_batch",
      { request: galleryBatchRequest() },
    );
    const accepted = window.confirm(
      `${batchActionLabel(state.mode)}\n\n`
      + `可显示命名条目：${preflight.total_entities.toLocaleString()}\n`
      + `现有唯一预览：${preflight.cache.blob_count.toLocaleString()}\n`
      + `加密缓存：${formatBytes(preflight.cache.encrypted_bytes)}\n`
      + `本地路径：${preflight.cache_root}\n\n`
      + "生成会在后台继续，可暂停或取消。是否开始？",
    );
    if (!accepted) return;
    state.galleryBatch = await invoke<CatalogThumbnailBatchStatus>(
      "start_catalog_thumbnail_batch",
      { request: galleryBatchRequest() },
    );
    state.galleryBatchGameDir = state.gameDir;
    state.galleryBatchBuildId = state.catalogMeta.build_id;
    renderGallery();
    scheduleGalleryBatchPoll();
  } catch (error) {
    state.status = `批量生成启动失败：${String(error)}`;
    render();
  }
}

function renderGalleryBatchStatus() {
  const root = document.querySelector<HTMLDivElement>("#galleryBatchStatus");
  if (!root) return;
  const snapshotActive = state.gallerySnapshotPhase === "decoding";
  if (snapshotActive) {
    const percent = state.gallerySnapshotTotal
      ? Math.min(100, Math.round(
        state.gallerySnapshotDecoded / state.gallerySnapshotTotal * 100,
      ))
      : 0;
    root.hidden = false;
    root.innerHTML = `
      <div class="gallery-batch-copy">
        <strong>正在预热已生成的本职业预览库</strong>
        <span>已解码 ${state.gallerySnapshotDecoded.toLocaleString()} / ${state.gallerySnapshotTotal.toLocaleString()} 个唯一预览</span>
      </div>
      <div class="gallery-batch-progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${percent}"><span style="width:${percent}%"></span></div>
      <div class="gallery-batch-actions"></div>
    `;
    return;
  }
  const job = state.galleryBatch;
  root.hidden = !job;
  if (!job) {
    root.innerHTML = "";
    return;
  }
  const percent = job.total_entities
    ? Math.min(100, Math.round(job.resolved_entities / job.total_entities * 100))
    : 0;
  const phaseLabels: Record<CatalogThumbnailBatchPhase, string> = {
    queued: "排队中",
    running: "生成中",
    paused: "已暂停",
    finalizing: "正在提交快照",
    completed: "已完成",
    cancelled: "已取消",
    failed: "任务失败",
  };
  const control = job.phase === "running"
    ? `<button data-gallery-batch-action="pause">暂停</button>`
    : job.phase === "paused"
      ? `<button data-gallery-batch-action="resume">继续</button>`
      : "";
  const cancel = ["queued", "running", "paused", "finalizing"].includes(job.phase)
    ? `<button data-gallery-batch-action="cancel">取消</button>`
    : "";
  root.innerHTML = `
    <div class="gallery-batch-copy">
      <strong>${escapeHtml(playableRoleLabel(job.playable_role))} · ${job.kind === "set" ? "套装" : "全部单件"} · ${phaseLabels[job.phase]}</strong>
      <span>已处理命名条目 ${job.resolved_entities.toLocaleString()} / ${job.total_entities.toLocaleString()} · 唯一预览 ${job.unique_visuals.toLocaleString()}</span>
    </div>
    <div class="gallery-batch-progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${percent}"><span style="width:${percent}%"></span></div>
    <div class="gallery-batch-actions">${control}${cancel}</div>
  `;
  root.querySelectorAll<HTMLButtonElement>("[data-gallery-batch-action]").forEach((button) => {
    button.addEventListener("click", () => {
      const action = button.dataset.galleryBatchAction as "pause" | "resume" | "cancel";
      void controlGalleryBatch(action);
    });
  });
}

async function controlGalleryBatch(action: "pause" | "resume" | "cancel") {
  if (!state.galleryBatch) return;
  try {
    state.galleryBatch = await invoke<CatalogThumbnailBatchStatus>(
      "control_catalog_thumbnail_batch",
      { jobId: state.galleryBatch.job_id, action },
    );
    renderGallery();
    scheduleGalleryBatchPoll();
  } catch (error) {
    state.status = String(error);
    render();
  }
}

function scheduleGalleryBatchPoll() {
  window.clearTimeout(galleryBatchPollTimer);
  const job = state.galleryBatch;
  if (!job || !["queued", "running", "paused", "finalizing"].includes(job.phase)) return;
  galleryBatchPollTimer = window.setTimeout(() => void pollGalleryBatch(), 650);
}

async function pollGalleryBatch() {
  if (!state.galleryBatch) return;
  const jobId = state.galleryBatch.job_id;
  try {
    const status = await invoke<CatalogThumbnailBatchStatus>(
      "get_catalog_thumbnail_batch_status",
      { jobId },
    );
    if (state.galleryBatch?.job_id !== jobId) return;
    state.galleryBatch = status;
    renderGallery();
    const completedCurrentScope = status.phase === "completed"
      && state.workspaceMode === "gallery"
      && (state.galleryBatchGameDir || state.gameDir) === state.gameDir
      && (state.galleryBatchBuildId || state.catalogMeta?.build_id || "")
        === (state.catalogMeta?.build_id || "")
      && status.kind === (state.mode === "sets" ? "set" : "item")
      && status.playable_role === state.selectedProfession;
    if (completedCurrentScope) {
      void refreshGallery();
    }
  } catch (error) {
    state.status = String(error);
  } finally {
    scheduleGalleryBatchPoll();
  }
}

async function showPreviewLibrary() {
  try {
    const [summary, jobs] = await Promise.all([
      invoke<CatalogThumbnailBatchPreflight["cache"]>("get_catalog_thumbnail_cache_summary"),
      invoke<CatalogThumbnailBatchStatus[]>("list_catalog_thumbnail_batch_jobs"),
    ]);
    const recent = jobs.slice(0, 6).map((job) => (
      `${playableRoleLabel(job.playable_role)} · ${job.kind === "set" ? "套装" : "全部单件"} · ${job.phase}`
    )).join("\n");
    window.alert(
      `本地加密预览库\n\n`
      + `实体索引：${summary.entity_count.toLocaleString()}\n`
      + `唯一缩略图：${summary.blob_count.toLocaleString()}\n`
      + `占用空间：${formatBytes(summary.encrypted_bytes)}`
      + (recent ? `\n\n最近任务：\n${recent}` : ""),
    );
  } catch (error) {
    state.status = `读取预览库失败：${String(error)}`;
    render();
  }
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function render() {
  const dyeOpen = avatarDyePageController?.isOpen() ?? false;
  galleryThumbnailScheduler.activateContext(galleryContextKey());
  document.querySelector(".shell")?.classList.toggle("gallery-view", state.workspaceMode === "gallery");
  const currentModeButton = document.querySelector<HTMLButtonElement>("#workspaceModeCurrent");
  const galleryModeButton = document.querySelector<HTMLButtonElement>("#workspaceModeGallery");
  currentModeButton?.classList.toggle("active", state.workspaceMode === "current");
  galleryModeButton?.classList.toggle("active", state.workspaceMode === "gallery");
  currentModeButton?.setAttribute("aria-pressed", String(state.workspaceMode === "current"));
  galleryModeButton?.setAttribute("aria-pressed", String(state.workspaceMode === "gallery"));
  document.querySelector("#status")!.textContent = state.loading || state.rendering || state.exporting ? "处理中" : state.status;
  document.querySelector("#stats")!.innerHTML = statsHtml();
  document.querySelector("#resultTitle")!.textContent = state.mode === "sets" ? "套装 / 礼盒款式" : "单件 / 自由搭配";
  renderPreviewHeading();
  syncHiddenFeatureEntries();
  document.querySelector("#modeSets")?.classList.toggle("active", !dyeOpen && !state.outfitSwapOpen && !state.wardrobeOpen && state.mode === "sets");
  document.querySelector("#modeItems")?.classList.toggle("active", !dyeOpen && !state.outfitSwapOpen && !state.wardrobeOpen && state.mode === "items");
  document.querySelector("#modeWardrobe")?.classList.toggle("active", state.wardrobeOpen);
  document.querySelector("#modeOutfitSwap")?.classList.toggle("active", state.outfitSwapOpen);
  renderFilterChips();
  renderOutfitGrid();
  renderList();
  renderGallery();
  renderLayerRows();
  syncPlaybackButton();
  renderWardrobePage();
  renderOutfitSwapPage();
  renderSingleCarrierDialog();
  const dyeEntry = document.querySelector<HTMLButtonElement>("#openAvatarDye");
  if (dyeEntry) dyeEntry.disabled = !state.activeLayers.length || state.rendering || state.loading;
  const previewControlsDisabled = catalogSelectionGate.isPending()
    || state.rendering
    || state.loading
    || !previewLayerRequests().length;
  const rerender = document.querySelector<HTMLButtonElement>("#rerender");
  if (rerender) rerender.disabled = previewControlsDisabled;
  const playback = document.querySelector<HTMLButtonElement>("#playPreview");
  if (playback) playback.disabled = previewControlsDisabled && !state.playing;
  avatarDyePageController?.render();
}

function syncHiddenFeatureEntries() {
  const wardrobeHidden = !state.wardrobeSwapEntryVisible;
  for (const selector of [
    "#modeWardrobe",
    "#modeOutfitSwap",
    "#saveCurrentOutfit",
    "#saveTargetOutfit",
    "#saveToWardrobe",
    "#openAvatarDye",
  ]) {
    const element = document.querySelector<HTMLElement>(selector);
    if (element) element.hidden = wardrobeHidden;
  }
}

function statsHtml() {
  const stats = state.catalogMeta?.stats;
  return `
    <div><span>物品</span><strong>${stats?.item_count.toLocaleString() ?? "-"}</strong></div>
    <div><span>套装</span><strong>${stats?.set_count.toLocaleString() ?? "-"}</strong></div>
    <div><span>图层</span><strong>${stats?.layer_count.toLocaleString() ?? "-"}</strong></div>
  `;
}

function renderFilterChips() {
  const professionNode = document.querySelector<HTMLDivElement>("#professionChips")!;
  const professionOptions = playableRoleOptions();
  if (!professionOptions.some(([key]) => key === state.selectedProfession)) {
    state.selectedProfession = "all";
  }
  professionNode.innerHTML = professionOptions
    .map(([key, label]) => `<button class="chip${state.selectedProfession === key ? " active" : ""}" data-profession="${escapeHtml(key)}">${escapeHtml(label)}</button>`)
    .join("");
  professionNode.querySelectorAll<HTMLButtonElement>("[data-profession]").forEach((button) => {
    button.addEventListener("click", () => {
      dismissTerminalGalleryBatch();
      const nextProfession = button.dataset.profession ?? "all";
      if (nextProfession === state.selectedProfession) {
        renderGallery();
        refreshCatalogQuery(false);
        return;
      }
      const selectedSetId = state.selectedSet
        && matchesSetProfession(state.selectedSet, nextProfession)
        ? state.selectedSet.set_id
        : "";
      stopPlayback();
      resetAnimationCache();
      invalidateCatalogSelection();
      state.selectedProfession = nextProfession;
      state.selectedSet = null;
      state.selectedSetItems = [];
      state.selectedItem = null;
      for (const slot of slots) state.outfit[slot] = null;
      state.activeLayers = [];
      state.activePlayableRoleKey = "";
      state.missingImgs = [];
      state.frame = previewStartFrameForProfession(nextProfession);
      clearCanvas();
      render();
      refreshCatalogQuery(false);
      if (selectedSetId) void selectSet(selectedSetId);
    });
  });
}

function renderOutfitGrid() {
  const grid = document.querySelector<HTMLDivElement>("#outfitGrid")!;
  grid.hidden = false;
  grid.innerHTML = slots
    .map((slot) => {
      const item = state.outfit[slot];
      const label = item ? escapeHtml(cleanDisplayName(item.item_name) || "未知单件") : "未选";
      return `
        <div class="outfit-cell">
          <button class="outfit-slot" data-slot-pick="${slot}">
            <span>${slotLabels[slot]}</span>
            <strong>${label}</strong>
          </button>
          ${item ? `<button class="slot-clear" data-slot-clear="${slot}" title="清空该部位">×</button>` : ""}
        </div>
      `;
    })
    .join("") + `<button class="outfit-clear" id="clearOutfit"></button>`;
  grid.querySelectorAll<HTMLButtonElement>("[data-slot-pick]").forEach((button) => {
    button.addEventListener("click", () => {
      invalidateCatalogSelection();
      state.selectedSlot = button.dataset.slotPick ?? "all";
      state.mode = "items";
      refreshCatalogQuery(false);
    });
  });
  grid.querySelectorAll<HTMLButtonElement>("[data-slot-clear]").forEach((button) => {
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      void clearOutfitSlot(button.dataset.slotClear as SlotKey);
    });
  });
  const clear = grid.querySelector<HTMLButtonElement>("#clearOutfit");
  if (clear) {
    const icon = createElement(Trash2);
    icon.setAttribute("width", "15");
    icon.setAttribute("height", "15");
    clear.replaceChildren(icon, document.createTextNode("清空"));
    clear.addEventListener("click", clearOutfit);
  }
}

function renderList() {
  const list = document.querySelector<HTMLDivElement>("#resultList")!;
  if (!state.catalogMeta) {
    list.innerHTML = `<div class="empty">未载入</div>`;
    return;
  }
  if (state.catalogQueryLoading) {
    list.innerHTML = `<div class="empty">正在查询…</div>`;
    return;
  }
  if (state.catalogQueryError) {
    list.innerHTML = `<div class="empty">${escapeHtml(state.catalogQueryError)}</div>`;
    return;
  }
  if (!state.catalogQueryStarted) {
    list.innerHTML = `<div class="empty">请选择模式、部位或输入搜索词</div>`;
    return;
  }
  if (state.mode === "sets") {
    const rows = state.resultSets;
    list.innerHTML = rows.map(setRowHtml).join("") || `<div class="empty">无匹配套装</div>`;
    list.querySelectorAll<HTMLButtonElement>("[data-set]").forEach((button) => {
      button.addEventListener("click", () => void selectSet(button.dataset.set!));
    });
  } else {
    const rows = state.resultItems;
    list.innerHTML = rows.map(itemRowHtml).join("") || `<div class="empty">无匹配单件</div>`;
    list.querySelectorAll<HTMLButtonElement>("[data-item]").forEach((button) => {
      button.addEventListener("click", () => void selectItem(button.dataset.item!));
    });
  }
}

function setRowHtml(set: AvatarSet) {
  const active = state.selectedSet?.set_id === set.set_id ? " active" : "";
  const setName = cleanDisplayName(set.set_name) || "未知套装";
  const names = set.member_names.map(cleanDisplayName).filter(Boolean).slice(0, 8).map(escapeHtml).join(" / ");
  return `
    <button class="result-row${active}" data-set="${escapeHtml(set.set_id)}">
      <strong>${escapeHtml(setName)}</strong>
      <span>${set.member_ids.length} 件 · ${names}</span>
    </button>
  `;
}

function itemRowHtml(item: AvatarItem) {
  const active = state.selectedItem?.item_id === item.item_id ? " active" : "";
  const studioSlot = studioSlotForItem(item);
  const rowPlayableRoleKey = state.selectedProfession !== "all"
    ? state.selectedProfession
    : (itemPlayableRoles(item).length === 1
      ? itemPlayableRoles(item)[0]
      : "all");
  const layerCount = layerCountForPlayableRole(item, rowPlayableRoleKey);
  return `
    <button class="result-row${active}" data-item="${escapeHtml(item.item_id)}">
      <strong>${escapeHtml(cleanDisplayName(item.item_name) || "未知单件")}</strong>
      <span>${slotLabels[studioSlot ?? item.slot] ?? item.slot} · ${professionLabel(item)} · ${layerCount.toLocaleString()} 层</span>
    </button>
  `;
}

async function selectSet(setId: string) {
  stopPlayback();
  resetAnimationCache();
  const selectionTicket = issueCatalogSelection();
  const isCurrentSelection = () => catalogSelectionGate.accepts(selectionTicket);
  state.status = "正在读取套装成员";
  render();
  try {
    const detail = await invoke<AvatarSetDetail>("get_avatar_set_with_members", {
      setId,
      playableRole: state.selectedProfession,
    });
    if (!isCurrentSelection()) return;
    cacheSelectedEntities(detail.items);
    state.selectedSet = detail.set;
    state.selectedSetItems = detail.items;
    state.selectedItem = null;
    state.activePlayableRoleKey = "";
    state.frame = previewStartFrameForProfession(state.selectedProfession);
    state.activeLayers = [];
    const memberIds = setMemberIdsForProfession(detail.set, state.selectedProfession);
    fillOutfitFromSet(detail.set);
    render();
    if (!memberIds.length) {
      state.status = state.selectedProfession === "all" && setPlayableRoles(detail.set).length > 1
        ? "该套装包含多个职业，请先选择一个具体职业"
        : "该套装没有当前职业可用的成员";
      clearCanvas();
      render();
      return;
    }
    await loadLayersAndRender(memberIds, detail.set, isCurrentSelection);
  } catch (error) {
    if (!isCurrentSelection()) return;
    state.selectedSet = null;
    state.selectedSetItems = [];
    state.activeLayers = [];
    state.activePlayableRoleKey = "";
    state.status = String(error);
    clearCanvas();
    render();
  } finally {
    finishCatalogSelection(selectionTicket);
  }
}

function fillOutfitFromSet(set: AvatarSet | null) {
  for (const slot of slots) state.outfit[slot] = null;
  if (!set) return;
  for (const item of setMemberItemsForProfession(set, state.selectedProfession)) {
    const studioSlot = item ? studioSlotForItem(item) : null;
    if (item && studioSlot) {
      state.outfit[studioSlot] = item;
    }
  }
}

async function selectItem(itemId: string) {
  stopPlayback();
  resetAnimationCache();
  const selectionTicket = issueCatalogSelection();
  const isCurrentSelection = () => catalogSelectionGate.accepts(selectionTicket);
  state.status = "正在读取单件详情";
  render();
  try {
    const items = await invoke<AvatarItem[]>("get_avatar_items_by_ids", {
      itemIds: [itemId],
    });
    if (!isCurrentSelection()) return;
    const item = items[0];
    if (!item || item.item_id !== itemId) {
      throw new Error(`内置数据缺少物品：${itemId}`);
    }
    cacheSelectedEntities(items);
    state.selectedItem = item;
    state.selectedSet = null;
    state.selectedSetItems = [];
    state.activePlayableRoleKey = "";
    state.frame = previewStartFrameForProfession(state.selectedProfession);
    const studioSlot = studioSlotForItem(item);
    if (studioSlot) {
      state.outfit[studioSlot] = item;
    }
    await loadOutfitAndRender(isCurrentSelection);
  } catch (error) {
    if (!isCurrentSelection()) return;
    state.selectedItem = null;
    state.selectedSetItems = [];
    state.activeLayers = [];
    state.activePlayableRoleKey = "";
    state.status = String(error);
    clearCanvas();
    render();
  } finally {
    finishCatalogSelection(selectionTicket);
  }
}

async function clearOutfit() {
  stopPlayback();
  resetAnimationCache();
  invalidateCatalogSelection();
  for (const slot of slots) state.outfit[slot] = null;
  state.selectedItem = null;
  state.selectedSet = null;
  state.selectedSetItems = [];
  state.activeLayers = [];
  state.activePlayableRoleKey = "";
  state.frame = previewStartFrameForProfession(state.selectedProfession);
  state.missingImgs = [];
  render();
  clearCanvas();
}

async function clearOutfitSlot(slot: SlotKey) {
  stopPlayback();
  resetAnimationCache();
  const selectionTicket = issueCatalogSelection();
  const isCurrentSelection = () => catalogSelectionGate.accepts(selectionTicket);
  state.outfit[slot] = null;
  state.selectedSet = null;
  state.selectedSetItems = [];
  state.selectedItem = null;
  state.mode = "items";
  state.activePlayableRoleKey = "";
  state.frame = previewStartFrameForProfession(state.selectedProfession);
  try {
    await loadOutfitAndRender(isCurrentSelection);
  } finally {
    finishCatalogSelection(selectionTicket);
  }
}

async function loadOutfitAndRender(isCurrent: () => boolean) {
  if (!isCurrent()) return;
  const itemIds = slots.map((slot) => state.outfit[slot]?.item_id).filter(Boolean) as string[];
  if (!itemIds.length) {
    resetAnimationCache();
    state.activeLayers = [];
    state.activePlayableRoleKey = "";
    clearCanvas();
    render();
    return;
  }
  await loadLayersAndRender(itemIds, null, isCurrent);
}

async function loadLayersAndRender(
  itemIds: string[],
  set: AvatarSet | null,
  isCurrent: () => boolean,
) {
  if (!isCurrent()) return;
  const selectedItems = selectedItemsByIds(itemIds);
  const effectivePlayableRoleKey = effectivePlayableRoleForItems(
    selectedItems,
    state.selectedProfession,
    set,
  );
  if (!effectivePlayableRoleKey) {
    state.activeLayers = [];
    state.activePlayableRoleKey = "";
    state.frame = 0;
    state.status = "当前选择涉及多个职业，请先选择一个具体职业";
    clearCanvas();
    render();
    return;
  }
  state.activePlayableRoleKey = effectivePlayableRoleKey;
  state.frame = previewStartFrameForProfession(effectivePlayableRoleKey);
  state.rendering = true;
  state.status = "正在读取图层数据";
  render();
  try {
    const layers = await invoke<AvatarLayer[]>("load_avatar_layers", {
      itemIds,
      playableRoleKey: effectivePlayableRoleKey,
    });
    if (!isCurrent()) return;
    const withSkin = ensureDefaultSkin(layers, effectivePlayableRoleKey);
    stopPlayback();
    resetAnimationCache();
    state.activeLayers = sortLayersForDraw(withSkin);
    await renderActivePreview(issueCatalogPreview(isCurrent));
  } catch (error) {
    if (!isCurrent()) return;
    catalogPreviewGate.invalidate();
    resetAnimationCache();
    state.activeLayers = [];
    state.activePlayableRoleKey = "";
    state.status = String(error);
    clearCanvas();
  } finally {
    if (isCurrent()) {
      state.rendering = false;
      render();
    }
  }
}

async function rerenderActivePreview() {
  if (isCatalogSelectionPending() || state.rendering || state.loading) return;
  stopPlayback();
  await renderActivePreview(issueCatalogPreview());
}

async function renderActivePreview(isCurrent: () => boolean) {
  if (!isCurrent()) return;
  renderPreviewHeading();
  if (!previewLayerRequests().length) {
    clearCanvas();
    return;
  }
  if (state.rendering && state.playing) return;
  state.rendering = true;
  render();
  try {
    const image = await invoke<PreviewImage>("render_avatar_layers", {
      gameDir: state.gameDir,
      layers: previewLayerRequests(),
    });
    if (!isCurrent()) return;
    state.missingImgs = image.missing_imgs;
    drawPreview(image);
    state.status = image.missing_imgs.length
      ? `合成完成，缺少 ${image.missing_imgs.length} 个 IMG`
      : `合成完成 · 第 ${state.frame} 帧`;
  } catch (error) {
    if (!isCurrent()) return;
    state.status = String(error);
    clearCanvas();
    stopPlayback();
  } finally {
    if (isCurrent()) {
      state.rendering = false;
      render();
    }
  }
}

function previewLayerRequests(): PreviewLayerRequest[] {
  return visibleAvatarLayers(state.activeLayers)
    .filter((layer) => layer.visual_img.trim())
    .map((layer) => ({
      img_path: layer.visual_img,
      frame_index: state.frame,
      palette_index: layer.palette_index,
      enable_dye_mask1: layer.enable_dye_mask1,
      enable_dye_mask2: layer.enable_dye_mask2,
    }));
}

function togglePlayback() {
  if (state.playing) {
    stopPlayback();
  } else {
    void startPlayback();
  }
}

async function startPlayback() {
  if (
    isCatalogSelectionPending()
    || state.rendering
    || state.loading
    || !previewLayerRequests().length
  ) return;
  const isCurrentPreview = issueCatalogPreview();
  const ready = await ensureAnimationFrames(isCurrentPreview);
  if (!isCurrentPreview() || !ready || !state.animationFrames.length) return;
  state.playing = true;
  state.playFrameIndex = Math.min(state.frame, state.animationFrames.length - 1);
  state.lastFrameAt = performance.now();
  state.lastStatusAt = 0;
  drawAnimationFrame(state.playFrameIndex);
  syncPlaybackButton();
  state.playTimer = window.requestAnimationFrame(playAnimationTick);
}

function stopPlayback() {
  if (state.playTimer) {
    window.cancelAnimationFrame(state.playTimer);
    state.playTimer = 0;
  }
  state.playing = false;
  syncPlaybackButton();
}

async function ensureAnimationFrames(isCurrent: () => boolean) {
  if (!isCurrent()) return false;
  const cacheKey = animationCacheKey();
  if (state.animationFrames.length && state.animationCacheKey === cacheKey) {
    return true;
  }
  state.rendering = true;
  state.status = "正在预渲染全部动画帧";
  render();
  try {
    const animation = await invoke<PreviewAnimation>("render_avatar_animation", {
      gameDir: state.gameDir,
      layers: previewLayerRequests(),
    });
    if (!isCurrent()) return false;
    state.animationFrames = animation.frames;
    state.animationImageData = animation.frames.map(toImageData);
    state.animationCacheKey = cacheKey;
    state.missingImgs = animation.missing_imgs;
    const startFrame = previewFrameIndexForProfession(
      state.activePlayableRoleKey,
      state.animationFrames.length,
    );
    state.frame = startFrame;
    state.playFrameIndex = startFrame;
    if (state.animationFrames[startFrame]) {
      drawAnimationFrame(startFrame);
    }
    state.status = animation.missing_imgs.length
      ? `预渲染完成 · ${animation.frame_count} 帧 · 缺少 ${animation.missing_imgs.length} 个 IMG`
      : `预渲染完成 · ${animation.frame_count} 帧`;
    return true;
  } catch (error) {
    if (!isCurrent()) return false;
    state.status = String(error);
    resetAnimationCache();
    clearCanvas();
    return false;
  } finally {
    if (isCurrent()) {
      state.rendering = false;
      render();
    }
  }
}

function playAnimationTick(now: number) {
  if (!state.playing || !state.animationFrames.length) return;
  if (now - state.lastFrameAt >= PLAY_FRAME_MS) {
    const step = Math.max(1, Math.floor((now - state.lastFrameAt) / PLAY_FRAME_MS));
    state.lastFrameAt += step * PLAY_FRAME_MS;
    state.playFrameIndex = (state.playFrameIndex + step) % state.animationFrames.length;
    state.frame = state.playFrameIndex;
    drawAnimationFrame(state.playFrameIndex);
    if (now - state.lastStatusAt > 500) {
      state.lastStatusAt = now;
      state.status = `播放中 · 第 ${state.playFrameIndex + 1}/${state.animationFrames.length} 帧`;
      const status = document.querySelector("#status");
      if (status) status.textContent = state.status;
    }
  }
  state.playTimer = window.requestAnimationFrame(playAnimationTick);
}

function resetAnimationCache() {
  state.animationFrames = [];
  state.animationImageData = [];
  state.animationCacheKey = "";
  state.playFrameIndex = 0;
  state.lastFrameAt = 0;
  state.lastStatusAt = 0;
}

function animationCacheKey() {
  return JSON.stringify({
    gameDir: state.gameDir,
    layers: visibleAvatarLayers(state.activeLayers)
      .filter((layer) => layer.visual_img.trim())
      .map((layer) => [
        layer.visual_img,
        layer.palette_index,
        layer.layer_order,
        layer.equipment_type_enum,
        layer.layer_sequence,
        layer.enable_dye_mask1,
        layer.enable_dye_mask2,
      ]),
  });
}

function syncPlaybackButton() {
  const button = document.querySelector<HTMLButtonElement>("#playPreview");
  if (!button) return;
  const icon = createElement(state.playing ? Pause : Play);
  icon.setAttribute("width", "16");
  icon.setAttribute("height", "16");
  button.replaceChildren(icon, document.createTextNode(state.playing ? "暂停" : "播放"));
}

async function exportNpk(mode: "composed" | "original") {
  if (!previewLayerRequests().length) {
    state.status = "没有可导出的图层";
    render();
    return;
  }
  stopPlayback();
  const composed = mode === "composed";
  const selected = await save({
    defaultPath: `${sanitizeFileName(previewTitle())}_${composed ? "合成" : "原图"}.${composed ? "webp" : "npk"}`,
    filters: composed
      ? [
          { name: "WebP", extensions: ["webp"] },
          { name: "NPK", extensions: ["npk"] },
        ]
      : [{ name: "NPK", extensions: ["npk"] }],
  });
  if (!selected) return;
  const isWebp = composed && selected.toLowerCase().endsWith("webp");
  state.exporting = true;
  state.status = isWebp ? "正在导出合成 WebP" : composed ? "正在导出合成 NPK" : "正在导出原始 IMG NPK";
  render();
  try {
    if (isWebp) {
      const result = await invoke<ExportWebpResult>("export_avatar_webp", {
        request: {
          game_dir: state.gameDir,
          output_path: selected,
          frame_duration_ms: PLAY_FRAME_MS,
          layers: previewLayerRequests(),
        },
      });
      state.status = `导出完成：动态 WebP ${result.frame_count} 帧 · ${result.width}x${result.height}`;
    } else {
      const result = await invoke<ExportNpkResult>("export_avatar_npk", {
        request: {
          game_dir: state.gameDir,
          output_path: selected,
          entry_name: `sprite/avatar_studio/${sanitizeAscii(previewTitle()) || "composed"}.img`,
          mode,
          layers: previewLayerRequests(),
        },
      });
      state.status = `导出完成：${result.entry_count} 个 IMG`;
    }
  } catch (error) {
    state.status = String(error);
  } finally {
    state.exporting = false;
    render();
  }
}

function drawPreview(image: PreviewImage) {
  drawImageData(image.width, image.height, toImageData(image));
}

function toImageData(image: PreviewImage) {
  const bytes = Uint8ClampedArray.from(atob(image.rgba_base64), (char) => char.charCodeAt(0));
  return new ImageData(bytes, image.width, image.height);
}

function drawAnimationFrame(index: number) {
  const image = state.animationFrames[index];
  const imageData = state.animationImageData[index];
  if (!image || !imageData) return;
  drawImageData(image.width, image.height, imageData);
}

function drawImageData(width: number, height: number, imageData: ImageData) {
  const canvas = document.querySelector<HTMLCanvasElement>("#previewCanvas")!;
  const ctx = canvas.getContext("2d")!;
  canvas.width = Math.max(1, width);
  canvas.height = Math.max(1, height);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.putImageData(imageData, 0, 0);
}

function clearCanvas() {
  const canvas = document.querySelector<HTMLCanvasElement>("#previewCanvas")!;
  const ctx = canvas.getContext("2d")!;
  canvas.width = 640;
  canvas.height = 520;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
}

function renderLayerRows() {
  const tbody = document.querySelector<HTMLTableSectionElement>("#layerRows")!;
  if (!state.activeLayers.length) {
    tbody.innerHTML = '<tr><td colspan="5" class="empty-cell">无图层</td></tr>';
    return;
  }
  const missing = new Set(state.missingImgs.map(imgPathForDisplay));
  const resolvedLayers = resolveAvatarLayerVisibility(state.activeLayers);
  const displayedLayers = resolvedLayers
    .filter((entry) => entry.visible && entry.layer.visual_img.trim())
    .slice(0, 320);
  if (!displayedLayers.length) {
    tbody.innerHTML = '<tr><td colspan="5" class="empty-cell">无可见图层</td></tr>';
    return;
  }
  tbody.innerHTML = displayedLayers
    .map((visibility, index) => {
      const layer = visibility.layer;
      const imgPath = imgPathForDisplay(layer.visual_img);
      const isMissing = missing.has(imgPath);
        const dyeButton = state.wardrobeSwapEntryVisible && visibility.visible && layer.visual_img.trim()
          ? `<button class="layer-dye-button" data-open-dye-layer="${index}"${isMissing ? " disabled" : ""}>调色板</button>`
          : "";
        return `
          <tr class="${isMissing ? "missing" : ""}">
            <td>${escapeHtml(slotLabels[layer.slot] ?? layer.slot)}</td>
            <td>${escapeHtml(layer.layer_tag)}</td>
            <td class="img-path-cell" title="${escapeHtml(imgPath)}">${escapeHtml(imgPath || "-")}</td>
          <td>${isMissing ? "缺失" : escapeHtml(layer.status)}</td>
          <td>${dyeButton}</td>
        </tr>
      `;
    })
    .join("");
  tbody.querySelectorAll<HTMLButtonElement>("[data-open-dye-layer]").forEach((button) => {
    button.addEventListener("click", () => {
      if (!state.wardrobeSwapEntryVisible) return;
      const entry = displayedLayers[Number(button.dataset.openDyeLayer)];
      if (entry?.visible) void openAvatarDyeFromStudio(entry.layer.layer_tag);
    });
  });
}

function ensureDefaultSkin(layers: AvatarLayer[], playableRoleKey: string) {
  if (layers.some((layer) => layer.slot === "skin")) return layers;
  const visualFamily = visualFamilyForPlayableRole(playableRoleKey);
  if (!visualFamily?.default_skin_img.trim()) return layers;
  return [
    {
      item_id: "default_skin",
      item_name: "默认皮肤",
      slot: "skin",
      visual_family_key: visualFamily.visual_family_key,
      layer_order: -100,
      equipment_type_enum: 8,
      hide_equipment_type_enums: [],
      enable_dye_mask1: false,
      enable_dye_mask2: false,
      layer_sequence: 0,
      layer_tag: "[skin]",
      variation: "0",
      palette_index: 0,
      visual_img: visualFamily.default_skin_img,
      status: "default_skin",
    },
    ...layers,
  ];
}

function sortLayersForDraw(layers: AvatarLayer[]) {
  return [...layers].sort((left, right) => {
    if (left.layer_order !== right.layer_order) return left.layer_order - right.layer_order;
    if (left.equipment_type_enum !== right.equipment_type_enum) {
      return left.equipment_type_enum - right.equipment_type_enum;
    }
    return left.layer_sequence - right.layer_sequence;
  });
}

function isStudioWeaponItem(item: AvatarItem | null | undefined) {
  if (!item) return false;
  return item.resource_kind === "weapon" || item.resource_kind === "weapon_avatar";
}

function studioSlotForItem(item: AvatarItem): SlotKey | null {
  if (isStudioWeaponItem(item)) return "weapon";
  return slots.includes(item.slot as SlotKey) ? item.slot as SlotKey : null;
}

function roleBinding(playableRoleKey: string) {
  const wanted = playableRoleKey.trim().toLowerCase();
  return state.catalogMeta?.roles.find(
    (role) => role.playable_role_key.trim().toLowerCase() === wanted,
  ) ?? null;
}

function visualFamilyByKey(visualFamilyKey: string) {
  const wanted = visualFamilyKey.trim().toLowerCase();
  return state.catalogMeta?.visual_families.find(
    (family) => family.visual_family_key.trim().toLowerCase() === wanted,
  ) ?? null;
}

function visualFamilyForPlayableRole(playableRoleKey: string) {
  const binding = roleBinding(playableRoleKey);
  return binding ? visualFamilyByKey(binding.visual_family_key) : null;
}

function playableRoleOptions(): Array<readonly [string, string]> {
  const catalogRoles = state.catalogMeta?.roles ?? [];
  const availableRoleKeys = new Set(
    catalogRoles
      .map((role) => role.playable_role_key.trim().toLowerCase())
      .filter(Boolean),
  );
  const options: Array<readonly [string, string]> = [["all", "全部职业"]];
  const seen = new Set(["all"]);
  for (const [key, label] of professionLabels) {
    if (key === "all" || !availableRoleKeys.has(key) || seen.has(key)) continue;
    seen.add(key);
    options.push([key, label]);
  }
  for (const role of catalogRoles) {
    const key = role.playable_role_key.trim().toLowerCase();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    options.push([key, playableRoleLabel(key)]);
  }
  return options;
}

function matchesProfession(item: AvatarItem, selected: string) {
  if (selected === "all") return true;
  return itemPlayableRoles(item).includes(selected);
}

function professionLabel(item: AvatarItem) {
  const labels = itemPlayableRoles(item).map(playableRoleLabel);
  return labels.join(" / ") || item.profession_dir || "未知职业";
}

function playableRoleLabel(playableRoleKey: string) {
  return (
    professionLabels.find(([candidate]) => candidate === playableRoleKey)?.[1]
    ?? playableRoleKey
  ) || "未知职业";
}

function itemPlayableRoles(item: AvatarItem) {
  return [...new Set(
    item.playable_roles
      .map((key) => key.trim().toLowerCase())
      .filter((key) => key && key !== "all"),
  )];
}

function matchesSetProfession(set: AvatarSet, selected: string) {
  if (selected === "all") return true;
  return setPlayableRoles(set).includes(selected);
}

const SELECTED_ENTITY_CACHE_LIMIT = 256;

function cacheSelectedEntities(items: AvatarItem[]) {
  for (const item of items) {
    state.selectedEntities.delete(item.item_id);
    state.selectedEntities.set(item.item_id, item);
  }
  while (state.selectedEntities.size > SELECTED_ENTITY_CACHE_LIMIT) {
    const oldest = state.selectedEntities.keys().next().value;
    if (oldest === undefined) break;
    state.selectedEntities.delete(oldest);
  }
}

function selectedItemsByIds(itemIds: string[]) {
  const outfitItems = new Map(
    slots
      .map((slot) => state.outfit[slot])
      .filter((item): item is AvatarItem => Boolean(item))
      .map((item) => [item.item_id, item]),
  );
  return itemIds
    .map((itemId) => state.selectedEntities.get(itemId) ?? outfitItems.get(itemId))
    .filter((item): item is AvatarItem => Boolean(item));
}

function setMemberItems(set: AvatarSet) {
  return projectSetMemberRecords(set.member_ids, state.selectedSetItems);
}

function setPlayableRoles(set: AvatarSet) {
  return [...new Set(
    set.playable_roles
      .map((key) => key.trim().toLowerCase())
      .filter((key) => key && key !== "all"),
  )];
}

function commonPlayableRoles(items: AvatarItem[]) {
  let common: Set<string> | null = null;
  for (const item of items) {
    const keys = itemPlayableRoles(item);
    if (!keys.length) continue;
    if (common === null) {
      common = new Set(keys);
      continue;
    }
    common = new Set(keys.filter((key) => common!.has(key)));
  }
  return [...(common ?? new Set<string>())];
}

function effectivePlayableRoleForItems(
  items: AvatarItem[],
  selected: string,
  set: AvatarSet | null = null,
) {
  if (selected !== "all") {
    const setMatches = !set || matchesSetProfession(set, selected);
    return setMatches && items.every((item) => matchesProfession(item, selected))
      ? selected
      : null;
  }
  const setKeys = set ? setPlayableRoles(set) : [];
  if (setKeys.length === 1 && items.every((item) => matchesProfession(item, setKeys[0]))) {
    return setKeys[0];
  }
  const common = commonPlayableRoles(items);
  return common.length === 1 ? common[0] : null;
}

function setMemberItemsForProfession(set: AvatarSet, selected: string) {
  const members = setMemberItems(set);
  const effective = selected !== "all"
    ? (matchesSetProfession(set, selected) ? selected : null)
    : (setPlayableRoles(set).length === 1 ? setPlayableRoles(set)[0] : null);
  if (!effective) return [];
  return members.filter((item) => matchesProfession(item, effective));
}

function setMemberIdsForProfession(set: AvatarSet, selected: string) {
  return setMemberItemsForProfession(set, selected).map((item) => item.item_id);
}

function layerCountForPlayableRole(item: AvatarItem, playableRoleKey: string) {
  if (playableRoleKey === "all") return item.layer_count;
  const visualFamily = visualFamilyForPlayableRole(playableRoleKey);
  if (!visualFamily) return 0;
  return item.layer_counts_by_visual_family[visualFamily.visual_family_key] ?? 0;
}

function previewTitle() {
  if (state.selectedSet) return cleanDisplayName(state.selectedSet.set_name) || "未知套装";
  if (state.mode === "items") {
    const names = slots
      .map((slot) => cleanDisplayName(state.outfit[slot]?.item_name ?? ""))
      .filter(Boolean);
    return names.length ? `自由搭配（${names.length}件）` : (cleanDisplayName(state.selectedItem?.item_name ?? "") || "未选择");
  }
  return cleanDisplayName(state.selectedItem?.item_name ?? "") || "未选择";
}

function renderPreviewHeading() {
  document.querySelector("#previewTitle")!.textContent = previewTitle();
  document.querySelector("#previewKind")!.textContent = state.selectedSet
    ? "套装合成"
    : state.mode === "items"
      ? "自由搭配"
      : "未选择";
}

function imgPathForDisplay(path: string) {
  const normalized = path.trim().replace(/\\/g, "/").replace(/^\/+/, "").toLowerCase();
  if (normalized.startsWith("sprite/")) return normalized;
  if (normalized.startsWith("character/")) return `sprite/${normalized}`;
  return normalized;
}

function isHiddenName(value: string) {
  return !cleanDisplayName(value);
}

function cleanDisplayName(value: string) {
  let name = value.trim();
  while (true) {
    const index = name.lastIndexOf(" - ");
    if (index < 0) break;
    const suffix = name.slice(index + 3).trim();
    if (!isInternalToken(suffix)) break;
    name = name.slice(0, index).trim();
  }
  if (!name || /^\d+$/.test(name) || /^name_\d+(?:\s|$|-)/i.test(name) || isInternalToken(name) || name.startsWith("<")) {
    return "";
  }
  return name;
}

function isInternalToken(value: string) {
  const token = value.trim().replace(/^</, "").replace(/>$/, "");
  return /^\d+::[A-Za-z0-9_-]+$/.test(token);
}

function sanitizeFileName(value: string) {
  return value.replace(/[\\/:*?"<>|]/g, "_").slice(0, 80) || "avatar";
}

function sanitizeAscii(value: string) {
  return value.replace(/[^a-zA-Z0-9_-]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 64);
}

function escapeHtml(value: string) {
  return value.replace(/[&<>"']/g, (char) => {
    const entities: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    };
    return entities[char];
  });
}

void boot();

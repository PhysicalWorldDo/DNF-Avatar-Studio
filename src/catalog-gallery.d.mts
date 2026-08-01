export type GalleryMode = "sets" | "items";

export type GallerySingleGateInput = {
  mode: GalleryMode;
  profession: string;
  slot: string;
};

export type GallerySingleGateResult = {
  allowed: boolean;
  missing: Array<"mode" | "profession" | "slot">;
};

export type GalleryAlias = {
  entity_id: string;
  name: string;
  source_order: number;
};

export type GalleryWireCard = {
  entity_kind: "set" | "item";
  entity_id: string;
  entity_recipe_key: string;
  name: string;
  slot: string;
  visual_signature: string;
  source_order: number;
  thumbnail_status: "missing" | "ready" | "queued" | "rendering" | "stale" | "failed";
  thumbnail_webp_base64: string | null;
  error: string | null;
};

export type GalleryCard = Omit<GalleryWireCard, "thumbnail_webp_base64" | "error"> & {
  thumbnail_webp_base64: string;
  thumbnail_object_url?: string;
  error: string;
  aliases?: GalleryAlias[];
  duplicate_count?: number;
};

export function gallerySingleGate(input: GallerySingleGateInput): GallerySingleGateResult;

export function mergeGalleryCards(
  existingCards: readonly GalleryCard[],
  incomingCards: readonly GalleryCard[],
  options?: { selectedEntityId?: string; query?: string },
): GalleryCard[];

export const SET_GALLERY_PAGE_SIZE: number;

export type GalleryPageWindow = {
  page: number;
  pageSize: number;
  loadedPageCount: number;
  startIndex: number;
  endIndex: number;
  cards: GalleryCard[];
  hasPrevious: boolean;
  hasNext: boolean;
};

export function galleryPageWindow(
  cards: readonly GalleryCard[],
  requestedPage: number,
  pageSize?: number,
  canLoadMore?: boolean,
): GalleryPageWindow;

export type GalleryRequestGate = {
  issue(): number;
  accepts(ticket: number): boolean;
  invalidate(): void;
};

export class GalleryThumbnailCancellationError extends Error {}

export type GalleryThumbnailScheduler = {
  schedule<T>(
    key: string,
    task: () => T | Promise<T>,
    context?: string,
  ): Promise<T> | null;
  activateContext(context: string): number;
  has(key: string): boolean;
  activeCount(): number;
  queuedCount(): number;
};

export function createGalleryRequestGate(): GalleryRequestGate;
export function isGalleryThumbnailCancellation(error: unknown): boolean;
export function createGalleryThumbnailScheduler(
  limit?: number,
  queueLimit?: number,
): GalleryThumbnailScheduler;

export type GallerySnapshotWarmupOptions = {
  concurrency?: number;
  createObjectUrl(base64: string, card: GalleryCard): string;
  decodeObjectUrl(objectUrl: string, card: GalleryCard): Promise<void>;
  revokeObjectUrl(objectUrl: string): void;
  onProgress?(completed: number, total: number): void;
  isCurrent?(): boolean;
};

export type GalleryMemorySnapshotPage = {
  page: number;
  cards: readonly GalleryCard[];
  html: string;
};

export type GalleryMemorySnapshot = {
  contextKey: string;
  cards: readonly GalleryCard[];
  pageSize: number;
  pages: readonly GalleryMemorySnapshotPage[];
};

export function warmGallerySnapshotCards(
  cards: readonly GalleryCard[],
  options: GallerySnapshotWarmupOptions,
): Promise<GalleryCard[]>;
export function createGalleryMemorySnapshot(
  contextKey: string,
  cards: readonly GalleryCard[],
  pageSize?: number,
  renderCard?: (card: GalleryCard) => string,
): GalleryMemorySnapshot;
export function disposeGalleryMemorySnapshot(
  snapshot: GalleryMemorySnapshot | null,
  revokeObjectUrl: (objectUrl: string) => void,
): void;
export function batchActionLabel(mode: GalleryMode): string;

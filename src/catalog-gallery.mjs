export function gallerySingleGate({ mode, profession, slot }) {
  const missing = [];
  if (mode !== "items") missing.push("mode");
  if (!profession || profession === "all") missing.push("profession");
  if (!slot || slot === "all") missing.push("slot");
  return Object.freeze({ allowed: missing.length === 0, missing });
}

function normalizedQuery(value) {
  return String(value ?? "").trim().toLocaleLowerCase();
}

const GALLERY_PLACEHOLDER_PREFIXES = Object.freeze([
  "未知",
  "未命名",
  "待确认",
  "后续确认",
  "待补充",
  "暂未命名",
  "名称待定",
  "待定",
]);

function hasMeaningfulGalleryName(value) {
  const name = String(value ?? "").trim();
  if (
    !name
    || (
      name !== "未知套装"
      && GALLERY_PLACEHOLDER_PREFIXES.some((prefix) => name.startsWith(prefix))
    )
    || name.startsWith("((")
    || name.startsWith("（（")
    || name.startsWith("<")
    || /^name_/iu.test(name)
    || /^\d+$/u.test(name)
  ) {
    return false;
  }
  return /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/u.test(name);
}

function stableMemberOrder(left, right) {
  const sourceOrder = Number(left.source_order) - Number(right.source_order);
  if (Number.isFinite(sourceOrder) && sourceOrder !== 0) return sourceOrder;
  return String(left.entity_id).localeCompare(String(right.entity_id));
}

function cardMember(card, alias = card) {
  const {
    aliases: _aliases,
    duplicate_count: _duplicateCount,
    ...base
  } = card;
  return {
    ...base,
    entity_id: String(alias.entity_id),
    name: String(alias.name),
    source_order: Number(alias.source_order),
  };
}

function membersFromCard(card) {
  if (!Array.isArray(card.aliases) || card.aliases.length === 0) {
    return [cardMember(card)];
  }
  return card.aliases.map((alias) => cardMember(card, alias));
}

function representativeForMembers(members, { selectedEntityId = "", query = "" }) {
  const ordered = [...members]
    .filter((entry) => hasMeaningfulGalleryName(entry.name))
    .sort(stableMemberOrder);
  const selected = ordered.find((entry) => entry.entity_id === selectedEntityId);
  if (selected) return selected;
  const wanted = normalizedQuery(query);
  if (wanted) {
    const matching = ordered.find((entry) => normalizedQuery(entry.name).includes(wanted));
    if (matching) return matching;
  }
  return ordered[0];
}

function thumbnailStateRank(member) {
  if (member.thumbnail_status === "ready" && member.thumbnail_webp_base64) return 6;
  return {
    rendering: 5,
    queued: 4,
    stale: 3,
    missing: 2,
    failed: 1,
    ready: 0,
  }[member.thumbnail_status] ?? 0;
}

function previewForMembers(members, representative) {
  return [...members].sort((left, right) => {
    const rank = thumbnailStateRank(right) - thumbnailStateRank(left);
    return rank || stableMemberOrder(left, right);
  })[0] ?? representative;
}

export function mergeGalleryCards(existingCards, incomingCards, options = {}) {
  const groups = new Map();
  for (const card of [...existingCards, ...incomingCards]) {
    const signature = String(card.visual_signature ?? "");
    if (!signature) continue;
    const group = groups.get(signature) ?? new Map();
    for (const member of membersFromCard(card)) {
      const memberKey = `${member.entity_kind}:${member.entity_id}`;
      const previous = group.get(memberKey);
      if (!previous || previous.thumbnail_status !== "ready") {
        group.set(memberKey, member);
      }
    }
    groups.set(signature, group);
  }

  return [...groups.entries()]
    .map(([visualSignature, membersById]) => {
      const members = [...membersById.values()]
        .filter((member) => member.thumbnail_status !== "failed")
        .sort(stableMemberOrder);
      const representative = representativeForMembers(members, options);
      if (!representative) return null;
      const preview = previewForMembers(members, representative);
      return {
        ...representative,
        entity_recipe_key: preview.entity_recipe_key,
        thumbnail_status: preview.thumbnail_status,
        thumbnail_webp_base64: preview.thumbnail_webp_base64,
        error: preview.error,
        source_order: members[0].source_order,
        visual_signature: visualSignature,
        aliases: members
          .filter((member) => hasMeaningfulGalleryName(member.name))
          .map(({ entity_id, name, source_order }) => ({
            entity_id,
            name,
            source_order,
          })),
        duplicate_count: members.length,
      };
    })
    .filter(Boolean)
    .sort(stableMemberOrder);
}

export const SET_GALLERY_PAGE_SIZE = 22;

export function galleryPageWindow(
  cards,
  requestedPage,
  pageSize = SET_GALLERY_PAGE_SIZE,
  canLoadMore = false,
) {
  const size = Math.max(1, Math.floor(Number(pageSize) || SET_GALLERY_PAGE_SIZE));
  const loadedPageCount = cards.length > 0 ? Math.ceil(cards.length / size) : 0;
  const page = loadedPageCount > 0
    ? Math.min(Math.max(1, Math.floor(Number(requestedPage) || 1)), loadedPageCount)
    : 1;
  const startIndex = (page - 1) * size;
  const visibleCards = cards.slice(startIndex, startIndex + size);
  const endIndex = startIndex + visibleCards.length;
  return Object.freeze({
    page,
    pageSize: size,
    loadedPageCount,
    startIndex,
    endIndex,
    cards: visibleCards,
    hasPrevious: page > 1,
    hasNext: endIndex < cards.length || Boolean(canLoadMore),
  });
}

export function createGalleryRequestGate() {
  let generation = 0;
  return Object.freeze({
    issue() {
      generation += 1;
      return generation;
    },
    accepts(ticket) {
      return ticket === generation;
    },
    invalidate() {
      generation += 1;
    },
  });
}

export class GalleryThumbnailCancellationError extends Error {
  constructor() {
    super("gallery thumbnail task was cancelled");
    this.name = "GalleryThumbnailCancellationError";
  }
}

export function isGalleryThumbnailCancellation(error) {
  return error instanceof GalleryThumbnailCancellationError
    || (
      typeof error === "object"
      && error !== null
      && error.name === "GalleryThumbnailCancellationError"
    );
}

export function createGalleryThumbnailScheduler(limit = 4, queueLimit = 64) {
  const concurrency = Math.max(1, Math.floor(Number(limit) || 1));
  const maxQueued = Math.max(1, Math.floor(Number(queueLimit) || 1));
  const pending = new Map();
  const queue = [];
  let active = 0;
  let activeContext = "";

  const drain = () => {
    while (active < concurrency && queue.length > 0) {
      const job = queue.shift();
      active += 1;
      Promise.resolve()
        .then(job.task)
        .then((value) => {
          active -= 1;
          if (pending.get(job.key) === job.promise) pending.delete(job.key);
          drain();
          job.resolve(value);
        }, (error) => {
          active -= 1;
          if (pending.get(job.key) === job.promise) pending.delete(job.key);
          drain();
          job.reject(error);
        });
    }
  };

  const cancelQueuedJob = (job) => {
    if (pending.get(job.key) === job.promise) pending.delete(job.key);
    job.reject(new GalleryThumbnailCancellationError());
  };

  return Object.freeze({
    schedule(key, task, context = activeContext) {
      const normalizedKey = String(key);
      const normalizedContext = String(context);
      if (!normalizedKey || typeof task !== "function") {
        throw new TypeError("thumbnail scheduler requires a key and task");
      }
      if (activeContext && normalizedContext !== activeContext) return null;
      const existing = pending.get(normalizedKey);
      if (existing) return existing;
      if (queue.length >= maxQueued) return null;

      let resolveTask;
      let rejectTask;
      const promise = new Promise((resolve, reject) => {
        resolveTask = resolve;
        rejectTask = reject;
      });
      pending.set(normalizedKey, promise);
      queue.push({
        key: normalizedKey,
        context: normalizedContext,
        task,
        promise,
        resolve: resolveTask,
        reject: rejectTask,
      });
      drain();
      return promise;
    },
    activateContext(context) {
      const normalizedContext = String(context);
      if (normalizedContext === activeContext) return 0;
      activeContext = normalizedContext;
      let cancelled = 0;
      for (let index = queue.length - 1; index >= 0; index -= 1) {
        const job = queue[index];
        if (job.context === activeContext) continue;
        queue.splice(index, 1);
        cancelQueuedJob(job);
        cancelled += 1;
      }
      return cancelled;
    },
    has(key) {
      return pending.has(String(key));
    },
    activeCount() {
      return active;
    },
    queuedCount() {
      return queue.length;
    },
  });
}

const disposedGallerySnapshots = new WeakSet();

export async function warmGallerySnapshotCards(cards, options) {
  const {
    concurrency = 6,
    createObjectUrl,
    decodeObjectUrl,
    revokeObjectUrl,
    onProgress = () => {},
    isCurrent = () => true,
  } = options ?? {};
  if (
    typeof createObjectUrl !== "function"
    || typeof decodeObjectUrl !== "function"
    || typeof revokeObjectUrl !== "function"
  ) {
    throw new TypeError("gallery snapshot warmup requires object URL lifecycle callbacks");
  }

  const workerCount = Math.max(1, Math.floor(Number(concurrency) || 1));
  const warmed = new Array(cards.length);
  const createdUrls = new Set();
  let cursor = 0;
  let completed = 0;
  let cancelled = false;

  const worker = async () => {
    while (!cancelled) {
      const index = cursor;
      cursor += 1;
      if (index >= cards.length) return;
      const card = cards[index];
      let objectUrl = String(card.thumbnail_object_url ?? "");
      let ownsUrl = false;
      try {
        if (
          !isCurrent()
          || card.thumbnail_status !== "ready"
          || (!objectUrl && !card.thumbnail_webp_base64)
        ) {
          if (!isCurrent()) cancelled = true;
          continue;
        }
        if (!objectUrl) {
          objectUrl = String(createObjectUrl(card.thumbnail_webp_base64, card));
          if (!objectUrl) throw new Error("gallery snapshot object URL is empty");
          ownsUrl = true;
          createdUrls.add(objectUrl);
        }
        await decodeObjectUrl(objectUrl, card);
        if (!isCurrent()) {
          cancelled = true;
          continue;
        }
        warmed[index] = {
          ...card,
          thumbnail_status: "ready",
          thumbnail_webp_base64: "",
          thumbnail_object_url: objectUrl,
          error: "",
        };
      } catch {
        if (objectUrl && (ownsUrl || card.thumbnail_object_url)) {
          revokeObjectUrl(objectUrl);
          createdUrls.delete(objectUrl);
        }
      } finally {
        completed += 1;
        onProgress(completed, cards.length);
      }
    }
  };

  await Promise.all(
    Array.from({ length: Math.min(workerCount, Math.max(1, cards.length)) }, worker),
  );
  if (cancelled || !isCurrent()) {
    for (const objectUrl of createdUrls) revokeObjectUrl(objectUrl);
    return [];
  }
  return warmed.filter(Boolean);
}

export function createGalleryMemorySnapshot(
  contextKey,
  cards,
  pageSize = SET_GALLERY_PAGE_SIZE,
  renderCard = () => "",
) {
  const size = Math.max(1, Math.floor(Number(pageSize) || SET_GALLERY_PAGE_SIZE));
  const readyCards = Object.freeze(cards.filter((card) => (
    card.thumbnail_status === "ready"
    && Boolean(card.thumbnail_object_url || card.thumbnail_webp_base64)
  )));
  const pages = [];
  for (let index = 0; index < readyCards.length; index += size) {
    const pageCards = Object.freeze(readyCards.slice(index, index + size));
    pages.push(Object.freeze({
      page: pages.length + 1,
      cards: pageCards,
      html: pageCards.map(renderCard).join(""),
    }));
  }
  return Object.freeze({
    contextKey: String(contextKey),
    cards: readyCards,
    pageSize: size,
    pages: Object.freeze(pages),
  });
}

export function disposeGalleryMemorySnapshot(snapshot, revokeObjectUrl) {
  if (
    !snapshot
    || disposedGallerySnapshots.has(snapshot)
    || typeof revokeObjectUrl !== "function"
  ) return;
  disposedGallerySnapshots.add(snapshot);
  const released = new Set();
  for (const card of snapshot.cards ?? []) {
    const objectUrl = String(card.thumbnail_object_url ?? "");
    if (!objectUrl || released.has(objectUrl)) continue;
    released.add(objectUrl);
    revokeObjectUrl(objectUrl);
  }
}

export function batchActionLabel(mode) {
  return mode === "sets" ? "生成本职业套装" : "生成本职业全部单件";
}

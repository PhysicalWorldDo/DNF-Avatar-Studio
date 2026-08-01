const PREVIEW_KINDS = new Set(["current", "target"]);

export function createOutfitSwapPreviewGate() {
  const latestGeneration = {
    current: 0,
    target: 0,
  };

  return Object.freeze({
    issue(kind, pageRequestId, linearDodge) {
      if (!PREVIEW_KINDS.has(kind)) throw new TypeError("unknown outfit preview kind");
      if (!Number.isSafeInteger(pageRequestId) || pageRequestId < 0) {
        throw new TypeError("invalid outfit preview page request ID");
      }
      const generation = latestGeneration[kind] + 1;
      if (!Number.isSafeInteger(generation)) throw new RangeError("outfit preview generation exhausted");
      latestGeneration[kind] = generation;
      return Object.freeze({ kind, pageRequestId, linearDodge: Boolean(linearDodge), generation });
    },

    accepts(ticket, pageRequestId, linearDodge) {
      return Boolean(
        ticket
        && PREVIEW_KINDS.has(ticket.kind)
        && ticket.pageRequestId === pageRequestId
        && ticket.linearDodge === Boolean(linearDodge)
        && ticket.generation === latestGeneration[ticket.kind]
      );
    },
  });
}

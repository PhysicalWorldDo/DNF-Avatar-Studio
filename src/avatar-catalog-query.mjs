export const CATALOG_QUERY_DEBOUNCE_MS = 100;

export function projectSetMemberRecords(memberIds, records) {
  const wanted = new Set(memberIds.map(String));
  return records.filter((record) => wanted.has(String(record.item_id)));
}

function createGenerationGate() {
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

export function createCatalogSelectionGate() {
  const generationGate = createGenerationGate();
  let pendingTicket = null;

  return Object.freeze({
    issue() {
      const ticket = generationGate.issue();
      pendingTicket = ticket;
      return ticket;
    },

    accepts(ticket) {
      return generationGate.accepts(ticket);
    },

    finish(ticket) {
      if (pendingTicket !== ticket || !generationGate.accepts(ticket)) return false;
      pendingTicket = null;
      return true;
    },

    isPending() {
      return pendingTicket !== null && generationGate.accepts(pendingTicket);
    },

    invalidate() {
      pendingTicket = null;
      generationGate.invalidate();
    },
  });
}

export function createCatalogPreviewGate() {
  return createGenerationGate();
}

export function createLatestCatalogQuery({
  execute,
  onResult,
  onError,
  debounceMs = CATALOG_QUERY_DEBOUNCE_MS,
  schedule = setTimeout,
  cancel = clearTimeout,
}) {
  let generation = 0;
  let pendingTimer = null;

  function cancelPendingTimer() {
    if (pendingTimer === null) return;
    cancel(pendingTimer);
    pendingTimer = null;
  }

  async function executeIfLatest(requestGeneration, request) {
    try {
      const result = await execute(request);
      if (requestGeneration !== generation) return false;
      onResult(result, request);
      return true;
    } catch (error) {
      if (requestGeneration !== generation) return false;
      onError(error, request);
      return false;
    }
  }

  function immediate(request) {
    cancelPendingTimer();
    const requestGeneration = ++generation;
    return executeIfLatest(requestGeneration, request);
  }

  function debounced(request) {
    cancelPendingTimer();
    const requestGeneration = ++generation;
    pendingTimer = schedule(() => {
      pendingTimer = null;
      return executeIfLatest(requestGeneration, request);
    }, debounceMs);
    return requestGeneration;
  }

  function invalidate() {
    cancelPendingTimer();
    generation += 1;
  }

  return {
    immediate,
    debounced,
    invalidate,
  };
}

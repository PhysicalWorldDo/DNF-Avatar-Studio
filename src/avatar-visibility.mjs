function normalizedEnums(values) {
  if (!Array.isArray(values)) return [];
  return [...new Set(values.map(Number).filter(Number.isInteger))];
}

export function resolveAvatarLayerVisibility(layers) {
  const sourcesByHiddenType = new Map();
  for (const layer of layers) {
    const source = {
      itemId: String(layer.item_id),
      itemName: String(layer.item_name ?? ""),
      slot: String(layer.slot ?? ""),
    };
    for (const hiddenType of normalizedEnums(layer.hide_equipment_type_enums)) {
      let sources = sourcesByHiddenType.get(hiddenType);
      if (!sources) {
        sources = new Map();
        sourcesByHiddenType.set(hiddenType, sources);
      }
      if (!sources.has(source.itemId)) sources.set(source.itemId, source);
    }
  }

  return layers.map((layer) => {
    const targetItemId = String(layer.item_id);
    const hiddenBy = [...(sourcesByHiddenType.get(Number(layer.equipment_type_enum))?.values() ?? [])]
      .filter((source) => source.itemId !== targetItemId);
    return {
      layer,
      visible: hiddenBy.length === 0,
      hiddenBy,
    };
  });
}

export function visibleAvatarLayers(layers) {
  return resolveAvatarLayerVisibility(layers)
    .filter((entry) => entry.visible)
    .map((entry) => entry.layer);
}

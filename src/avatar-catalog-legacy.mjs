export const ITEM_RESULT_LIMIT = 800;
export const SET_RESULT_LIMIT = 500;

const studioSlots = new Set([
  "skin",
  "cap",
  "hair",
  "face",
  "neck",
  "coat",
  "belt",
  "pants",
  "shoes",
  "weapon",
]);

function isInternalToken(value) {
  const token = String(value ?? "").trim().replace(/^</, "").replace(/>$/, "");
  return /^\d+::[A-Za-z0-9_-]+$/.test(token);
}

export function cleanLegacyDisplayName(value) {
  let name = String(value ?? "").trim();
  while (true) {
    const index = name.lastIndexOf(" - ");
    if (index < 0) break;
    const suffix = name.slice(index + 3).trim();
    if (!isInternalToken(suffix)) break;
    name = name.slice(0, index).trim();
  }
  if (
    !name
    || /^\d+$/.test(name)
    || /^name_\d+(?:\s|$|-)/i.test(name)
    || isInternalToken(name)
    || name.startsWith("<")
  ) {
    return "";
  }
  return name;
}

function normalizedPlayableRoles(record) {
  return [...new Set(
    (record.playable_roles ?? [])
      .map((key) => String(key).trim().toLowerCase())
      .filter((key) => key && key !== "all"),
  )];
}

function studioSlotForItem(item) {
  const resourceKind = String(item.resource_kind ?? "").trim().toLowerCase();
  if (resourceKind === "weapon" || resourceKind === "weapon_avatar") return "weapon";
  const slot = String(item.slot ?? "");
  return studioSlots.has(slot) ? slot : null;
}

export function legacyFilterAvatarItems(
  items,
  { selectedSlot, selectedProfession, query, slotLabels },
) {
  const q = String(query ?? "").toLowerCase();
  return items
    .filter((item) => {
      const itemName = cleanLegacyDisplayName(item.item_name);
      if (!itemName) return false;
      const studioSlot = studioSlotForItem(item);
      if (selectedSlot !== "all" && studioSlot !== selectedSlot) return false;
      if (
        selectedProfession !== "all"
        && !normalizedPlayableRoles(item).includes(selectedProfession)
      ) {
        return false;
      }
      if (!q) return true;
      const displaySlot = slotLabels[studioSlot ?? item.slot] ?? item.slot;
      return itemName.toLowerCase().includes(q) || String(displaySlot).toLowerCase().includes(q);
    })
    .slice(0, ITEM_RESULT_LIMIT);
}

export function legacyFilterAvatarSets(sets, { selectedProfession, query }) {
  const q = String(query ?? "").toLowerCase();
  return sets
    .filter((set) => {
      const setName = cleanLegacyDisplayName(set.set_name);
      if (!setName) return false;
      if (
        selectedProfession !== "all"
        && !normalizedPlayableRoles(set).includes(selectedProfession)
      ) {
        return false;
      }
      const memberNames = (set.member_names ?? [])
        .map(cleanLegacyDisplayName)
        .filter(Boolean);
      if (!q) return true;
      return (
        setName.toLowerCase().includes(q)
        || memberNames.some((name) => name.toLowerCase().includes(q))
      );
    })
    .slice(0, SET_RESULT_LIMIT);
}

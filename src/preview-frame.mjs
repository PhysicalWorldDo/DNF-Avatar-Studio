const PROFESSION_PREVIEW_START_FRAMES = new Map([
  ["thief", 10],
  ["gunner", 18],
  ["at_gunner", 6],
  ["swordman", 95],
]);

export function previewStartFrameForProfession(playableRoleKey) {
  const role = String(playableRoleKey ?? "").trim().toLowerCase();
  return PROFESSION_PREVIEW_START_FRAMES.get(role) ?? 0;
}

export function previewFrameIndexForProfession(playableRoleKey, frameCount) {
  const count = Number.isSafeInteger(frameCount) && frameCount > 0 ? frameCount : 0;
  return count > 0 ? previewStartFrameForProfession(playableRoleKey) % count : 0;
}

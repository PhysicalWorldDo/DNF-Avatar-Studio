export type AvatarVisibilitySource = {
  itemId: string;
  itemName: string;
  slot: string;
};

export type AvatarVisibilityLayer = {
  item_id: string;
  item_name?: string;
  slot?: string;
  equipment_type_enum: number;
  hide_equipment_type_enums?: readonly number[];
};

export type ResolvedAvatarLayerVisibility<T extends AvatarVisibilityLayer> = {
  layer: T;
  visible: boolean;
  hiddenBy: AvatarVisibilitySource[];
};

export function resolveAvatarLayerVisibility<T extends AvatarVisibilityLayer>(
  layers: readonly T[],
): ResolvedAvatarLayerVisibility<T>[];

export function visibleAvatarLayers<T extends AvatarVisibilityLayer>(layers: readonly T[]): T[];

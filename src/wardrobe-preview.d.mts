export type OpaqueBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type PreviewPlacement = {
  sourceX: number;
  sourceY: number;
  sourceWidth: number;
  sourceHeight: number;
  x: number;
  y: number;
  width: number;
  height: number;
};

export function opaquePixelBounds(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
): OpaqueBounds | null;

export function centeredPreviewPlacement(
  bounds: OpaqueBounds,
  targetWidth: number,
  targetHeight: number,
  options: { padding: number; maxWidth: number; maxHeight: number },
): PreviewPlacement;

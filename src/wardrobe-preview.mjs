export function opaquePixelBounds(rgba, width, height) {
  let left = width;
  let top = height;
  let right = -1;
  let bottom = -1;
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      if (rgba[(y * width + x) * 4 + 3] === 0) continue;
      left = Math.min(left, x);
      top = Math.min(top, y);
      right = Math.max(right, x);
      bottom = Math.max(bottom, y);
    }
  }
  return right < left || bottom < top
    ? null
    : { x: left, y: top, width: right - left + 1, height: bottom - top + 1 };
}

export function centeredPreviewPlacement(bounds, targetWidth, targetHeight, options) {
  const availableWidth = Math.max(1, Math.min(options.maxWidth, targetWidth - options.padding * 2));
  const availableHeight = Math.max(1, Math.min(options.maxHeight, targetHeight - options.padding * 2));
  const scale = Math.min(availableWidth / bounds.width, availableHeight / bounds.height);
  const width = Math.max(1, Math.round(bounds.width * scale));
  const height = Math.max(1, Math.round(bounds.height * scale));
  return {
    sourceX: bounds.x,
    sourceY: bounds.y,
    sourceWidth: bounds.width,
    sourceHeight: bounds.height,
    x: Math.round((targetWidth - width) / 2),
    y: Math.round((targetHeight - height) / 2),
    width,
    height,
  };
}

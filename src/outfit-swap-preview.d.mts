export type OutfitSwapPreviewKind = "current" | "target";

export type OutfitSwapPreviewTicket = Readonly<{
  kind: OutfitSwapPreviewKind;
  pageRequestId: number;
  linearDodge: boolean;
  generation: number;
}>;

export type OutfitSwapPreviewGate = Readonly<{
  issue(
    kind: OutfitSwapPreviewKind,
    pageRequestId: number,
    linearDodge: boolean,
  ): OutfitSwapPreviewTicket;
  accepts(
    ticket: OutfitSwapPreviewTicket,
    pageRequestId: number,
    linearDodge: boolean,
  ): boolean;
}>;

export function createOutfitSwapPreviewGate(): OutfitSwapPreviewGate;

export const CATALOG_QUERY_DEBOUNCE_MS: 100;

export function projectSetMemberRecords<Record extends { item_id: string }>(
  memberIds: readonly string[],
  records: readonly Record[],
): Record[];

export type CatalogGenerationGate = {
  issue(): number;
  accepts(ticket: number): boolean;
  invalidate(): void;
};

export type CatalogSelectionGate = CatalogGenerationGate & {
  finish(ticket: number): boolean;
  isPending(): boolean;
};

export function createCatalogSelectionGate(): CatalogSelectionGate;
export function createCatalogPreviewGate(): CatalogGenerationGate;

export type LatestCatalogQueryOptions<Request, Result> = {
  execute(request: Request): Promise<Result>;
  onResult(result: Result, request: Request): void;
  onError(error: unknown, request: Request): void;
  debounceMs?: number;
  schedule?: (
    callback: () => void | Promise<boolean>,
    delay: number,
  ) => ReturnType<typeof setTimeout>;
  cancel?: (timer: ReturnType<typeof setTimeout>) => void;
};

export type LatestCatalogQuery<Request> = {
  immediate(request: Request): Promise<boolean>;
  debounced(request: Request): number;
  invalidate(): void;
};

export function createLatestCatalogQuery<Request, Result>(
  options: LatestCatalogQueryOptions<Request, Result>,
): LatestCatalogQuery<Request>;

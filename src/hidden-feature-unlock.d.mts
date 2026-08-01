export type HiddenFeature = "wardrobe_swap";

export type HiddenFeatureUnlockResult<Config> = {
  matched: boolean;
  feature: HiddenFeature | null;
  already_unlocked: boolean;
  config: Config;
};

export type HiddenFeatureUnlockController<Config> = {
  submit(phrase: string): Promise<HiddenFeatureUnlockResult<Config> | null>;
};

export function shouldSubmitHiddenFeatureUnlock(event: {
  key: string;
  isComposing: boolean;
}): boolean;

export function verifiedFeatureVisibility(config: {
  wardrobe_swap_unlocked: boolean;
}): {
  wardrobeSwapEntryVisible: boolean;
};

export function hasInvalidHiddenFeatureCredential(config: {
  wardrobe_swap_credential_invalid: boolean;
}): boolean;

export function createHiddenFeatureUnlockController<Config>(dependencies: {
  invoke(
    command: "unlock_hidden_feature",
    arguments_: { phrase: string },
  ): Promise<HiddenFeatureUnlockResult<Config>>;
  getInputValue(): string;
  clearInput(): void;
  applyConfig(config: Config): void;
  onMatched(result: HiddenFeatureUnlockResult<Config>): void | Promise<void>;
  onError(error: unknown): void;
}): HiddenFeatureUnlockController<Config>;

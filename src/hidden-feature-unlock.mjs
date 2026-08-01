export function shouldSubmitHiddenFeatureUnlock(event) {
  return event.key === "Enter" && !event.isComposing;
}

export function verifiedFeatureVisibility(config) {
  return {
    wardrobeSwapEntryVisible: config.wardrobe_swap_unlocked === true,
  };
}

export function hasInvalidHiddenFeatureCredential(config) {
  return config.wardrobe_swap_credential_invalid === true;
}

export function createHiddenFeatureUnlockController(dependencies) {
  let queue = Promise.resolve();

  async function run(submittedPhrase) {
    try {
      const result = await dependencies.invoke("unlock_hidden_feature", {
        phrase: submittedPhrase,
      });
      if (!result.matched) return result;

      dependencies.applyConfig(result.config);
      if (dependencies.getInputValue() === submittedPhrase) {
        dependencies.clearInput();
      }
      await dependencies.onMatched(result);
      return result;
    } catch (error) {
      dependencies.onError(error);
      return null;
    }
  }

  return {
    submit(phrase) {
      const operation = queue.then(() => run(String(phrase)));
      queue = operation.then(
        () => undefined,
        () => undefined,
      );
      return operation;
    },
  };
}

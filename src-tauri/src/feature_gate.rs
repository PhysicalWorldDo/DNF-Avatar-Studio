use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DIGEST_DOMAIN: &[u8] = b"dnf-avatar-studio:hidden-feature:v1";
const PROOF_VERSION: u8 = 1;
const MAX_PHRASE_BYTES: usize = 256;
const MAX_PROOF_TEXT_BYTES: usize = 16 * 1024;
const MAX_PROOF_PAYLOAD_BYTES: usize = 4 * 1024;

const WARDROBE_SWAP_DIGEST: [u8; 32] = [
    0x09, 0x08, 0xa5, 0xab, 0x37, 0x51, 0x27, 0x45, 0x31, 0xc1, 0x18, 0x4d, 0x55, 0x2c, 0x40, 0xe6,
    0x07, 0x8b, 0x09, 0xa9, 0xb3, 0xa0, 0x04, 0x32, 0xcb, 0xba, 0x6a, 0x1d, 0xa6, 0x4b, 0xff, 0x32,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenFeature {
    WardrobeSwap,
}

impl HiddenFeature {
    pub fn id(self) -> &'static str {
        match self {
            Self::WardrobeSwap => "wardrobe_swap",
        }
    }

    fn proof_purpose(self) -> &'static [u8] {
        match self {
            Self::WardrobeSwap => b"dnf-avatar-studio:hidden-feature-proof:v1:wardrobe-swap",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeatureGate {
    wardrobe_swap_digest: [u8; 32],
}

impl FeatureGate {
    pub fn production() -> Self {
        Self::new(WARDROBE_SWAP_DIGEST)
    }

    pub(crate) fn new(wardrobe_swap_digest: [u8; 32]) -> Self {
        Self {
            wardrobe_swap_digest,
        }
    }

    pub fn feature_for_phrase(&self, phrase: &str) -> Option<HiddenFeature> {
        let normalized = normalized_phrase(phrase)?;
        self.normalized_phrase_matches(HiddenFeature::WardrobeSwap, normalized)
            .then_some(HiddenFeature::WardrobeSwap)
    }

    pub fn issue_proof(&self, feature: HiddenFeature, phrase: &str) -> Result<String, String> {
        let normalized = normalized_phrase(phrase).ok_or_else(invalid_phrase_error)?;
        if !self.normalized_phrase_matches(feature, normalized) {
            return Err(invalid_phrase_error());
        }
        let payload = FeatureUnlockPayload {
            version: PROOF_VERSION,
            feature_id: feature.id().to_string(),
            normalized_phrase: normalized.to_string(),
        };
        let encoded =
            postcard::to_allocvec(&payload).map_err(|_| "生成隐藏功能激活凭据失败".to_string())?;
        let encrypted =
            crate::local_protection::protect_for_current_user(&encoded, feature.proof_purpose())?;
        Ok(STANDARD.encode(encrypted))
    }

    pub fn proof_is_valid(&self, feature: HiddenFeature, proof: &str) -> bool {
        if proof.is_empty() || proof.len() > MAX_PROOF_TEXT_BYTES {
            return false;
        }
        let Ok(encrypted) = STANDARD.decode(proof) else {
            return false;
        };
        let Ok(plain) = crate::local_protection::unprotect_for_current_user(
            &encrypted,
            feature.proof_purpose(),
        ) else {
            return false;
        };
        if plain.len() > MAX_PROOF_PAYLOAD_BYTES {
            return false;
        }
        let Ok(payload) = postcard::from_bytes::<FeatureUnlockPayload>(&plain) else {
            return false;
        };
        if payload.version != PROOF_VERSION || payload.feature_id != feature.id() {
            return false;
        }
        let Some(normalized) = normalized_phrase(&payload.normalized_phrase) else {
            return false;
        };
        normalized.len() == payload.normalized_phrase.len()
            && self.normalized_phrase_matches(feature, normalized)
    }

    fn normalized_phrase_matches(&self, feature: HiddenFeature, phrase: &str) -> bool {
        let Some(actual) = phrase_digest(feature, phrase) else {
            return false;
        };
        constant_time_eq(&actual, self.expected_digest(feature))
    }

    fn expected_digest(&self, feature: HiddenFeature) -> &[u8; 32] {
        match feature {
            HiddenFeature::WardrobeSwap => &self.wardrobe_swap_digest,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FeatureUnlockPayload {
    version: u8,
    feature_id: String,
    normalized_phrase: String,
}

pub(crate) fn phrase_digest(feature: HiddenFeature, phrase: &str) -> Option<[u8; 32]> {
    let normalized = normalized_phrase(phrase)?;
    let mut hasher = Sha256::new();
    hasher.update(DIGEST_DOMAIN);
    hasher.update([0]);
    hasher.update(feature.id().as_bytes());
    hasher.update([0]);
    hasher.update(normalized.as_bytes());
    Some(hasher.finalize().into())
}

fn normalized_phrase(phrase: &str) -> Option<&str> {
    let normalized = phrase.trim();
    if normalized.is_empty() || normalized.len() > MAX_PHRASE_BYTES {
        return None;
    }
    Some(normalized)
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut different = 0u8;
    for index in 0..left.len() {
        different |= left[index] ^ right[index];
    }
    different == 0
}

fn invalid_phrase_error() -> String {
    "隐藏功能口令无效".to_string()
}

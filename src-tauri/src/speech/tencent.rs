use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::time::{SystemTime, UNIX_EPOCH};
use urlencoding::encode;
use uuid::Uuid;

type HmacSha1 = Hmac<Sha1>;

/// Generate a Tencent Cloud V1 HMAC-SHA1 signature for ASR.
///
/// Sorts the params alphabetically (excluding appid), builds the sign string in the format
/// `asr.cloud.tencent.com/asr/v2/{appid}?key1=val1&key2=val2&...`
/// (RAW params, NOT URL-encoded), then HMAC-SHA1 signs with secret_key
/// and base64 encodes the result.
pub fn generate_signature(secret_key: &str, app_id: &str, params: &[(&str, &str)]) -> String {
    let mut sorted: Vec<(&str, &str)> = params.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    // Build sign string with RAW params (NOT URL-encoded in V1)
    let query_parts: Vec<String> = sorted
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    let sign_str = format!(
        "asr.cloud.tencent.com/asr/v2/{}?{}",
        app_id,
        query_parts.join("&")
    );

    // HMAC-SHA1 sign
    let mut mac =
        HmacSha1::new_from_slice(secret_key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(sign_str.as_bytes());
    let result = mac.finalize();
    let code = result.into_bytes();

    // Base64 encode the signature
    base64::engine::general_purpose::STANDARD.encode(code)
}

/// Build a full ASR V2 URL with the V1 HMAC-SHA1 signature.
///
/// Generates a timestamp, nonce, expiry (24h), builds the param list,
/// creates the signature via [`generate_signature`], and assembles the
/// final `https://asr.cloud.tencent.com/asr/v2/{appid}?{params}&signature={sig}` URL.
pub fn build_asr_url(
    app_id: &str,
    secret_id: &str,
    secret_key: &str,
    engine_model: &str,
    audio_format: u8,
    need_vad: bool,
) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expired = timestamp + 86400;
    let nonce = timestamp;
    let voice_id = Uuid::new_v4().to_string();

    let timestamp_str = timestamp.to_string();
    let expired_str = expired.to_string();
    let audio_format_str = audio_format.to_string();
    let nonce_str = nonce.to_string();

    // Params for signing and URL (appid is NOT in params — it goes in the URL path)
    let mut params: Vec<(&str, &str)> = vec![
        ("secretid", secret_id),
        ("timestamp", &timestamp_str),
        ("expired", &expired_str),
        ("nonce", &nonce_str),
        ("engine_model_type", engine_model),
        ("voice_id", &voice_id),
        ("voice_format", &audio_format_str),
        ("needvad", if need_vad { "1" } else { "0" }),
    ];

    // Generate signature (app_id passed separately, sorted internally)
    let signature = generate_signature(secret_key, app_id, &params);

    // Sort params for the URL query string
    params.sort_by(|a, b| a.0.cmp(b.0));

    let query_parts: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect();

    format!(
        "https://asr.cloud.tencent.com/asr/v2/{}?{}&signature={}",
        app_id,
        query_parts.join("&"),
        encode(&signature)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_signature_known() {
        let secret_key = "test_secret_key";
        let app_id = "test_app";
        let params = [
            ("timestamp", "1234567890"),
            ("secretid", "test_secret_id"),
        ];
        let sig = generate_signature(secret_key, app_id, &params);
        assert!(!sig.is_empty(), "signature should not be empty");
        assert!(sig.len() > 10, "signature should be a reasonable length");
    }

    #[test]
    fn test_generate_signature_deterministic() {
        let secret_key = "test_secret_key";
        let app_id = "test_app";
        let params = [("timestamp", "1234567890")];
        let sig1 = generate_signature(secret_key, app_id, &params);
        let sig2 = generate_signature(secret_key, app_id, &params);
        assert_eq!(sig1, sig2, "same inputs should produce same signature");
    }

    #[test]
    fn test_build_url_format() {
        let url = build_asr_url(
            "12345",
            "test_secret_id",
            "test_secret_key",
            "16k_zh",
            1,
            true,
        );
        assert!(
            url.starts_with("wss://") || url.starts_with("https://"),
            "URL should start with wss:// or https://, got: {}",
            url
        );
        assert!(
            url.contains("signature="),
            "URL should contain signature=, got: {}",
            url
        );
        assert!(
            url.contains("engine_model_type=16k_zh"),
            "URL should contain engine_model_type=16k_zh, got: {}",
            url
        );
    }
}

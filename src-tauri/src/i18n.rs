//! Minimal i18n — mirrors src-ui/src/i18n.ts provider keys.
//! Language controlled by config.language field.

pub fn provider_short(provider: &str, lang: &str) -> String {
    match provider {
        "tencent" if lang == "zh" => "腾讯云".into(),
        "tencent" => "Tencent Cloud".into(),
        "local" if lang == "zh" => "远程 STT".into(),
        "local" => "Remote STT".into(),
        "local_embedded" if lang == "zh" => "本地嵌入式".into(),
        "local_embedded" => "Local Embedded".into(),
        _ => provider.to_string(),
    }
}

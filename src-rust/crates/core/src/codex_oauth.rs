//! OpenAI Codex OAuth configuration and constants.
//!

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

/// OpenAI Codex OAuth client ID (shared with the OpenCode ecosystem).
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// OpenAI OAuth issuer base URL.
pub const CODEX_ISSUER: &str = "https://auth.openai.com";

/// OpenAI OAuth authorization endpoint
pub const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";

/// OpenAI OAuth token endpoint
pub const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Codex Responses API endpoint (used for inference after login)
pub const CODEX_API_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";

/// Local redirect URI for OAuth callback
pub const CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

/// OAuth callback port
pub const CODEX_OAUTH_PORT: u16 = 1455;

/// OAuth scopes requested from OpenAI
pub const CODEX_SCOPES: &str = "openid profile email offline_access";

/// Available Codex models
pub const CODEX_MODELS: &[(&str, &str)] = &[
    ("gpt-5.5", "GPT-5.5 (default)"),
    ("gpt-5.4", "GPT-5.4"),
    ("gpt-5.4-mini", "GPT-5.4 Mini"),
    ("gpt-5.3-codex", "GPT-5.3 Codex"),
    ("gpt-5.3-codex-spark", "GPT-5.3 Codex Spark"),
    ("gpt-5.2", "GPT-5.2"),
    ("gpt-5.2-codex", "GPT-5.2 Codex"),
    ("gpt-5.1-codex", "GPT-5.1 Codex"),
    ("gpt-5.1-codex-mini", "GPT-5.1 Codex Mini"),
    ("gpt-5.1-codex-max", "GPT-5.1 Codex Max"),
];

/// Default Codex model to use
pub const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";

/// A Codex model exposed by either Claurst's fallback table or native Codex's
/// `~/.codex/models_cache.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexModelEntry {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub context_window: Option<u32>,
    pub max_context_window: Option<u32>,
    pub effective_context_window_percent: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct NativeCodexModelsCache {
    #[serde(default)]
    models: Vec<NativeCodexModel>,
}

#[derive(Debug, Deserialize)]
struct NativeCodexModel {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    visibility: String,
    #[serde(default)]
    priority: i64,
    context_window: Option<u32>,
    max_context_window: Option<u32>,
    effective_context_window_percent: Option<u8>,
}

/// Context-window hints for the OAuth Codex models Claurst knows about when
/// native Codex has not fetched `models_cache.json` yet.
pub fn codex_model_context_window(model_id: &str) -> Option<u32> {
    match model_id {
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" | "gpt-5.3-codex" | "gpt-5.2" => {
            Some(272_000)
        }
        "gpt-5.3-codex-spark" => Some(128_000),
        _ => None,
    }
}

pub fn codex_model_max_context_window(model_id: &str) -> Option<u32> {
    match model_id {
        "gpt-5.4" => Some(1_000_000),
        other => codex_model_context_window(other),
    }
}

fn fallback_codex_models() -> Vec<CodexModelEntry> {
    CODEX_MODELS
        .iter()
        .map(|(id, name)| CodexModelEntry {
            id: (*id).to_string(),
            display_name: (*name).to_string(),
            description: "OAuth-backed Codex model".to_string(),
            context_window: codex_model_context_window(id),
            max_context_window: codex_model_max_context_window(id),
            effective_context_window_percent: Some(95),
        })
        .collect()
}

fn codex_home_dir() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
}

/// Native Codex model cache path, if the Codex CLI has fetched one.
pub fn native_codex_models_cache_path() -> Option<PathBuf> {
    codex_home_dir().map(|home| home.join("models_cache.json"))
}

fn parse_native_codex_models(text: &str) -> Vec<CodexModelEntry> {
    let Ok(mut cache) = serde_json::from_str::<NativeCodexModelsCache>(text) else {
        return Vec::new();
    };

    cache.models.sort_by_key(|model| model.priority);
    let mut seen = std::collections::HashSet::new();
    cache
        .models
        .into_iter()
        .filter(|model| !model.slug.is_empty())
        .filter(|model| model.visibility == "list")
        .filter(|model| seen.insert(model.slug.clone()))
        .map(|model| {
            let display_name = if model.display_name.is_empty() {
                model.slug.clone()
            } else {
                model.display_name
            };
            let description = if model.description.is_empty() {
                "Native Codex model".to_string()
            } else {
                model.description
            };
            CodexModelEntry {
                id: model.slug,
                display_name,
                description,
                context_window: model.context_window,
                max_context_window: model.max_context_window,
                effective_context_window_percent: model.effective_context_window_percent,
            }
        })
        .collect()
}

/// Available Codex models, preferring native Codex's current cache and falling
/// back to Claurst's baked-in list for first-run/offline use.
pub fn available_codex_models() -> Vec<CodexModelEntry> {
    let mut models = native_codex_models_cache_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_native_codex_models(&text))
        .unwrap_or_default();

    let mut seen: std::collections::HashSet<String> =
        models.iter().map(|model| model.id.clone()).collect();
    for model in fallback_codex_models() {
        if seen.insert(model.id.clone()) {
            models.push(model);
        }
    }
    models
}

/// Extract the ChatGPT account identifier expected by the Codex backend.
///
/// OpenAI has used multiple JWT claim shapes across Codex-compatible clients,
/// so keep this intentionally tolerant and prefer explicit ChatGPT account
/// claims before falling back to organization ids.
pub fn extract_account_id_from_jwt(token: &str) -> Option<String> {
    let payload_b64 = token.split('.').nth(1)?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let json: Value = serde_json::from_slice(&payload).ok()?;

    json["chatgpt_account_id"]
        .as_str()
        .or_else(|| json["https://api.openai.com/auth"]["chatgpt_account_id"].as_str())
        .or_else(|| json["https://api.openai.com/auth"]["account_id"].as_str())
        .or_else(|| json["https://api.openai.com/auth.chatgpt_account_id"].as_str())
        .or_else(|| json["https://api.openai.com/auth.account_id"].as_str())
        .or_else(|| {
            json["organizations"]
                .as_array()
                .and_then(|orgs| orgs.first())
                .and_then(|org| org["id"].as_str())
        })
        .map(str::to_owned)
}

/// Convert an OAuth `expires_in` value to an absolute Unix timestamp.
pub fn expires_at_from_now(expires_in_secs: Option<u64>) -> Option<u64> {
    expires_in_secs.map(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_add(secs)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_native_codex_context_window() {
        let json = r#"{
            "models": [{
                "slug": "gpt-5.5",
                "display_name": "GPT-5.5",
                "description": "frontier",
                "visibility": "list",
                "priority": 0,
                "context_window": 272000,
                "max_context_window": 272000,
                "effective_context_window_percent": 95
            }]
        }"#;

        let models = parse_native_codex_models(json);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-5.5");
        assert_eq!(models[0].context_window, Some(272_000));
        assert_eq!(models[0].max_context_window, Some(272_000));
        assert_eq!(models[0].effective_context_window_percent, Some(95));
    }

    #[test]
    fn test_codex_constants_not_empty() {
        assert!(!CODEX_CLIENT_ID.is_empty());
        assert!(!CODEX_AUTHORIZE_URL.is_empty());
        assert!(!CODEX_TOKEN_URL.is_empty());
        assert!(!CODEX_REDIRECT_URI.is_empty());
        assert!(!CODEX_SCOPES.is_empty());
        assert!(!CODEX_MODELS.is_empty());
        assert!(!DEFAULT_CODEX_MODEL.is_empty());
    }

    #[test]
    fn test_codex_models_contains_default() {
        let default_found = CODEX_MODELS
            .iter()
            .any(|(model, _)| model == &DEFAULT_CODEX_MODEL);
        assert!(
            default_found,
            "DEFAULT_CODEX_MODEL must be in CODEX_MODELS list"
        );
    }

    #[test]
    fn test_redirect_uri_is_localhost() {
        assert!(CODEX_REDIRECT_URI.contains("localhost:1455"));
    }

    #[test]
    fn test_extract_account_id_from_auth_claims() {
        let payload = r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct_123"}}"#;
        let token = format!("a.{}.c", URL_SAFE_NO_PAD.encode(payload.as_bytes()));

        assert_eq!(
            extract_account_id_from_jwt(&token).as_deref(),
            Some("acct_123")
        );
    }

    #[test]
    fn test_extract_account_id_from_organizations() {
        let payload = r#"{"organizations":[{"id":"org_123"}]}"#;
        let token = format!("a.{}.c", URL_SAFE_NO_PAD.encode(payload.as_bytes()));

        assert_eq!(
            extract_account_id_from_jwt(&token).as_deref(),
            Some("org_123")
        );
    }

    #[test]
    fn test_expires_at_from_now_sets_future_time() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expires_at = expires_at_from_now(Some(3600)).unwrap();

        assert!(expires_at >= now + 3599);
    }
    #[test]
    fn test_codex_default_tracks_native_frontier() {
        assert_eq!(DEFAULT_CODEX_MODEL, "gpt-5.5");
        assert_eq!(
            CODEX_MODELS.first().map(|(id, _)| *id),
            Some(DEFAULT_CODEX_MODEL)
        );
    }

    #[test]
    fn test_parse_native_codex_models_filters_hidden_and_sorts_by_priority() {
        let cache = r#"{
            "models": [
                {"slug":"hidden","display_name":"Hidden","description":"nope","visibility":"hidden","priority":0},
                {"slug":"gpt-5.4","display_name":"GPT-5.4","description":"older","visibility":"list","priority":20},
                {"slug":"gpt-5.5","display_name":"GPT-5.5","description":"frontier","visibility":"list","priority":10},
                {"slug":"gpt-5.5","display_name":"Duplicate","description":"dupe","visibility":"list","priority":30}
            ]
        }"#;

        let models = parse_native_codex_models(cache);
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["gpt-5.5", "gpt-5.4"]
        );
        assert_eq!(models[0].display_name, "GPT-5.5");
        assert_eq!(models[0].description, "frontier");
    }

}

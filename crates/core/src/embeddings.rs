//! Optional semantic stage.
//!
//! An embedding provider is used only when one actually answers. Nothing here degrades into a pretend model: when no provider is reachable the matcher runs lexically and says so, and no score is ever labelled AI on the strength of a deterministic rule.
//!
//! Only the lexical shortlist is embedded, never the whole corpus on every refresh, and every vector is cached by the content that produced it.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Db;

/// What the interface is allowed to claim about the ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SemanticStatus {
    /// A provider answered and reranking is live.
    Available { provider: String, model: String },
    /// A provider is configured but did not answer. The reason is shown rather than hidden.
    Unavailable { provider: String, reason: String },
    /// No provider is configured at all.
    Disabled,
}

impl SemanticStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// One line the interface can show verbatim. It never uses the word AI unless a model really ran.
    pub fn label(&self) -> String {
        match self {
            Self::Available { provider, model } => {
                format!("lexical ranking with semantic reranking by {provider} model {model}")
            }
            Self::Unavailable { provider, reason } => {
                format!("lexical ranking only: the {provider} embedding provider did not answer ({reason})")
            }
            Self::Disabled => "lexical ranking only: no embedding provider is configured".into(),
        }
    }
}

/// Reads an embedding provider from the environment. `EMBEDDINGS_PROVIDER=ollama` turns it on; anything else leaves it off.
#[derive(Debug, Clone)]
pub struct EmbeddingProvider {
    pub kind: String,
    pub model: String,
    endpoint: String,
    client: reqwest::Client,
}

impl EmbeddingProvider {
    pub fn from_env() -> Option<Self> {
        let kind = std::env::var("EMBEDDINGS_PROVIDER").unwrap_or_else(|_| "disabled".into());
        if kind != "ollama" {
            return None;
        }
        let endpoint = std::env::var("EMBEDDINGS_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".into());
        let model = std::env::var("EMBEDDINGS_MODEL").unwrap_or_else(|_| "nomic-embed-text".into());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?;
        Some(Self {
            kind,
            model,
            endpoint,
            client,
        })
    }

    /// Ask the provider for one vector. The caller treats any error as "no semantic stage this time" rather than as a failure of the search.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let url = format!("{}/api/embed", self.endpoint.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "model": self.model, "input": text }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("provider responded {}", response.status()));
        }
        let body: EmbedResponse = response.json().await.map_err(|e| e.to_string())?;
        body.into_vector()
            .ok_or_else(|| "response carried no embedding".into())
    }

    /// One cheap call that decides whether the semantic stage runs at all.
    pub async fn status(&self) -> SemanticStatus {
        match self.embed("availability probe").await {
            Ok(vector) if !vector.is_empty() => SemanticStatus::Available {
                provider: self.kind.clone(),
                model: self.model.clone(),
            },
            Ok(_) => SemanticStatus::Unavailable {
                provider: self.kind.clone(),
                reason: "provider returned an empty vector".into(),
            },
            Err(reason) => SemanticStatus::Unavailable {
                provider: self.kind.clone(),
                reason,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    #[serde(default)]
    embeddings: Option<Vec<Vec<f32>>>,
    #[serde(default)]
    embedding: Option<Vec<f32>>,
}

impl EmbedResponse {
    fn into_vector(self) -> Option<Vec<f32>> {
        self.embeddings
            .and_then(|mut v| (!v.is_empty()).then(|| v.remove(0)))
            .or(self.embedding)
    }
}

/// Cosine similarity, or `None` when the vectors cannot be compared. Returning `None` rather than zero keeps an incomparable pair out of the ranking instead of scoring it as a poor match.
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }
    Some((dot / (norm_a * norm_b)).clamp(-1.0, 1.0))
}

#[derive(Debug, FromRow)]
struct CachedVector {
    content_hash: String,
    vector: Vec<f32>,
}

/// Fetch a cached vector, but only if it was computed from the same content by the same model.
pub async fn cached(
    pool: &Db,
    opportunity_id: Uuid,
    model: &str,
    content_hash: &str,
) -> Option<Vec<f32>> {
    let row = sqlx::query_as::<_, CachedVector>(
        "select content_hash, vector from opportunity_embeddings where opportunity_id = $1 and model = $2",
    )
    .bind(opportunity_id)
    .bind(model)
    .fetch_optional(pool)
    .await
    .ok()??;
    (row.content_hash == content_hash).then_some(row.vector)
}

pub async fn store(
    pool: &Db,
    opportunity_id: Uuid,
    model: &str,
    content_hash: &str,
    vector: &[f32],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into opportunity_embeddings (opportunity_id, model, content_hash, vector)
         values ($1,$2,$3,$4)
         on conflict (opportunity_id, model)
         do update set content_hash = excluded.content_hash, vector = excluded.vector, created_at = now()",
    )
    .bind(opportunity_id)
    .bind(model)
    .bind(content_hash)
    .bind(vector)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_vectors_is_one_and_of_opposites_is_minus_one() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert_eq!(cosine(&[1.0, 0.0], &[-1.0, 0.0]), Some(-1.0));
        let orthogonal = cosine(&[1.0, 0.0], &[0.0, 1.0]).unwrap();
        assert!(orthogonal.abs() < 1e-6);
    }

    #[test]
    fn incomparable_vectors_yield_no_similarity_rather_than_a_misleading_zero() {
        assert_eq!(cosine(&[], &[]), None);
        assert_eq!(cosine(&[1.0, 2.0], &[1.0]), None);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), None);
    }

    #[test]
    fn a_provider_is_only_built_when_one_is_actually_configured() {
        // The shipped default is `local`, which is not a provider this build can call, so nothing is constructed and the matcher stays lexical.
        std::env::set_var("EMBEDDINGS_PROVIDER", "local");
        assert!(EmbeddingProvider::from_env().is_none());
        std::env::set_var("EMBEDDINGS_PROVIDER", "disabled");
        assert!(EmbeddingProvider::from_env().is_none());
        std::env::set_var("EMBEDDINGS_PROVIDER", "ollama");
        let provider =
            EmbeddingProvider::from_env().expect("ollama is a provider this build can call");
        assert_eq!(provider.kind, "ollama");
        std::env::remove_var("EMBEDDINGS_PROVIDER");
    }

    #[test]
    fn status_text_never_claims_ai_when_no_model_ran() {
        let disabled = SemanticStatus::Disabled.label();
        assert!(disabled.contains("lexical ranking only"));
        assert!(!disabled.to_lowercase().contains(" ai "));

        let unavailable = SemanticStatus::Unavailable {
            provider: "ollama".into(),
            reason: "provider responded 501 Not Implemented".into(),
        };
        assert!(unavailable.label().contains("did not answer"));
        assert!(unavailable.label().contains("501"));
        assert!(!unavailable.is_available());

        let available = SemanticStatus::Available {
            provider: "ollama".into(),
            model: "nomic-embed-text".into(),
        };
        assert!(available.is_available());
        assert!(available.label().contains("nomic-embed-text"));
    }

    #[test]
    fn an_embedding_response_is_read_in_either_shape_the_provider_may_use() {
        let plural: EmbedResponse = serde_json::from_str(r#"{"embeddings":[[0.1,0.2]]}"#).unwrap();
        assert_eq!(plural.into_vector(), Some(vec![0.1, 0.2]));
        let singular: EmbedResponse = serde_json::from_str(r#"{"embedding":[0.3]}"#).unwrap();
        assert_eq!(singular.into_vector(), Some(vec![0.3]));
        let neither: EmbedResponse = serde_json::from_str(r#"{"error":"nope"}"#).unwrap();
        assert_eq!(neither.into_vector(), None);
    }
}

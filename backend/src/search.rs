use rust_stemmers::{Algorithm, Stemmer};

#[derive(Clone, Debug, PartialEq)]
pub enum SearchMode {
    /// Unicode-aware prefix matching — best baseline for any language (default).
    Default,
    /// SQLite trigram tokenizer: language-agnostic substring matching.
    Trigram,
    /// Snowball stemmer for the given language algorithm.
    Stemmed(Algorithm),
}

impl SearchMode {
    /// String key used to persist the active mode in `app_config`.
    pub fn as_str(&self) -> String {
        match self {
            Self::Default => "default".to_owned(),
            Self::Trigram => "trigram".to_owned(),
            Self::Stemmed(algo) => serde_json::to_string(algo)
                .unwrap()
                .trim_matches('"')
                .to_owned(),
        }
    }

    pub fn fts_tokenizer(&self) -> &'static str {
        match self {
            Self::Trigram => "trigram",
            _ => "unicode61 remove_diacritics 2",
        }
    }

    /// Normalize text for FTS indexing — applies stemming for Stemmed mode, passthrough otherwise.
    pub fn normalize_text(&self, text: &str) -> String {
        match self {
            Self::Stemmed(algo) => stem_tokens(text, *algo),
            _ => text.to_owned(),
        }
    }

    /// Build an FTS5 MATCH expression from a raw user query string.
    pub fn build_fts_query(&self, q: &str) -> String {
        match self {
            // trigram doesn't support prefix queries; terms must be ≥3 chars
            Self::Trigram => q
                .split_whitespace()
                .filter(|t| t.chars().count() >= 3)
                .map(|t| t.replace('"', "\"\""))
                .collect::<Vec<_>>()
                .join(" "),
            Self::Stemmed(algo) => stem_query(q, *algo),
            Self::Default => {
                let normalized = q.replace(['@', '-', '.'], " ");
                normalized
                    .split_whitespace()
                    .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        }
    }
}

impl std::str::FromStr for SearchMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "" | "default" => Ok(Self::Default),
            "trigram" => Ok(Self::Trigram),
            name => serde_json::from_str::<Algorithm>(&format!("\"{name}\""))
                .map(Self::Stemmed)
                .map_err(|_| {
                    format!(
                        "unknown search mode '{name}' — use 'default', 'trigram', or a Snowball \
                     algorithm name: Arabic, Danish, Dutch, English, Finnish, French, German, \
                     Greek, Hungarian, Italian, Norwegian, Portuguese, Romanian, Russian, \
                     Spanish, Swedish, Tamil, Turkish"
                    )
                }),
        }
    }
}

fn stem_tokens(text: &str, algo: Algorithm) -> String {
    let stemmer = Stemmer::create(algo);
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|word| stemmer.stem(&word.to_lowercase()).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn stem_query(q: &str, algo: Algorithm) -> String {
    let stemmer = Stemmer::create(algo);
    q.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|t| {
            let stemmed = stemmer.stem(&t.to_lowercase()).into_owned();
            format!("\"{}\"*", stemmed.replace('"', "\"\""))
        })
        .collect::<Vec<_>>()
        .join(" ")
}
